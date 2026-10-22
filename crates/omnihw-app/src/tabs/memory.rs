use super::{ago, group_box, or_dash, pill, section, temp};
use crate::heat::{GREEN, ORANGE, RED};
use crate::sampler::{Sampler, Snapshot};
use egui::{Color32, Id, RichText, Ui};
use omnihw_core::SystemInfo;
use omnihw_core::memtest::{MemTestResult, Verdict};
use std::time::Duration;

/// egui-memory flag: the user asked for a memory test this frame.
const RUN: &str = "mem_test";

pub fn show(ui: &mut egui::Ui, info: &SystemInfo, snap: &Snapshot) {
    let dimms: Vec<_> = info
        .smbios
        .iter()
        .flat_map(|s| s.populated_memory())
        .collect();
    let first = dimms.first();
    let usable_mb = info.memory.map(|m| m.total_bytes / (1024 * 1024));
    let installed_mb: u64 = dimms.iter().map(|d| d.size_mb).sum();
    let modules = match &info.smbios {
        Some(s) if s.memory_slots > 0 => format!("{} of {} slots", dimms.len(), s.memory_slots),
        _ => dimms.len().to_string(),
    };

    let mut rows = vec![
        (
            "Type",
            first.map(|d| d.memory_type.to_string()).unwrap_or_default(),
        ),
        (
            "Installed",
            if installed_mb > 0 {
                format!("{} GB", installed_mb / 1024)
            } else {
                String::new()
            },
        ),
        (
            "Usable",
            usable_mb
                .map(|m| format!("{:.1} GB", m as f64 / 1024.0))
                .unwrap_or_default(),
        ),
        ("Modules", modules),
    ];
    if let Some(d) = first {
        rows.push((
            "Speed (configured)",
            format!("{} MT/s", d.configured_speed_mts),
        ));
    }
    if let Some(t) = snap.table.get("mem/load") {
        rows.push(("Load", format!("{:.0} %", t.reading.value)));
    }
    section(ui, "General", &rows);
    quick_test(ui, snap);

    if dimms.is_empty() {
        if !crate::elevate::elevated() {
            ui.weak("Module details come from the SMBIOS tables, which only root can read.");
            super::root_read_button(ui, snap, "Memory module details");
        }
        return;
    }
    ui.label(egui::RichText::new("Modules (SMBIOS)").strong());
    egui::Grid::new("dimms")
        .striped(true)
        .num_columns(8)
        .spacing([12.0, 4.0])
        .show(ui, |ui| {
            super::header_row(
                ui,
                &[
                    "Slot",
                    "Size",
                    "Type",
                    "Rated",
                    "Configured",
                    "Ranks",
                    "Manufacturer",
                    "Part Number",
                ],
            );
            for d in &dimms {
                ui.label(&d.locator);
                ui.label(format!("{} GB", d.size_mb / 1024));
                ui.label(format!("{} {}", d.memory_type, d.form_factor));
                ui.label(format!("{} MT/s", d.speed_mts));
                ui.label(format!("{} MT/s", d.configured_speed_mts));
                ui.label(if d.rank > 0 {
                    d.rank.to_string()
                } else {
                    "-".into()
                });
                ui.label(or_dash(&d.manufacturer));
                ui.label(or_dash(&d.part_number));
                ui.end_row();
            }
        });
}

fn quick_test(ui: &mut Ui, snap: &Snapshot) {
    let state = &snap.mem_test;
    group_box(ui, RichText::new("Quick test").strong(), 2.0, 6.0, |ui| {
        ui.add(
            egui::Label::new(
                RichText::new(
                    "Measures read, write and copy bandwidth and latency in a few seconds. \
                     It uses most CPU cores briefly; nothing is changed.",
                )
                .weak(),
            )
            .wrap(),
        );
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if state.running {
                let (phase, step, total) = state.progress.read();
                ui.add(egui::Spinner::new());
                ui.label(format!("Testing… {} ({step}/{total})", phase.name()));
                // Keep the phase text current while the worker thread runs.
                ui.ctx().request_repaint_after(Duration::from_millis(200));
            } else if ui
                .button(RichText::new("Run memory test").strong())
                .on_hover_text("Takes a few seconds; close heavy apps first for a fair result")
                .clicked()
            {
                temp::set(ui.ctx(), Id::new(RUN), true);
            }
            if let Some((_, at)) = &state.result {
                ui.weak(format!("Last run {}", ago(*at)));
            }
        });
        match &state.result {
            Some((Ok(r), _)) => results(ui, r),
            Some((Err(e), _)) => {
                ui.colored_label(crate::heat::ink(ui.visuals(), RED), e);
            }
            None => {}
        }
        super::whea_warning(ui, &state.whea_during);
    });
}

fn results(ui: &mut Ui, r: &MemTestResult) {
    ui.add_space(4.0);
    egui::Grid::new("mem_test_results")
        .num_columns(2)
        .spacing([16.0, 4.0])
        .show(ui, |ui| {
            let row = |ui: &mut Ui, k: &str, v: String| {
                ui.label(k);
                ui.label(RichText::new(v).strong());
                ui.end_row();
            };
            row(ui, "Read", format!("{:.1} GB/s", r.read_gbs));
            row(
                ui,
                "Read (1 core)",
                format!("{:.1} GB/s", r.read_single_gbs),
            );
            row(ui, "Write", format!("{:.1} GB/s", r.write_gbs));
            row(ui, "Copy", format!("{:.1} GB/s", r.copy_gbs));
            row(ui, "Latency", format!("{:.1} ns", r.latency_ns));
            if let Some(e) = &r.expected {
                row(
                    ui,
                    "Expected",
                    format!(
                        "~{:.1} GB/s theoretical ({}-{}, {} channel{}{})",
                        e.theoretical_gbs,
                        e.memory_type,
                        e.mts,
                        e.channels,
                        if e.channels == 1 { "" } else { "s" },
                        if e.channels_estimated {
                            ", estimated"
                        } else {
                            ""
                        },
                    ),
                );
            }
            if let Some(p) = r.efficiency_pct() {
                row(ui, "Read efficiency", format!("{p:.0} %"));
            }
        });
    let single_on = match r.read_cpu {
        Some(id) => format!("CPU {}", id.number),
        None => "one thread".to_string(),
    };
    ui.weak(format!(
        "{} threads ({single_on} for the 1-core read), {} MB buffer. One core can't use every \
         channel, so its read is lower. Copy counts bytes read plus bytes written.",
        r.threads, r.buffer_mb
    ));
    let Some(e) = &r.expected else {
        ui.weak("No memory speed in SMBIOS, so there's nothing to compare with.");
        return;
    };
    let below_rated = e.rated_mts > e.mts;
    if let Some(v) = r.bandwidth_verdict() {
        let text = match v {
            Verdict::Good => "About what this memory should deliver.".to_string(),
            _ if below_rated => format!(
                "Lower than expected, and the memory runs at {} MT/s although it's rated for {}. \
                 Enable XMP/EXPO in the BIOS, and check that the DIMMs are in the slots the \
                 manual recommends for dual channel.",
                e.mts, e.rated_mts
            ),
            _ => "Lower than expected for this memory. Check that the DIMMs are in the slots \
                the manual recommends for dual channel and that XMP/EXPO is enabled. \
                Background load also lowers the result."
                .to_string(),
        };
        verdict_line(ui, "Bandwidth", v, &text);
    }
    if let Some(v) = r.latency_verdict() {
        let (lo, hi) = e.latency_typical_ns;
        let text = match v {
            Verdict::Good => format!(
                "Within the typical {lo:.0}-{hi:.0} ns for {}.",
                e.memory_type
            ),
            _ => format!(
                "Slower than the typical {lo:.0}-{hi:.0} ns for {}. Loose timings, low memory \
                 speed or background load raise latency.",
                e.memory_type
            ),
        };
        verdict_line(ui, "Latency", v, &text);
    }
}

fn verdict_line(ui: &mut Ui, what: &str, v: Verdict, text: &str) {
    ui.horizontal_wrapped(|ui| {
        ui.label(format!("{what}:"));
        pill(ui, v.name(), verdict_color(v), true);
        ui.add(egui::Label::new(text).wrap());
    });
}

fn verdict_color(v: Verdict) -> Color32 {
    match v {
        Verdict::Good => GREEN,
        Verdict::Low => ORANGE,
        Verdict::VeryLow => RED,
    }
}

/// Start a memory test if the button asked for one. Call once per frame.
pub fn dispatch(ctx: &egui::Context, sampler: &Sampler) {
    if temp::take::<bool>(ctx, Id::new(RUN)).unwrap_or(false) {
        sampler.run_mem_test();
    }
}
