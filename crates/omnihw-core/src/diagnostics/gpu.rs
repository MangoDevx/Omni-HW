//! GPU checks: monitor wiring and refresh rate, PCIe link, temperature, VRAM, drivers,
//! hardware slowdown and Resizable BAR.

use super::helpers::{
    DDU_URL, driver_step, finding, format_date, guided, months_since, plural, summarize,
};
use super::types::MIN_SAMPLES;
use super::{Category, Context, Level, Outcome, RECENT, Step};
use crate::bios_help::{self, BiosSetting};
use crate::checkup::{Connection, Device, gpu_package_vendor, problem_is_missing_driver};
use crate::cpuid::Vendor;
use crate::format_bytes;
use crate::gpu::{Display, GpuAdapter, ReBar, VENDOR_INTEL, pci_vendor_name};

/// PCI vendor id of Microsoft's virtual display adapters (Hyper-V and the like).
const VENDOR_MICROSOFT: u32 = 0x1414;

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
            out.push(finding(
                Level::Warning,
                Category::Gpu,
                format!("{} runs at PCIe x{cur_w} instead of x{max_w}", g.name),
                format!("The card supports x{max_w} but the link trained at x{cur_w}, reducing bandwidth."),
                &[
                    "Make sure the card is in the top full-length (CPU) slot and fully seated.",
                    "Check whether an M.2 drive or other card is sharing the slot's lanes (see the motherboard manual).",
                    "Reseat the card and clean the slot contacts; check riser cables if one is used.",
                ],
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

pub(super) fn monitor_refresh(ctx: &Context) -> Outcome {
    let mons: Vec<_> = ctx
        .info
        .checkup
        .monitors
        .iter()
        .filter(|m| m.connection != Connection::Virtual)
        .collect();
    if mons.is_empty() {
        return Outcome::Skipped("No monitor information".into());
    }
    let mut out = Vec::new();
    for m in &mons {
        if !m.below_max() {
            continue;
        }
        let internal = m.connection == Connection::Internal;
        let mut steps = vec![Step::settings(
            format!(
                "Open Settings > System > Display > Advanced display, select {} and choose {} Hz under \"Choose a refresh rate\".",
                m.label(),
                m.max_hz
            ),
            "ms-settings:display-advanced",
        )];
        if !internal {
            steps.push(Step::new(
                "If the higher rate keeps resetting or flickers, use a DisplayPort cable (or an HDMI 2.0/2.1 cable rated for the resolution) plugged into the graphics card.",
            ));
        }
        steps.push(Step::new(
            "In games, also set the refresh rate (and any frame cap) to match.",
        ));
        out.push(guided(
            if internal { Level::Info } else { Level::Warning },
            Category::Gpu,
            format!(
                "{} runs at {:.0} Hz but supports {} Hz",
                m.label(),
                m.current_hz,
                m.max_hz
            ),
            format!(
                "Windows offers {} Hz for this monitor at {}x{}, but it's set to {:.0} Hz. A high-refresh monitor left at 60 Hz feels no smoother than a regular one.",
                m.max_hz, m.width, m.height, m.current_hz
            ),
            steps,
        ));
    }
    if out.is_empty() {
        return Outcome::Pass(
            mons.iter()
                .map(|m| format!("{}: {:.0} Hz", m.label(), m.current_hz))
                .collect::<Vec<_>>()
                .join("; "),
        );
    }
    Outcome::Findings(out)
}

/// Display adapters running without the vendor's driver.
pub(super) fn gpu_driver_missing(ctx: &Context) -> Outcome {
    let displays: Vec<_> = ctx
        .info
        .checkup
        .devices
        .iter()
        .filter(|d| d.is_display())
        .collect();
    if displays.is_empty() {
        return Outcome::Skipped("Display adapter list not available".into());
    }
    let mut out = Vec::new();
    for d in &displays {
        let no_driver = d.is_basic_display() || d.problem.is_some_and(problem_is_missing_driver);
        // Hyper-V and other virtual adapters use Microsoft drivers on purpose.
        let Some(v) = d.pci_vendor().filter(|&v| v != VENDOR_MICROSOFT) else {
            continue;
        };
        if !no_driver {
            continue;
        }
        let brand = pci_vendor_name(v).unwrap_or("graphics");
        out.push(guided(
            Level::Critical,
            Category::Gpu,
            format!("No {brand} graphics driver installed"),
            format!(
                "A {brand} graphics adapter is running on \"{}\" (Windows' fallback driver). Without the real driver, games run poorly or not at all, high refresh rates and resolutions are missing and the GPU can't be monitored.",
                d.name
            ),
            vec![
                driver_step(v, "the latest driver for your graphics card"),
                Step::new("Run the installer and restart when it asks."),
                Step::settings(
                    "If you can't download it, Windows Update often offers the driver under Advanced options > Optional updates.",
                    "ms-settings:windowsupdate-optionalupdates",
                ),
            ],
        ));
    }
    if out.is_empty() {
        return Outcome::Pass(
            displays
                .iter()
                .map(|d| d.name.as_str())
                .collect::<Vec<_>>()
                .join("; "),
        );
    }
    Outcome::Findings(out)
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

pub(super) fn leftover_gpu_drivers(ctx: &Context) -> Outcome {
    let packages = &ctx.info.checkup.gpu_driver_packages;
    let mut present: Vec<u32> = ctx.info.gpus.iter().map(|g| g.vendor_id).collect();
    present.extend(
        ctx.info
            .checkup
            .devices
            .iter()
            .filter(|d| d.is_display())
            .filter_map(Device::pci_vendor),
    );
    let intel_cpu = ctx.info.cpu.vendor == Vendor::Intel;
    let leftovers: Vec<(&String, u32)> = packages
        .iter()
        .filter_map(|p| Some((p, gpu_package_vendor(p)?)))
        // An Intel CPU's graphics may just be switched off in the BIOS.
        .filter(|&(_, v)| !present.contains(&v) && !(v == VENDOR_INTEL && intel_cpu))
        .collect();
    if leftovers.is_empty() {
        return Outcome::Pass(if packages.is_empty() {
            "No graphics driver packages listed".into()
        } else {
            "Installed graphics drivers match the GPUs".into()
        });
    }
    let names: Vec<&str> = leftovers.iter().map(|(p, _)| p.as_str()).collect();
    let mut vendors: Vec<&str> = leftovers
        .iter()
        .filter_map(|&(_, v)| pci_vendor_name(v))
        .collect();
    vendors.dedup();
    Outcome::Findings(vec![guided(
        Level::Info,
        Category::Gpu,
        format!(
            "Old {} graphics driver still installed",
            vendors.join(" and ")
        ),
        format!(
            "{} is installed, but no such graphics card is present. Leftovers from a previous card can cause crashes, black screens or the wrong control panel opening.",
            names.join(", ")
        ),
        vec![
            Step::settings(
                format!(
                    "Uninstall {} in Settings > Apps > Installed apps.",
                    names.join(" and ")
                ),
                "ms-settings:appsfeatures",
            ),
            Step::url(
                "For a thorough clean-up (recommended after switching between NVIDIA and AMD), use Display Driver Uninstaller (DDU) in Safe Mode, then reinstall the current card's driver.",
                DDU_URL,
            ),
        ],
    )])
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
                    "If the option is missing or greyed out, disable CSM / Legacy boot so the PC boots in UEFI mode (Windows must be on a GPT disk).",
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

pub(super) fn gpu_driver_age(ctx: &Context) -> Outcome {
    let mut out = Vec::new();
    let mut seen = Vec::new();
    for g in ctx.info.gpus.iter().filter(|g| !g.is_integrated()) {
        let Some(date) = g.driver_date else {
            continue;
        };
        let date_text = format_date(date);
        seen.push(format!("{}: {date_text}", g.name));
        if months_since(ctx, date.0, date.1) >= 12 {
            out.push(guided(
                Level::Info,
                Category::Gpu,
                format!("{} driver is over a year old", g.name),
                format!("The installed driver is dated {date_text}. Newer drivers often bring game fixes and performance improvements."),
                vec![driver_step(g.vendor_id, "the latest driver")],
            ));
        }
    }
    summarize(out, &seen, "Driver date not available")
}

#[cfg(test)]
mod tests {
    use super::super::testutil::*;
    use super::*;
    use crate::SystemInfo;
    use crate::checkup::Monitor;
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
    fn driver_age_check() {
        use crate::gpu::GpuAdapter;
        let s = SensorTable::default();
        let g = GpuAdapter {
            name: "RTX".into(),
            vendor_id: 0x10DE,
            device_id: 0x2204,
            dedicated_vram: 24 << 30,
            driver_date: Some((2026, 9, 30)),
            ..Default::default()
        };
        let mut info = SystemInfo {
            gpus: vec![g],
            ..Default::default()
        };
        assert!(matches!(gpu_driver_age(&ctx(&info, &s)), Outcome::Pass(_)));
        info.gpus[0].driver_date = Some((2025, 6, 1));
        assert_eq!(levels(&gpu_driver_age(&ctx(&info, &s))), vec![Level::Info]);
    }

    #[test]
    fn refresh_rate() {
        let s = SensorTable::default();
        let mut info = SystemInfo::default();
        assert!(matches!(
            monitor_refresh(&ctx(&info, &s)),
            Outcome::Skipped(_)
        ));
        info.checkup.monitors = vec![Monitor {
            name: "Gaming".into(),
            connection: Connection::Hdmi,
            width: 2560,
            height: 1440,
            current_hz: 60.0,
            max_hz: 144,
            ..Default::default()
        }];
        let o = monitor_refresh(&ctx(&info, &s));
        assert_eq!(levels(&o), vec![Level::Warning]);
        let Outcome::Findings(f) = o else {
            unreachable!()
        };
        assert_eq!(
            f[0].steps[0].link,
            Some(super::super::Link::Settings(
                "ms-settings:display-advanced".into()
            ))
        );
        info.checkup.monitors[0].current_hz = 143.97;
        assert!(matches!(monitor_refresh(&ctx(&info, &s)), Outcome::Pass(_)));
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
