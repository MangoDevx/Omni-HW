//! Dashboard: the CPU-Z identity, GPU-Z card details and HWiNFO live sensors on one page.

mod widgets;

use super::health::Report;
use crate::heat::{self, GREEN, ORANGE, RED, Severity, lerp};
use crate::sampler::Snapshot;
use egui::{Align2, Color32, FontId, RichText, Sense, Ui, vec2};
use omnihw_core::gpu::{GpuAdapter, ReBar};
use omnihw_core::sensors::{CPU_PACKAGE_POWER, CPU_TEMP};
use omnihw_core::{SystemInfo, format_bytes, format_uptime};
use std::collections::VecDeque;
use widgets::{Metric, bar, card, m, metrics, severity_icon, sparkline, sparkline_colored};

const CPU: Color32 = heat::BLUE;
const GPU: Color32 = Color32::from_rgb(38, 166, 154);
const MEM: Color32 = Color32::from_rgb(171, 71, 188);
const SYS: Color32 = Color32::from_rgb(245, 156, 26);
const DISK: Color32 = Color32::from_rgb(92, 107, 192);

/// Two columns from this width up.
const TWO_COLUMN_WIDTH: f32 = 760.0;

/// Returns true when the user clicks the Health Check summary.
pub fn show(ui: &mut Ui, info: &SystemInfo, snap: &Snapshot, health: &Report) -> bool {
    header(ui, info);
    let open_health = health_banner(ui, health);
    ui.add_space(8.0);
    if ui.available_width() >= TWO_COLUMN_WIDTH {
        ui.columns(2, |c| {
            cpu_card(&mut c[0], info, snap);
            memory_card(&mut c[0], info, snap);
            system_card(&mut c[0], info, snap);
            gpu_cards(&mut c[1], info, snap);
            storage_card(&mut c[1], info, snap);
        });
    } else {
        cpu_card(ui, info, snap);
        gpu_cards(ui, info, snap);
        memory_card(ui, info, snap);
        storage_card(ui, info, snap);
        system_card(ui, info, snap);
    }
    open_health
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

fn header(ui: &mut Ui, info: &SystemInfo) {
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
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(&info.cpu.brand).size(18.0).strong());
        for g in &info.gpus {
            ui.label(RichText::new("·").size(18.0).weak());
            ui.label(RichText::new(&g.name).size(18.0).strong());
        }
    });
    if !parts.is_empty() {
        ui.weak(parts.join("  ·  "));
    }
}

fn ghz(mhz: f64) -> String {
    format!("{:.2} GHz", mhz / 1000.0)
}

fn cpu_card(ui: &mut Ui, info: &SystemInfo, snap: &Snapshot) {
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
    card(ui, CPU, "Processor", &sub.join("  ·  "), |ui| {
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
        metrics(ui, &row);
        ui.add_space(4.0);
        sparkline(ui, "Usage", hist(snap, "cpu/load/total"), Some(100.0), CPU);
        ui.add_space(6.0);
        core_tiles(ui, info, snap);
    });
}

/// One tile per physical core showing its clock. Blue shade follows load; orange with a
/// warning icon / red with an exclamation icon when the busiest thread crosses `heat::CORE_LOAD`.
fn core_tiles(ui: &mut Ui, info: &SystemInfo, snap: &Snapshot) {
    let Some(t) = &info.topology else { return };
    let base = ui.visuals().extreme_bg_color;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
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
            let (rect, resp) = ui.allocate_exact_size(vec2(58.0, 40.0), Sense::hover());
            let p = ui.painter();
            let sev = heat::CORE_LOAD.severity(load);
            let fill = match sev {
                Severity::Normal => lerp(base, CPU, 0.15 + 0.85 * (load / 100.0) as f32),
                Severity::Warning => lerp(base, ORANGE, 0.75),
                Severity::Critical => lerp(base, RED, 0.85),
            };
            p.rect_filled(rect, 4, fill);
            // Black or white by tile brightness, so tiles read in both themes.
            let text = crate::heat::text_on(fill);
            let mut label_pos = rect.left_top() + vec2(5.0, 3.0);
            if sev != Severity::Normal {
                severity_icon(p, rect.left_top() + vec2(10.0, 10.0), sev);
                label_pos.x += 14.0;
            }
            p.text(
                label_pos,
                Align2::LEFT_TOP,
                &short,
                FontId::proportional(10.0),
                text,
            );
            if let Some(c) = clock {
                p.text(
                    rect.center_bottom() - vec2(0.0, 4.0),
                    Align2::CENTER_BOTTOM,
                    format!("{:.2}", c / 1000.0),
                    FontId::monospace(13.0),
                    text,
                );
            }
            resp.on_hover_ui(|ui| {
                ui.strong(&label);
                match sev {
                    Severity::Warning => {
                        ui.colored_label(
                            crate::heat::ink(ui.visuals(), ORANGE),
                            format!("Heavy load ({load:.0} %)"),
                        );
                    }
                    Severity::Critical => {
                        ui.colored_label(
                            crate::heat::ink(ui.visuals(), RED),
                            format!("Saturated ({load:.0} %)"),
                        );
                    }
                    Severity::Normal => {}
                }
                if let Some(t) = temp {
                    ui.label(format!("Temperature: {t:.0} °C"));
                }
                if let Some(c) = clock {
                    ui.label(format!("Clock: {c:.0} MHz"));
                }
                for (l, v) in core.logical.iter().zip(&loads) {
                    ui.label(format!("Thread {}: {v:.0} %", l.number));
                }
            });
        }
    });
}

fn gpu_cards(ui: &mut Ui, info: &SystemInfo, snap: &Snapshot) {
    if info.gpus.is_empty() {
        card(ui, GPU, "Graphics", "No hardware adapter found", |_| {});
    }
    for g in &info.gpus {
        gpu_card(ui, g, snap);
    }
}

fn gpu_card(ui: &mut Ui, g: &GpuAdapter, snap: &Snapshot) {
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

    card(ui, GPU, &g.name, &sub.join("  ·  "), |ui| {
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
        metrics(ui, &row);
        ui.add_space(4.0);
        sparkline(ui, "Load", hist(snap, &load_id), Some(100.0), GPU);
        if let Some(h) = hist(snap, &id("temp")) {
            ui.add_space(4.0);
            sparkline_colored(ui, "Temperature", Some(h), Some(100.0), |v| {
                heat::GPU_TEMP.color(GREEN, v as f64)
            });
        }
        ui.add_space(6.0);
        let used = val(snap, &id("vram/used")).or_else(|| val(snap, &id("vram/dedicated")));
        let total_mb = g.dedicated_vram as f64 / 1048576.0;
        if let Some(u) = used.filter(|_| total_mb > 0.0) {
            let frac = u / total_mb;
            bar(
                ui,
                frac as f32,
                format!("VRAM {:.1} / {:.1} GB", u / 1024.0, total_mb / 1024.0),
                heat::VRAM_USED.color(GPU, frac),
            );
        }
        rebar_line(ui, g);
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
            ui.add_space(2.0);
            ui.weak(extras.join("  ·  "));
        }
    });
}

fn memory_card(ui: &mut Ui, info: &SystemInfo, snap: &Snapshot) {
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
    card(ui, MEM, &title, &sub.join("  ·  "), |ui| {
        if let (Some(used), Some(total)) = (val(snap, "mem/used"), total_mb) {
            let frac = used / total;
            bar(
                ui,
                frac as f32,
                format!("{:.1} / {:.1} GB used", used / 1024.0, total / 1024.0),
                heat::MEMORY_USED.color(MEM, frac),
            );
        } else {
            ui.weak("Waiting for first sample…");
        }
        ui.add_space(4.0);
        sparkline(ui, "Load", hist(snap, "mem/load"), Some(100.0), MEM);
    });
}

fn system_card(ui: &mut Ui, info: &SystemInfo, snap: &Snapshot) {
    card(ui, SYS, "System", "", |ui| {
        let mut rows: Vec<(&str, String)> = Vec::new();
        if let Some(o) = &info.os {
            let os = if o.kernel.is_empty() { "Windows" } else { "OS" };
            rows.push((os, o.summary()));
            rows.push(("Computer", o.computer_name.clone()));
        }
        if let Some(u) = omnihw_core::platform::uptime_secs() {
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
        egui::Grid::new("sys_rows")
            .num_columns(2)
            .spacing([16.0, 4.0])
            .show(ui, |ui| {
                for (k, v) in rows {
                    ui.weak(k);
                    ui.add(egui::Label::new(RichText::new(v).strong()).wrap());
                    ui.end_row();
                }
            });
    });
}

fn storage_card(ui: &mut Ui, info: &SystemInfo, snap: &Snapshot) {
    if info.drives.is_empty() {
        return;
    }
    let total: u64 = info.drives.iter().map(|d| d.size_bytes).sum();
    let sub = format!(
        "{} drives  ·  {} total",
        info.drives.len(),
        format_bytes(total)
    );
    card(ui, DISK, "Storage", &sub, |ui| {
        for (i, d) in info.drives.iter().enumerate() {
            if i > 0 {
                ui.separator();
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
            ui.label(RichText::new(d.name()).strong());
            ui.weak(line.join("  ·  "));
            if let Some(h) = &d.nvme_health {
                bar(
                    ui,
                    h.health_pct() as f32 / 100.0,
                    format!(
                        "Health {}%  ·  {:.1} TB written",
                        h.health_pct(),
                        h.data_written_bytes as f64 / 1e12
                    ),
                    heat::DRIVE_WEAR.color(GREEN, 100.0 - h.health_pct() as f64),
                );
            }
            for v in &d.volumes {
                let used = v.total_bytes.saturating_sub(v.free_bytes);
                let frac = if v.total_bytes > 0 {
                    used as f32 / v.total_bytes as f32
                } else {
                    0.0
                };
                bar(
                    ui,
                    frac,
                    format!(
                        "{}  {} free of {}",
                        v.mount,
                        format_bytes(v.free_bytes),
                        format_bytes(v.total_bytes)
                    ),
                    DISK,
                );
            }
        }
    });
}

/// "Resizable BAR: On": green when on, orange when off, muted when unknown.
fn rebar_line(ui: &mut Ui, g: &GpuAdapter) {
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
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.weak("Resizable BAR");
        let resp = match color {
            Some(c) => ui.label(
                RichText::new(rebar.label())
                    .strong()
                    .color(crate::heat::ink(ui.visuals(), c)),
            ),
            None => ui.weak(rebar.label()),
        };
        resp.on_hover_text(hover);
    });
}

/// One-line Health Check summary under the header; clicking it opens the Health tab.
fn health_banner(ui: &mut Ui, health: &Report) -> bool {
    let (text, color) = match &health.summary {
        Some((text, color)) => (text.as_str(), *color),
        None => ("No problems found", GREEN),
    };
    ui.horizontal_wrapped(|ui| {
        let mut clicked = ui
            .add(
                egui::Button::new(
                    RichText::new(format!("Health Check: {text}"))
                        .color(crate::heat::ink(ui.visuals(), color))
                        .strong(),
                )
                .frame(false),
            )
            .on_hover_text("Open the Health Check tab")
            .clicked();
        if let Some(g) = &health.gaming_summary {
            clicked |= ui
                .add(
                    egui::Button::new(RichText::new(format!("·  {g}")).color(
                        super::health::level_color(omnihw_core::diagnostics::Level::Info),
                    ))
                    .frame(false),
                )
                .on_hover_text("Open the Health Check tab (Gaming section)")
                .clicked();
        }
        clicked |= ui
            .add(
                egui::Button::new(
                    RichText::new("›")
                        .color(crate::heat::ink(ui.visuals(), color))
                        .strong(),
                )
                .frame(false),
            )
            .clicked();
        clicked
    })
    .inner
}
