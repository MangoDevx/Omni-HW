//! GPU checks: monitor wiring, PCIe link, temperature, VRAM, hardware slowdown and Resizable
//! BAR.

use super::helpers::{finding, guided, plural, summarize};
use super::types::MIN_SAMPLES;
use super::{Category, Context, Level, Outcome, RECENT, Step};
use crate::bios_help::{self, BiosSetting};
use crate::format_bytes;
use crate::gpu::{Display, GpuAdapter, ReBar};

fn display_list(g: &GpuAdapter) -> String {
    g.displays
        .iter()
        .map(Display::summary)
        .collect::<Vec<_>>()
        .join(", ")
}

/// On a desktop with a graphics card, monitors plugged into the motherboard's video outputs
/// run on the CPU's integrated graphics: games render slowly or on the wrong GPU.
pub(super) fn monitor_connection(ctx: &Context) -> Outcome {
    let gpus = &ctx.info.gpus;
    let discrete: Vec<_> = gpus.iter().filter(|g| !g.is_integrated()).collect();
    let integrated: Vec<_> = gpus.iter().filter(|g| g.is_integrated()).collect();
    if discrete.is_empty() {
        return Outcome::Skipped("No separate graphics card".into());
    }
    if ctx.info.battery.is_some() {
        return Outcome::Skipped(
            "Laptops route displays through the integrated GPU by design".into(),
        );
    }
    let on_igpu: usize = integrated.iter().map(|g| g.displays.len()).sum();
    let on_card: usize = discrete.iter().map(|g| g.displays.len()).sum();
    let card = discrete[0];
    if on_igpu == 0 {
        if on_card == 0 {
            return Outcome::Skipped("No connected displays found".into());
        }
        return Outcome::Pass(format!(
            "{} connected to {}",
            plural(on_card, "display", "displays"),
            discrete
                .iter()
                .filter(|g| !g.displays.is_empty())
                .map(|g| g.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let igpu = integrated
        .iter()
        .find(|g| !g.displays.is_empty())
        .copied()
        .unwrap_or(integrated[0]);
    let tips: &[&str] = &[
        "Plug the monitor cable into the graphics card's ports (the horizontal row of ports lower on the back of the case, below the motherboard's I/O panel).",
        "Use the motherboard's HDMI/DisplayPort only if you have no graphics card.",
        "Optionally disable integrated graphics in the BIOS (often \"iGPU Multi-Monitor\" / \"Primary Display\") so the motherboard ports can't be used by mistake.",
    ];
    if on_card == 0 {
        return Outcome::Findings(vec![finding(
            Level::Critical,
            Category::Gpu,
            "Your monitor is plugged into the motherboard, not the graphics card",
            format!(
                "All displays ({}) are connected to {} (integrated graphics). The {} is not driving any display, so games and video run on the much slower integrated GPU or pay a copy penalty.",
                display_list(igpu),
                igpu.name,
                card.name
            ),
            tips,
        )]);
    }
    Outcome::Findings(vec![finding(
        Level::Info,
        Category::Gpu,
        "A display is connected to the motherboard's video output",
        format!(
            "{} is on {} (integrated graphics) while other displays use {}. Fine for a secondary screen, but games shown on it render through the integrated GPU.",
            display_list(igpu),
            igpu.name,
            card.name
        ),
        &["Connect any gaming monitor to the graphics card's ports."],
    )])
}

pub(super) fn gpu_pcie_link(ctx: &Context) -> Outcome {
    let mut out = Vec::new();
    let mut checked = Vec::new();
    for g in &ctx.info.gpus {
        let Some(n) = &g.nvml else { continue };
        let (Some((cur_gen, cur_w)), Some((max_gen, max_w))) = (n.pcie_current, n.pcie_max) else {
            continue;
        };
        checked.push(format!("{}: x{cur_w} {cur_gen}.0", g.name));
        if cur_w < max_w {
            // Exactly half is how boards split a slot's lanes; anything else is usually seating.
            let (detail, steps): (String, &[&str]) = if cur_w * 2 == max_w {
                (
                    format!(
                        "The card supports x{max_w} but the link trained at x{cur_w}, halving its bandwidth. Exactly half usually means the motherboard split the slot's lanes: a card in the second full-length slot, or on some boards an M.2 drive in a slot that borrows the graphics lanes."
                    ),
                    &[
                        "Look up the lane-sharing table in the motherboard manual (often under \"Expansion slots\" or \"M.2\") to see which slot or M.2 socket takes lanes from the graphics slot.",
                        "Move that card or drive to a slot or M.2 socket that doesn't share lanes with the graphics slot.",
                        "If nothing shares the lanes, reseat the card in the top full-length slot and check any riser cable.",
                    ],
                )
            } else {
                (
                    format!(
                        "The card supports x{max_w} but the link trained at x{cur_w}, reducing bandwidth."
                    ),
                    &[
                        "Make sure the card is in the top full-length (CPU) slot and fully seated.",
                        "Check whether an M.2 drive or other card is sharing the slot's lanes (see the motherboard manual).",
                        "Reseat the card and clean the slot contacts; check riser cables if one is used.",
                    ],
                )
            };
            out.push(finding(
                Level::Warning,
                Category::Gpu,
                format!("{} runs at PCIe x{cur_w} instead of x{max_w}", g.name),
                detail,
                steps,
            ));
        } else if cur_gen < max_gen {
            // Lower generation at idle is normal power saving; only flag it while the GPU is busy.
            let busy = ctx
                .current(&format!("{}/nv/load", g.sensor_prefix()))
                .is_some_and(|l| l >= 50.0);
            if busy {
                let tip = format!(
                    "Check the BIOS PCIe slot setting (set it to Auto or Gen {max_gen}) and any riser cable's rating."
                );
                out.push(
                    finding(
                        Level::Info,
                        Category::Gpu,
                        format!("{} is at PCIe {cur_gen}.0 under load", g.name),
                        format!("The card supports PCIe {max_gen}.0 but is running at {cur_gen}.0 while busy."),
                        &[&tip],
                    )
                    .bios(),
                );
            }
        }
    }
    summarize(out, &checked, "Needs PCIe link data (NVIDIA GPUs via NVML)")
}

pub(super) fn gpu_temperature(ctx: &Context) -> Outcome {
    let mut out = Vec::new();
    let mut seen = Vec::new();
    for g in &ctx.info.gpus {
        let Some(t) = ctx.recent_avg(&format!("{}/temp", g.sensor_prefix()), RECENT) else {
            continue;
        };
        seen.push(format!("{}: {t:.0} °C", g.name));
        let level = match t {
            t if t >= 90.0 => Level::Critical,
            t if t >= 83.0 => Level::Warning,
            _ => continue,
        };
        out.push(finding(
            level,
            Category::Gpu,
            format!("{} is running hot", g.name),
            format!("GPU temperature has averaged {t:.0} °C over the last couple of minutes. Most GPUs start reducing clocks in the low-to-mid 80s."),
            &[
                "Clean dust from the card's heatsink and fans and from case filters.",
                "Improve case airflow (intake at the front, exhaust at the back/top).",
                "Set a more aggressive fan curve, or cap the frame rate in games.",
                "On older cards, replacing thermal paste and pads can lower temperatures noticeably.",
            ],
        ));
    }
    summarize(out, &seen, "No GPU temperature sensor")
}

pub(super) fn gpu_vram(ctx: &Context) -> Outcome {
    let mut out = Vec::new();
    let mut seen = Vec::new();
    for g in &ctx.info.gpus {
        let pre = g.sensor_prefix();
        let used = ctx
            .recent_avg(&format!("{pre}/vram/used"), RECENT)
            .or_else(|| ctx.recent_avg(&format!("{pre}/vram/dedicated"), RECENT));
        let (Some(used_mb), true) = (used, g.dedicated_vram > 0) else {
            continue;
        };
        let frac = used_mb * 1_048_576.0 / g.dedicated_vram as f64;
        seen.push(format!("{}: {:.0} %", g.name, frac * 100.0));
        if frac >= 0.95 {
            out.push(finding(
                Level::Warning,
                Category::Gpu,
                format!("{} video memory is full", g.name),
                format!(
                    "{:.0} % of {} VRAM has been in use. When VRAM runs out, games stutter as data spills to system RAM.",
                    frac * 100.0,
                    format_bytes(g.dedicated_vram)
                ),
                &[
                    "Lower texture quality or resolution in games.",
                    "Close other GPU-heavy apps (browsers with video, streaming tools).",
                ],
            ));
        }
    }
    summarize(out, &seen, "Collecting samples")
}

pub(super) fn gpu_hw_slowdown(ctx: &Context) -> Outcome {
    let mut out = Vec::new();
    let mut seen = Vec::new();
    let mut waiting = false;
    for g in ctx.info.gpus.iter().filter(|g| g.nvml.is_some()) {
        let pre = g.sensor_prefix();
        let get = |id: &str| ctx.sensors.get(&format!("{pre}/{id}"));
        let (Some(load), Some(slow), Some(thermal), Some(brake)) = (
            get("nv/load"),
            get("nv/throttle/hw_slowdown"),
            get("nv/throttle/hw_thermal"),
            get("nv/throttle/power_brake"),
        ) else {
            continue;
        };
        // All histories are appended on every sample, so align them from the newest end.
        let busy: Vec<(f32, f32, f32)> = load
            .history
            .iter()
            .rev()
            .zip(slow.history.iter().rev())
            .zip(thermal.history.iter().rev())
            .zip(brake.history.iter().rev())
            .filter(|(((l, _), _), _)| **l >= 50.0)
            .map(|(((_, s), t), b)| (*s, *t, *b))
            .collect();
        if busy.len() < MIN_SAMPLES {
            waiting = true;
            continue;
        }
        let count = |f: fn(&(f32, f32, f32)) -> f32| busy.iter().filter(|x| f(x) >= 0.5).count();
        let (n_slow, n_thermal, n_brake) = (count(|x| x.0), count(|x| x.1), count(|x| x.2));
        seen.push(format!("{}: no hardware slowdown under load", g.name));
        let enough = (busy.len() / 20).max(3);
        let power_steps = || {
            vec![
                Step::new(
                    "Power the card with separate PCIe cables from the power supply, one per 8-pin socket; avoid a single cable with a daisy-chained second plug.",
                ),
                Step::new(
                    "On 12VHPWR / 12V-2x6 cards, push the connector in fully until it clicks, with no sharp bend right at the plug.",
                ),
                Step::new(
                    "Check the power supply meets the card maker's recommended wattage, and that it isn't old or failing.",
                ),
            ]
        };
        if n_brake >= enough || (n_slow >= enough && n_thermal < enough) {
            seen.pop();
            out.push(guided(
                Level::Warning,
                Category::Gpu,
                format!("{} is held back by its power input", g.name),
                format!(
                    "Under load the card signalled a hardware {} in {} of {} samples. The GPU drops its clocks sharply when its power input looks unsafe, usually because of a loose or daisy-chained power cable or a weak power supply.",
                    if n_brake >= enough { "power brake" } else { "slowdown" },
                    n_brake.max(n_slow),
                    busy.len()
                ),
                power_steps(),
            ));
        } else if n_thermal >= enough {
            seen.pop();
            out.push(guided(
                Level::Warning,
                Category::Gpu,
                format!("{} hits its hardware temperature limit", g.name),
                format!(
                    "Under load the card triggered its hardware thermal slowdown in {n_thermal} of {} samples, cutting clocks hard to protect itself.",
                    busy.len()
                ),
                vec![
                    Step::new("Clean dust from the card's heatsink and fans and the case filters."),
                    Step::new("Improve case airflow and set a more aggressive fan curve."),
                    Step::new(
                        "On older cards, new thermal pads and paste often fix hot memory or hotspot temperatures.",
                    ),
                ],
            ));
        }
    }
    summarize(
        out,
        &seen,
        if waiting {
            "Needs some time with the GPU under load (e.g. a game) to judge"
        } else {
            "Needs an NVIDIA GPU (throttle reasons via NVML)"
        },
    )
}

pub(super) fn resizable_bar(ctx: &Context) -> Outcome {
    let mut out = Vec::new();
    let mut seen = Vec::new();
    for g in ctx.info.gpus.iter().filter(|g| !g.is_integrated()) {
        match g.rebar() {
            ReBar::Enabled { bar_bytes } => {
                seen.push(format!("{}: on ({} BAR)", g.name, format_bytes(bar_bytes)))
            }
            ReBar::Disabled { bar_bytes } => {
                if g.supports_rebar() == Some(false) {
                    seen.push(format!("{}: not supported by this GPU", g.name));
                    continue;
                }
                seen.push(format!("{}: off", g.name));
                let mut steps = bios_help::bios_steps(
                    ctx.info,
                    BiosSetting::ResizableBar,
                    "Enable \"Above 4G Decoding\" and \"Re-Size BAR Support\".",
                    "",
                );
                steps.push(Step::new(
                    "If the option is missing or greyed out, disable CSM / Legacy boot so the PC boots in UEFI mode (the system must be installed in UEFI mode on a GPT disk).",
                ));
                steps.push(Step::new(
                    "Update the motherboard BIOS and GPU driver; RTX 30 cards also need a ReBAR-capable VBIOS from the card maker.",
                ));
                out.push(
                    guided(
                        Level::Warning,
                        Category::Gpu,
                        format!("Resizable BAR is off for {}", g.name),
                        format!(
                            "The CPU can only see a {} window of the GPU's {} memory. With Resizable BAR on, many games gain a few percent (some 10 % or more).",
                            format_bytes(bar_bytes),
                            format_bytes(g.dedicated_vram)
                        ),
                        steps,
                    )
                    .bios(),
                );
            }
            ReBar::Unknown => {}
        }
    }
    summarize(
        out,
        &seen,
        "ReBAR status is only available for NVIDIA GPUs so far",
    )
}

#[cfg(test)]
mod tests {
    use super::super::testutil::*;
    use super::*;
    use crate::SystemInfo;
    use crate::gpu::VENDOR_NVIDIA;
    use crate::sensors::Reading;
    use crate::sensors::{SensorTable, Unit};

    #[test]
    fn monitor_on_motherboard() {
        use crate::gpu::{Display, GpuAdapter};
        let s = SensorTable::default();
        let disp = |n: &str| Display {
            name: format!(r"\\.\{n}"),
            width: 2560,
            height: 1440,
        };
        let card = |displays| GpuAdapter {
            name: "RTX".into(),
            vendor_id: 0x10DE,
            device_id: 0x2204,
            dedicated_vram: 24 << 30,
            displays,
            ..Default::default()
        };
        let igpu = |displays| GpuAdapter {
            name: "UHD 770".into(),
            vendor_id: 0x8086,
            device_id: 0xA780,
            dedicated_vram: 128 << 20,
            displays,
            ..Default::default()
        };

        let mut info = SystemInfo {
            gpus: vec![card(vec![]), igpu(vec![disp("DISPLAY1")])],
            ..Default::default()
        };
        let o = monitor_connection(&ctx(&info, &s));
        assert_eq!(levels(&o), vec![Level::Critical]);
        let Outcome::Findings(f) = o else {
            unreachable!()
        };
        assert!(f[0].detail.contains("DISPLAY1 (2560x1440)"));

        info.gpus = vec![card(vec![disp("DISPLAY1")]), igpu(vec![disp("DISPLAY2")])];
        assert_eq!(
            levels(&monitor_connection(&ctx(&info, &s))),
            vec![Level::Info]
        );

        info.gpus = vec![card(vec![disp("DISPLAY1")]), igpu(vec![])];
        assert!(matches!(
            monitor_connection(&ctx(&info, &s)),
            Outcome::Pass(_)
        ));

        info.gpus = vec![igpu(vec![disp("DISPLAY1")])];
        assert!(matches!(
            monitor_connection(&ctx(&info, &s)),
            Outcome::Skipped(_)
        ));

        info.gpus = vec![card(vec![]), igpu(vec![disp("DISPLAY1")])];
        info.battery = Some(Default::default());
        assert!(matches!(
            monitor_connection(&ctx(&info, &s)),
            Outcome::Skipped(_)
        ));
    }

    #[test]
    fn rebar_check() {
        use crate::gpu::{GpuAdapter, NvmlDetails};
        let s = SensorTable::default();
        let card = |bar| GpuAdapter {
            name: "RTX 3090".into(),
            vendor_id: 0x10DE,
            device_id: 0x2204,
            dedicated_vram: 24 << 30,
            nvml: Some(NvmlDetails {
                bar1_total: Some(bar),
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut info = SystemInfo {
            gpus: vec![card(256 << 20)],
            ..Default::default()
        };
        assert_eq!(
            levels(&resizable_bar(&ctx(&info, &s))),
            vec![Level::Warning]
        );
        info.gpus = vec![card(32 << 30)];
        assert!(matches!(resizable_bar(&ctx(&info, &s)), Outcome::Pass(_)));
        // Turing doesn't support ReBAR, so a legacy-size BAR is expected.
        info.gpus[0].device_id = 0x1E04;
        info.gpus[0].nvml.as_mut().unwrap().bar1_total = Some(256 << 20);
        assert!(matches!(resizable_bar(&ctx(&info, &s)), Outcome::Pass(_)));
        info.gpus[0].nvml = None;
        assert!(matches!(
            resizable_bar(&ctx(&info, &s)),
            Outcome::Skipped(_)
        ));
    }

    #[test]
    fn gpu_slowdown() {
        use crate::gpu::{GpuAdapter, NvmlDetails};
        let info = SystemInfo {
            gpus: vec![GpuAdapter {
                name: "RTX".into(),
                vendor_id: VENDOR_NVIDIA,
                luid: 1,
                nvml: Some(NvmlDetails::default()),
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut s = SensorTable::default();
        let feed = |s: &mut SensorTable, load: f64, slow: f64, brake: f64| {
            let r = |id: &str, v: f64| Reading {
                id: format!("gpu/1/{id}"),
                label: id.into(),
                group: "g".into(),
                unit: Unit::None,
                value: v,
            };
            s.update([
                r("nv/load", load),
                r("nv/throttle/hw_slowdown", slow),
                r("nv/throttle/hw_thermal", 0.0),
                r("nv/throttle/power_brake", brake),
            ]);
        };
        assert!(matches!(
            gpu_hw_slowdown(&ctx(&info, &s)),
            Outcome::Skipped(_)
        ));
        for _ in 0..20 {
            feed(&mut s, 99.0, 0.0, 0.0);
        }
        assert!(matches!(gpu_hw_slowdown(&ctx(&info, &s)), Outcome::Pass(_)));
        for _ in 0..10 {
            feed(&mut s, 99.0, 1.0, 1.0);
        }
        assert_eq!(
            levels(&gpu_hw_slowdown(&ctx(&info, &s))),
            vec![Level::Warning]
        );
    }
}
