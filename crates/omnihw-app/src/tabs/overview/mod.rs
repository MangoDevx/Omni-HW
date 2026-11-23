//! Dashboard: the CPU-Z identity, GPU-Z card details and HWiNFO live sensors on one page.

mod widgets;

use crate::app::{OmniApp, Tab};
use crate::heat::{self, GREEN, ORANGE, RED, Severity, lerp, rgb8};
use crate::sampler::Snapshot;
use crate::ui;
use gpui_kit::component::separator::Separator;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::{
    AnyElement, Context, Div, FontWeight, InteractiveElement, IntoElement, ParentElement, Rgba,
    StatefulInteractiveElement, Styled, Window, div, px, rems,
};
use omnihw_core::gpu::{GpuAdapter, ReBar};
use omnihw_core::sensors::{CPU_PACKAGE_POWER, CPU_TEMP};
use omnihw_core::{SystemInfo, format_bytes, format_uptime};
use std::collections::VecDeque;
use std::rc::Rc;
use widgets::{Metric, bar, card, m, metrics, severity_icon, sparkline, sparkline_colored};

const CPU: Rgba = heat::BLUE;
const GPU: Rgba = rgb8(38, 166, 154);
const MEM: Rgba = rgb8(171, 71, 188);
const SYS: Rgba = rgb8(245, 156, 26);
const DISK: Rgba = rgb8(92, 107, 192);

/// Two columns from this page width up (in rems, so it follows the UI scale).
const TWO_COLUMN_WIDTH: f32 = 760.0 / 16.0;

/// Clicking the Health Check summary opens the Health tab.
pub fn show(app: &mut OmniApp, window: &mut Window, cx: &mut Context<OmniApp>) -> AnyElement {
    let two_columns =
        crate::app::page_width_rems(app.settings.nav_style, window) >= TWO_COLUMN_WIDTH;
    let Some(info) = app.sampler.info() else {
        return ui::column().into_any_element();
    };
    let snap = app.sampler.snapshot();
    // Copy the banner's lines out of the cached report (it borrows `app.health`).
    let (summary, gaming_summary) = {
        let report = app.health.get(info, &snap, app.settings.show_gaming);
        (report.summary.clone(), report.gaming_summary.clone())
    };
    let app: &OmniApp = app;
    let is_open = |key: &str| ui::is_open(app, key, true);

    let mut page = ui::column()
        .child(header(info, cx))
        .child(health_banner(summary, gaming_summary, cx))
        .child(ui::space(8.0));
    if two_columns {
        let left = v_flex()
            .id("overview-left")
            .flex_1()
            .min_w_0()
            .child(cpu_card(info, &snap, &is_open, cx))
            .child(memory_card(info, &snap, &is_open, cx))
            .child(system_card(info, &snap, &is_open, cx));
        let right = v_flex()
            .id("overview-right")
            .flex_1()
            .min_w_0()
            .children(gpu_cards(info, &snap, &is_open, cx))
            .children(storage_card(info, &snap, &is_open, cx));
        page = page.child(
            h_flex()
                .w_full()
                .items_start()
                .gap_2()
                .child(left)
                .child(right),
        );
    } else {
        page = page
            .child(cpu_card(info, &snap, &is_open, cx))
            .children(gpu_cards(info, &snap, &is_open, cx))
            .child(memory_card(info, &snap, &is_open, cx))
            .children(storage_card(info, &snap, &is_open, cx))
            .child(system_card(info, &snap, &is_open, cx));
    }
    page.into_any_element()
}

fn val(snap: &Snapshot, id: &str) -> Option<f64> {
    snap.table.get(id).map(|t| t.reading.value)
}

fn hist<'a>(snap: &'a Snapshot, id: &str) -> Option<&'a VecDeque<f32>> {
    snap.table.get(id).map(|t| &t.history)
}

fn or_wait(v: Option<String>) -> String {
    v.unwrap_or_else(|| "…".into())
}

fn header(info: &SystemInfo, cx: &mut Context<OmniApp>) -> Div {
    let mut parts: Vec<String> = Vec::new();
    let board = info.smbios.as_ref().and_then(|s| s.board.as_ref());
    if let Some(b) = board {
        parts.push(format!("{} {}", b.manufacturer, b.product));
    }
    // Prebuilt systems and laptops name the product in the system record; DIY boards repeat the board id there.
    if let Some(sys) = info.smbios.as_ref().and_then(|s| s.system.as_ref()) {
        let generic = sys.product.is_empty()
            || sys.product.contains("To be filled")
            || sys.product.contains("System Product Name")
            || board.is_some_and(|b| b.product.contains(&sys.product));
        if !generic {
            parts.insert(0, format!("{} {}", sys.manufacturer, sys.product));
        }
    }
    let names = ui::wrap_row()
        .text_lg()
        .child(ui::strong(info.cpu.brand.clone()))
        .children(info.gpus.iter().enumerate().map(|(i, g)| {
            h_flex()
                .id(("header-gpu", i))
                .gap_2()
                .child(ui::weak("·", cx))
                .child(ui::strong(g.name.clone()))
        }));
    let mut header = ui::column().child(names);
    if !parts.is_empty() {
        header = header.child(ui::weak(parts.join("  ·  "), cx));
    }
    header
}

fn ghz(mhz: f64) -> String {
    format!("{:.2} GHz", mhz / 1000.0)
}

fn cpu_card(
    info: &SystemInfo,
    snap: &Snapshot,
    is_open: &dyn Fn(&str) -> bool,
    cx: &mut Context<OmniApp>,
) -> AnyElement {
    let mut sub: Vec<String> = Vec::new();
    if let Some(n) = info.codename {
        sub.push(format!("{} · {} · {}", n.codename, n.microarch, n.process));
    }
    if let Some(t) = &info.topology {
        let (p, e) = t.p_e_counts();
        let cores = if e > 0 {
            format!("{p}P + {e}E cores")
        } else {
            format!("{} cores", t.cores.len())
        };
        sub.push(format!("{cores} / {} threads", t.logical_count()));
    }
    if let Some(b) = info.base_mhz {
        sub.push(format!("base {}", ghz(b as f64)));
    }
    card(
        "cpu-card",
        CPU,
        "Processor",
        &sub.join("  ·  "),
        is_open,
        cx,
        |cx| {
            let power = snap
                .table
                .first_of(&CPU_PACKAGE_POWER)
                .map(|t| format!("{:.0} W", t.reading.value));
            let temp = val(snap, CPU_TEMP).map(|t| format!("{t:.0} °C"));
            let mut row = vec![
                m("Clock (avg)", or_wait(val(snap, "cpu/clock/avg").map(ghz))),
                m(
                    "Usage",
                    or_wait(val(snap, "cpu/load/total").map(|v| format!("{v:.0} %"))),
                ),
            ];
            if let Some(p) = power {
                row.push(m("Package Power", p));
            }
            if let Some(t) = temp {
                row.push(m("Temperature", t));
            }
            let top = info.topology.as_ref().and_then(|t| {
                (0..t.cores.len())
                    .filter_map(|i| val(snap, &format!("cpu/clock/core{i}")))
                    .reduce(f64::max)
            });
            row.push(m("Fastest Core", or_wait(top.map(ghz))));
            v_flex()
                .gap_1()
                .child(metrics(&row, cx))
                .child(sparkline(
                    "Usage",
                    hist(snap, "cpu/load/total"),
                    Some(100.0),
                    CPU,
                    cx,
                ))
                .child(ui::space(2.0))
                .children(core_tiles(info, snap, cx))
        },
    )
    .into_any_element()
}

/// What a core tile's hover text shows.
struct CoreTip {
    label: String,
    sev: Severity,
    load: f64,
    temp: Option<f64>,
    clock: Option<f64>,
    /// "Thread N: L %" per logical processor.
    threads: Vec<String>,
}

/// One tile per physical core showing its clock. Blue shade follows load; orange with a
/// warning icon / red with an exclamation icon when the busiest thread crosses `heat::CORE_LOAD`.
fn core_tiles(info: &SystemInfo, snap: &Snapshot, cx: &mut Context<OmniApp>) -> Option<Div> {
    let t = info.topology.as_ref()?;
    let base = widgets::track(cx);
    let mut tiles = h_flex().flex_wrap().gap_1();
    for (i, core) in t.cores.iter().enumerate() {
        let label = t.core_label(i);
        let short = label.replace("-core ", "").replace("Core ", "C");
        let loads: Vec<f64> = core
            .logical
            .iter()
            .filter_map(|l| val(snap, &format!("cpu/load/{},{}", l.group, l.number)))
            .collect();
        let load = loads.iter().copied().fold(0.0, f64::max);
        let clock = val(snap, &format!("cpu/clock/core{i}"));
        let temp = val(snap, &format!("cpu/temp/core{i}"));
        let sev = heat::CORE_LOAD.severity(load);
        let fill = match sev {
            Severity::Normal => lerp(base, CPU, 0.15 + 0.85 * (load / 100.0) as f32),
            Severity::Warning => lerp(base, ORANGE, 0.75),
            Severity::Critical => lerp(base, RED, 0.85),
        };
        // Black or white by tile brightness, so tiles read in both themes.
        let text = heat::text_on(fill);
        let tip = Rc::new(CoreTip {
            threads: core
                .logical
                .iter()
                .zip(&loads)
                .map(|(l, v)| format!("Thread {}: {v:.0} %", l.number))
                .collect(),
            label,
            sev,
            load,
            temp,
            clock,
        });
        let tile = ui::clickable(("core", i))
            .relative()
            .flex_none()
            .w(rems(58.0 / 16.0))
            .h(rems(40.0 / 16.0))
            .rounded(px(4.0))
            .bg(fill)
            .child(
                h_flex()
                    .absolute()
                    .top(px(3.0))
                    .left(px(4.0))
                    .gap(px(3.0))
                    .text_size(rems(10.0 / 16.0))
                    .children(severity_icon(sev))
                    .child(ui::label(short).text_color(text)),
            )
            .children(clock.map(|c| {
                div()
                    .absolute()
                    .bottom(px(4.0))
                    .left_0()
                    .w_full()
                    .flex()
                    .justify_center()
                    .text_size(rems(13.0 / 16.0))
                    .child(ui::mono(format!("{:.2}", c / 1000.0), cx).text_color(text))
            }))
            .tooltip(move |window, cx| {
                let tip = tip.clone();
                Tooltip::element(move |_, cx| core_tip(&tip, cx)).build(window, cx)
            });
        tiles = tiles.child(tile);
    }
    Some(tiles)
}

/// The hover text of a core tile: its name, a load warning, temperature, clock and the load
/// of each of its threads.
fn core_tip(tip: &CoreTip, cx: &gpui_kit::App) -> Div {
    let load = tip.load;
    let warning = match tip.sev {
        Severity::Warning => Some(ui::colored(format!("Heavy load ({load:.0} %)"), ORANGE, cx)),
        Severity::Critical => Some(ui::colored(format!("Saturated ({load:.0} %)"), RED, cx)),
        Severity::Normal => None,
    };
    v_flex()
        .child(ui::strong(tip.label.clone()))
        .children(warning)
        .children(
            tip.temp
                .map(|t| ui::label(format!("Temperature: {t:.0} °C"))),
        )
        .children(tip.clock.map(|c| ui::label(format!("Clock: {c:.0} MHz"))))
        .children(tip.threads.iter().map(|t| ui::label(t.clone())))
}

fn gpu_cards(
    info: &SystemInfo,
    snap: &Snapshot,
    is_open: &dyn Fn(&str) -> bool,
    cx: &mut Context<OmniApp>,
) -> Vec<AnyElement> {
    if info.gpus.is_empty() {
        return vec![
            card(
                "gpu-none",
                GPU,
                "Graphics",
                "No hardware adapter found",
                is_open,
                cx,
                |_| div(),
            )
            .into_any_element(),
        ];
    }
    info.gpus
        .iter()
        .enumerate()
        .map(|(i, g)| gpu_card(i, g, snap, is_open, cx))
        .collect()
}

fn gpu_card(
    index: usize,
    g: &GpuAdapter,
    snap: &Snapshot,
    is_open: &dyn Fn(&str) -> bool,
    cx: &mut Context<OmniApp>,
) -> AnyElement {
    let pre = g.sensor_prefix();
    let id = |s: &str| format!("{pre}/{s}");
    let mut sub: Vec<String> = Vec::new();
    if let Some(c) = g.chip() {
        sub.push(format!("{} · {} · {}", c.chip, c.architecture, c.process));
    }
    sub.push(format!("{} VRAM", format_bytes(g.dedicated_vram)));
    if let Some(b) = g.board_vendor().filter(|b| *b != g.vendor_name()) {
        sub.push(b.to_string());
    }
    let driver = g
        .nvml
        .as_ref()
        .map(|n| n.driver_version.clone())
        .filter(|d| !d.is_empty());
    sub.push(format!(
        "driver {}",
        driver.unwrap_or_else(|| g.driver_version.clone())
    ));

    card(
        ("gpu-card", index),
        GPU,
        &g.name,
        &sub.join("  ·  "),
        is_open,
        cx,
        |cx| {
            // Prefer the vendor's load figure (what GPU-Z shows); fall back to the busiest engine.
            let load_id = if snap.table.get(&id("nv/load")).is_some() {
                id("nv/load")
            } else {
                id("load")
            };
            let mut row = vec![m(
                "Load",
                or_wait(val(snap, &load_id).map(|v| format!("{v:.0} %"))),
            )];
            let temp = val(snap, &id("temp"));
            if let Some(t) = temp {
                row.push(Metric {
                    caption: "Temperature",
                    value: format!("{t:.0} °C"),
                    color: heat::GPU_TEMP.severity(t).color(),
                });
            }
            if let Some(p) = val(snap, &id("power")) {
                row.push(m("Power", format!("{p:.0} W")));
            }
            if let Some(c) = val(snap, &id("clock/core")) {
                row.push(m("Core Clock", format!("{c:.0} MHz")));
            }
            let mut body = v_flex().gap_1().child(metrics(&row, cx)).child(sparkline(
                "Load",
                hist(snap, &load_id),
                Some(100.0),
                GPU,
                cx,
            ));
            if let Some(h) = hist(snap, &id("temp")) {
                body = body.child(sparkline_colored(
                    "Temperature",
                    Some(h),
                    Some(100.0),
                    |v| heat::GPU_TEMP.color(GREEN, v as f64),
                    cx,
                ));
            }
            body = body.child(ui::space(2.0));
            let used = val(snap, &id("vram/used")).or_else(|| val(snap, &id("vram/dedicated")));
            let total_mb = g.dedicated_vram as f64 / 1048576.0;
            if let Some(u) = used.filter(|_| total_mb > 0.0) {
                let frac = u / total_mb;
                body = body.child(bar(
                    frac as f32,
                    format!("VRAM {:.1} / {:.1} GB", u / 1024.0, total_mb / 1024.0),
                    heat::VRAM_USED.color(GPU, frac),
                    cx,
                ));
            }
            body = body.child(rebar_line(g, cx));
            let mut extras: Vec<String> = Vec::new();
            if let Some(f) = val(snap, &id("fan")) {
                extras.push(format!("Fan {f:.0} %"));
            }
            if let Some(c) = val(snap, &id("clock/mem")) {
                extras.push(format!("Memory {c:.0} MHz"));
            }
            if let Some(p) = val(snap, &id("power_pct")) {
                extras.push(format!("{p:.0} % of power limit"));
            }
            if let Some(s) = val(snap, &id("pstate")) {
                extras.push(format!("P{s:.0}"));
            }
            if !extras.is_empty() {
                body = body.child(ui::weak(extras.join("  ·  "), cx));
            }
            body
        },
    )
    .into_any_element()
}

fn memory_card(
    info: &SystemInfo,
    snap: &Snapshot,
    is_open: &dyn Fn(&str) -> bool,
    cx: &mut Context<OmniApp>,
) -> AnyElement {
    let dimms: Vec<_> = info
        .smbios
        .iter()
        .flat_map(|s| s.populated_memory())
        .collect();
    let installed: u64 = dimms.iter().map(|d| d.size_mb).sum();
    let total_mb = info.memory.map(|m| m.total_bytes as f64 / 1048576.0);
    let title = match dimms.first() {
        Some(d) => format!(
            "Memory  {} GB {}-{}",
            installed / 1024,
            d.memory_type,
            d.configured_speed_mts
        ),
        None => "Memory".into(),
    };
    let mut sub: Vec<String> = Vec::new();
    if let Some(d) = dimms.first() {
        let same = dimms.iter().all(|x| x.size_mb == d.size_mb);
        let layout = if same {
            format!("{} x {} GB", dimms.len(), d.size_mb / 1024)
        } else {
            format!("{} modules", dimms.len())
        };
        sub.push(layout);
        if let Some(s) = info.smbios.as_ref().filter(|s| s.memory_slots > 0) {
            sub.push(format!("{} of {} slots", dimms.len(), s.memory_slots));
        }
        if !d.manufacturer.is_empty() {
            sub.push(d.manufacturer.clone());
        }
        if !d.part_number.is_empty() {
            sub.push(d.part_number.clone());
        }
    }
    card(
        "memory-card",
        MEM,
        &title,
        &sub.join("  ·  "),
        is_open,
        cx,
        |cx| {
            let used = if let (Some(used), Some(total)) = (val(snap, "mem/used"), total_mb) {
                let frac = used / total;
                bar(
                    frac as f32,
                    format!("{:.1} / {:.1} GB used", used / 1024.0, total / 1024.0),
                    heat::MEMORY_USED.color(MEM, frac),
                    cx,
                )
            } else {
                div().child(ui::weak("Waiting for first sample…", cx))
            };
            v_flex().gap_1().child(used).child(sparkline(
                "Load",
                hist(snap, "mem/load"),
                Some(100.0),
                MEM,
                cx,
            ))
        },
    )
    .into_any_element()
}

fn system_card(
    info: &SystemInfo,
    snap: &Snapshot,
    is_open: &dyn Fn(&str) -> bool,
    cx: &mut Context<OmniApp>,
) -> AnyElement {
    card("system-card", SYS, "System", "", is_open, cx, |cx| {
        let mut rows: Vec<(&str, String)> = Vec::new();
        if let Some(o) = &info.os {
            rows.push(("Windows", o.summary()));
            rows.push(("Computer", o.computer_name.clone()));
        }
        if let Some(u) = snap.uptime_secs {
            rows.push(("Uptime", format_uptime(u)));
        }
        if let Some(c) = val(snap, "battery/charge") {
            let ac = val(snap, "battery/ac").is_some_and(|v| v > 0.5);
            rows.push((
                "Battery",
                format!("{c:.0} %{}", if ac { " (plugged in)" } else { "" }),
            ));
        }
        if let Some(b) = &info.battery_health {
            rows.push(("Battery Health", b.summary()));
        }
        if let Some(s) = &info.smbios {
            if let Some(b) = &s.board {
                rows.push(("Motherboard", format!("{} {}", b.manufacturer, b.product)));
            }
            if let Some(b) = &s.bios {
                rows.push(("BIOS", format!("{} ({})", b.version, b.release_date)));
            }
            if let Some(p) = s.processors.first() {
                rows.push(("CPU Socket", p.socket.clone()));
            }
        }
        let cpu_w = snap
            .table
            .first_of(&CPU_PACKAGE_POWER)
            .map(|t| t.reading.value);
        let gpu_w: Option<f64> = info
            .gpus
            .iter()
            .filter_map(|g| val(snap, &format!("{}/power", g.sensor_prefix())))
            .reduce(|a, b| a + b);
        if let (Some(c), Some(g)) = (cpu_w, gpu_w) {
            rows.push(("CPU + GPU Power", format!("{:.0} W", c + g)));
        }
        rows.push(("Sensors", format!("{} live", snap.table.entries().len())));
        // Keys as wide as the widest; values take the rest and wrap.
        ui::grid(2).children(rows.into_iter().enumerate().flat_map(|(i, (k, v))| {
            [
                div()
                    .id(("key", i))
                    .child(ui::weak(k.to_string(), cx))
                    .into_any_element(),
                div()
                    .id(("value", i))
                    .min_w_0()
                    .child(ui::strong(v))
                    .into_any_element(),
            ]
        }))
    })
    .into_any_element()
}

fn storage_card(
    info: &SystemInfo,
    snap: &Snapshot,
    is_open: &dyn Fn(&str) -> bool,
    cx: &mut Context<OmniApp>,
) -> Option<AnyElement> {
    if info.drives.is_empty() {
        return None;
    }
    let total: u64 = info.drives.iter().map(|d| d.size_bytes).sum();
    let sub = format!(
        "{}  ·  {} total",
        ui::plural(info.drives.len(), "drive"),
        format_bytes(total)
    );
    let card = card("storage-card", DISK, "Storage", &sub, is_open, cx, |cx| {
        let mut body = v_flex().gap_1();
        for (i, d) in info.drives.iter().enumerate() {
            if i > 0 {
                body = body.child(Separator::horizontal());
            }
            let pre = d.sensor_prefix();
            let mut line = vec![d.kind().to_string(), format_bytes(d.size_bytes)];
            if let Some(t) = val(snap, &format!("{pre}/temp")) {
                line.push(format!("{t:.0} °C"));
            }
            if let Some(a) = val(snap, &format!("{pre}/activity")) {
                line.push(format!("{a:.0} % active"));
            }
            let (r, w) = (
                val(snap, &format!("{pre}/read")),
                val(snap, &format!("{pre}/write")),
            );
            if let (Some(r), Some(w)) = (r, w) {
                line.push(format!("R {r:.1} / W {w:.1} MB/s"));
            }
            let mut drive = v_flex()
                .id(("drive", i))
                .gap_1()
                .child(ui::strong(d.name()))
                .child(ui::weak(line.join("  ·  "), cx));
            if let Some(h) = &d.nvme_health {
                drive = drive.child(bar(
                    h.health_pct() as f32 / 100.0,
                    format!(
                        "Health {}%  ·  {:.1} TB written",
                        h.health_pct(),
                        h.data_written_bytes as f64 / 1e12
                    ),
                    heat::DRIVE_WEAR.color(GREEN, 100.0 - h.health_pct() as f64),
                    cx,
                ));
            }
            for (j, v) in d.volumes.iter().enumerate() {
                let used = v.total_bytes.saturating_sub(v.free_bytes);
                let frac = if v.total_bytes > 0 {
                    used as f32 / v.total_bytes as f32
                } else {
                    0.0
                };
                drive = drive.child(div().id(("volume", j)).child(bar(
                    frac,
                    format!(
                        "{}  {} free of {}",
                        v.mount,
                        format_bytes(v.free_bytes),
                        format_bytes(v.total_bytes)
                    ),
                    DISK,
                    cx,
                )));
            }
            body = body.child(drive);
        }
        body
    });
    Some(card.into_any_element())
}

/// "Resizable BAR: On": green when on, orange when off, muted when unknown.
fn rebar_line(g: &GpuAdapter, cx: &mut Context<OmniApp>) -> Div {
    let rebar = g.rebar();
    let (color, hover) = match rebar {
        ReBar::Enabled { bar_bytes } => (
            Some(GREEN),
            format!(
                "The CPU can address all GPU memory ({} BAR).",
                format_bytes(bar_bytes)
            ),
        ),
        ReBar::Disabled { bar_bytes } => (
            Some(ORANGE),
            format!(
                "Only a {} window is CPU-visible. See the Health Check for how to enable it.",
                format_bytes(bar_bytes)
            ),
        ),
        ReBar::Unknown => (None, "Only available for NVIDIA GPUs so far.".to_string()),
    };
    let value = match color {
        Some(c) => ui::colored(rebar.label(), c, cx).font_weight(FontWeight::SEMIBOLD),
        None => ui::weak(rebar.label(), cx),
    };
    ui::row()
        .pt_0p5()
        .child(ui::weak("Resizable BAR", cx))
        .child(ui::with_tip("rebar", value, hover))
}

/// One-line Health Check summary under the header; clicking it opens the Health tab.
fn health_banner(
    summary: Option<(String, Rgba)>,
    gaming_summary: Option<String>,
    cx: &mut Context<OmniApp>,
) -> Div {
    let (text, color) = summary.unwrap_or_else(|| ("No problems found".into(), GREEN));
    let open = || cx.listener(|this: &mut OmniApp, _, _, cx| this.open(Tab::Health, cx));
    let link = |id: &'static str| ui::clickable(id).cursor_pointer();
    ui::wrap_row()
        .child(
            link("health-banner")
                .child(
                    ui::colored(format!("Health Check: {text}"), color, cx)
                        .font_weight(FontWeight::SEMIBOLD),
                )
                .tooltip(ui::tip("Open the Health Check tab"))
                .on_click(open()),
        )
        .children(gaming_summary.map(|g| {
            link("health-banner-gaming")
                .child(
                    ui::label(format!("·  {g}")).text_color(super::health::level_color(
                        omnihw_core::diagnostics::Level::Info,
                    )),
                )
                .tooltip(ui::tip("Open the Health Check tab (Gaming section)"))
                .on_click(open())
        }))
        .child(
            link("health-banner-arrow")
                .child(ui::colored("›", color, cx).font_weight(FontWeight::SEMIBOLD))
                .on_click(open()),
        )
}

#[cfg(test)]
mod tests {
    use crate::app::Tab;
    use crate::sampler::Snapshot;
    use crate::settings::Settings;
    use crate::tabs::page_tests::{assert_shows, go, machines, open, page, snapshot, texts};
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{AppContext, TestAppContext};

    #[gpui_kit::test]
    fn overview_page(cx: &mut TestAppContext) {
        for info in machines() {
            let shown = page(cx, &info, snapshot(&info), Tab::Overview);
            assert_shows(
                &shown,
                &[
                    &info.cpu.brand,
                    &info.gpus[0].name,
                    "Health Check: ",
                    "Processor",
                    "Memory",
                    "System",
                    "Storage",
                    "Clock (avg)",
                    "Resizable BAR",
                    "Usage",
                    "Uptime",
                ],
            );
        }
    }

    #[gpui_kit::test]
    fn overview_before_the_first_sample(cx: &mut TestAppContext) {
        for info in machines() {
            let shown = page(cx, &info, Snapshot::default(), Tab::Overview);
            assert_shows(
                &shown,
                &["Processor", "Waiting for first sample", &info.gpus[0].name],
            );
        }
    }

    #[gpui_kit::test]
    fn health_banner_opens_the_health_tab(cx: &mut TestAppContext) {
        let info = crate::tabs::page_tests::machines()[0].clone();
        let (window, app) = open(cx, info.clone(), snapshot(&info), Settings::default());
        go(cx, &app, Tab::Overview);
        texts(cx, window);
        cx.update_window(window, |_, window, cx| window.click("health-banner", cx))
            .unwrap();
        assert_eq!(cx.read(|cx| app.read(cx).tab), Tab::Health);
    }

    /// Wide windows put the GPU card in the right column (after the left column's Memory
    /// card); narrow ones stack the cards with the GPU right after the CPU.
    #[gpui_kit::test]
    fn narrow_windows_stack_the_cards(cx: &mut TestAppContext) {
        let info = crate::tabs::page_tests::machines()[0].clone();
        let (window, app) = open(cx, info.clone(), snapshot(&info), Settings::default());
        go(cx, &app, Tab::Overview);
        let order = |shown: &[String]| {
            let at = |s: &str| shown.iter().position(|t| t.starts_with(s)).unwrap();
            at("Resizable BAR") < at("Memory  ")
        };
        let shown = texts(cx, window);
        assert!(!order(&shown), "two columns");
        cx.simulate_window_resize(
            window,
            gpui_kit::size(gpui_kit::px(700.0), gpui_kit::px(3000.0)),
        );
        let shown = texts(cx, window);
        assert!(order(&shown), "one column");
        assert_shows(
            &shown,
            &["Processor", "System", "Storage", &info.gpus[0].name],
        );
    }

    #[gpui_kit::test]
    fn cards_collapse(cx: &mut TestAppContext) {
        let info = crate::tabs::page_tests::machines()[0].clone();
        let (window, app) = open(cx, info.clone(), snapshot(&info), Settings::default());
        go(cx, &app, Tab::Overview);
        assert_shows(&texts(cx, window), &["Fastest Core"]);
        cx.update_window(window, |_, window, cx| {
            window.click("overview_card/Processor", cx)
        })
        .unwrap();
        let shown = texts(cx, window);
        assert!(!shown.iter().any(|t| t.contains("Fastest Core")));
        assert_shows(&shown, &["Processor"]);
    }
}
