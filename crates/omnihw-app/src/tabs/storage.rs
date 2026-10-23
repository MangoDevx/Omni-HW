use super::{ago, group_box, pill, section, temp};
use crate::heat::{GREEN, ORANGE, RED};
use crate::sampler::{Sampler, Snapshot};
use egui::{Id, RichText};
use omnihw_core::checkup::PcieLink;
use omnihw_core::disktest::{self, DiskTestResult, Verdict};
use omnihw_core::smart::AtaSmart;
use omnihw_core::storage::{BusType, Drive, sata_gen_speed};
use omnihw_core::{SystemInfo, format_bytes};
use std::time::Duration;

/// egui-memory value: the drive index whose speed test awaits confirmation.
const CONFIRM: &str = "disk_test_confirm";

/// Summary of the key SMART values plus a collapsible table of every attribute.
fn smart_section(ui: &mut egui::Ui, d: &Drive, s: &AtaSmart) {
    let opt = |v: Option<u64>| v.map(|v| v.to_string()).unwrap_or_else(|| "-".into());
    let mut rows = vec![];
    if let Some(life) = s.life_remaining_pct() {
        rows.push(("Life Remaining", format!("{life} %")));
    }
    if let Some(t) = s.temperature_c() {
        rows.push(("Temperature", format!("{t} °C")));
    }
    rows.push(("Power-On Hours", opt(s.power_on_hours())));
    rows.push(("Power Cycles", opt(s.power_cycles())));
    rows.push(("Reallocated Sectors", opt(s.reallocated_sectors())));
    rows.push(("Pending Sectors", opt(s.pending_sectors())));
    rows.push(("Uncorrectable Errors", opt(s.uncorrectable())));
    rows.push(("CRC (Cable) Errors", opt(s.crc_errors())));
    section(ui, "Health (SATA SMART)", &rows);
    egui::CollapsingHeader::new(format!("SMART attributes ({})", s.attributes.len()))
        .id_salt(("smart", d.index))
        .show(ui, |ui| {
            egui::Grid::new(("smart_attrs", d.index))
                .striped(true)
                .num_columns(5)
                .spacing([14.0, 2.0])
                .show(ui, |ui| {
                    super::header_row(ui, &["ID", "Attribute", "Current", "Worst", "Raw"]);
                    for a in &s.attributes {
                        ui.label(RichText::new(format!("{:02X}", a.id)).monospace());
                        ui.label(a.name);
                        ui.label(RichText::new(a.current.to_string()).monospace());
                        ui.label(RichText::new(a.worst.to_string()).monospace());
                        ui.label(RichText::new(a.raw.to_string()).monospace());
                        ui.end_row();
                    }
                });
        });
    ui.add_space(6.0);
}

pub fn show(ui: &mut egui::Ui, info: &SystemInfo, snap: &Snapshot) {
    if info.drives.is_empty() {
        ui.label("No physical drives found.");
        return;
    }
    for d in &info.drives {
        drive(ui, d, snap, &info.checkup.nvme_links);
        ui.add_space(10.0);
    }
}

fn drive(ui: &mut egui::Ui, d: &Drive, snap: &Snapshot, links: &[PcieLink]) {
    ui.label(
        RichText::new(format!("Disk {}: {}", d.index, d.name()))
            .size(16.0)
            .strong(),
    );
    ui.add_space(4.0);
    section(
        ui,
        "Device",
        &[
            ("Type", d.kind().to_string()),
            ("Interface", d.bus.name()),
            ("Capacity", format_bytes(d.size_bytes)),
            ("Firmware", d.firmware.clone()),
            ("Serial Number", d.serial.trim_end_matches('.').to_string()),
            ("Removable", if d.removable { "Yes" } else { "No" }.into()),
        ],
    );
    speed_test(ui, d, snap, disktest::nvme_link(d, links));

    if !d.volumes.is_empty() {
        ui.group(|ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("Volumes").strong());
            for v in &d.volumes {
                let used = v.total_bytes.saturating_sub(v.free_bytes);
                let frac = if v.total_bytes > 0 {
                    used as f32 / v.total_bytes as f32
                } else {
                    0.0
                };
                let label = if v.label.is_empty() {
                    String::new()
                } else {
                    format!(" {}", v.label)
                };
                ui.add(
                    egui::ProgressBar::new(frac)
                        .text(format!(
                            "{}{label} ({}) - {} free of {}",
                            v.mount,
                            v.file_system,
                            format_bytes(v.free_bytes),
                            format_bytes(v.total_bytes)
                        ))
                        .desired_height(18.0),
                );
            }
        });
        ui.add_space(6.0);
    }

    match &d.nvme_health {
        Some(h) => {
            let warnings = h.warnings();
            let tb = |b: u128| format!("{:.2} TB", b as f64 / 1e12);
            section(
                ui,
                "Health (NVMe SMART)",
                &[
                    (
                        "Health",
                        format!("{}% (life used {}%)", h.health_pct(), h.percentage_used),
                    ),
                    (
                        "Available Spare",
                        format!(
                            "{}% (threshold {}%)",
                            h.available_spare_pct, h.spare_threshold_pct
                        ),
                    ),
                    ("Temperature", format!("{} °C", h.temperature_c)),
                    ("Data Written", tb(h.data_written_bytes)),
                    ("Data Read", tb(h.data_read_bytes)),
                    ("Power-On Hours", h.power_on_hours.to_string()),
                    ("Power Cycles", h.power_cycles.to_string()),
                    ("Unsafe Shutdowns", h.unsafe_shutdowns.to_string()),
                    ("Media Errors", h.media_errors.to_string()),
                    (
                        "Critical Warnings",
                        if warnings.is_empty() {
                            "None".into()
                        } else {
                            warnings.join(", ")
                        },
                    ),
                ],
            );
        }
        None if d.bus == omnihw_core::storage::BusType::Nvme => {
            health_unread(
                ui,
                snap,
                "This drive didn't return its NVMe health log.",
                "The NVMe health log can only be read by root.",
            );
        }
        None => {}
    }

    match &d.ata_smart {
        Some(s) => smart_section(ui, d, s),
        None if !matches!(
            d.bus,
            BusType::Nvme
                | BusType::Usb
                | BusType::Virtual
                | BusType::Sd
                | BusType::Mmc
                | BusType::Spaces
        ) =>
        {
            health_unread(
                ui,
                snap,
                "This drive didn't return SMART data.",
                "SMART data can only be read by root.",
            );
        }
        None => {}
    }

    let prefix = format!("{}/", d.sensor_prefix());
    let rows: Vec<_> = snap
        .table
        .entries()
        .iter()
        .filter(|t| t.reading.id.starts_with(&prefix))
        .collect();
    // SATA drives report their temperature through the drivetemp module, which most distros
    // don't load by default.
    let no_temp =
        d.bus == BusType::Sata && !rows.iter().any(|t| t.reading.id == format!("{prefix}temp"));
    if !rows.is_empty() {
        ui.add_space(4.0);
        ui.label(RichText::new("Sensors").strong());
        super::sensors::sensor_grid(ui, &prefix, rows.into_iter());
    }
    if no_temp {
        ui.weak(
            "No temperature: load the drivetemp kernel module (sudo modprobe drivetemp, and              add drivetemp to /etc/modules-load.d/drivetemp.conf to keep it after a restart).",
        );
    }
}

/// The "Health" group for a drive whose health data wasn't read: why, and "Read as root..."
/// when root could read it.
fn health_unread(ui: &mut egui::Ui, snap: &Snapshot, as_root: &str, needs_root: &str) {
    group_box(ui, RichText::new("Health").strong(), 0.0, 6.0, |ui| {
        if crate::elevate::elevated() {
            ui.weak(as_root);
        } else {
            ui.weak(needs_root);
            super::root_read_button(ui, snap, "Drive health");
        }
    });
}

/// The "Speed test" group: run button (confirmed in [`dispatch`]), live progress and the last
/// result for this drive.
fn speed_test(ui: &mut egui::Ui, d: &Drive, snap: &Snapshot, link: Option<&PcieLink>) {
    let state = &snap.disk_test;
    let running_here = state.running == Some(d.index);
    let last = state.results.get(&d.index);
    group_box(ui, RichText::new("Speed test").strong(), 4.0, 6.0, |ui| {
        ui.horizontal(|ui| {
            if running_here {
                ui.add(egui::Spinner::new());
                ui.label("Testing...");
            } else {
                let busy = state.running.is_some();
                let resp = ui
                    .add_enabled(
                        !busy && disktest::pick_volume(d).is_some(),
                        egui::Button::new(RichText::new("Run speed test").strong()),
                    )
                    .on_hover_text("Measure read and write speed with a temporary file");
                let resp = if busy {
                    resp.on_disabled_hover_text("Another drive is being tested")
                } else {
                    resp.on_disabled_hover_text(format!(
                        "No volume with enough free space (needs {} free)",
                        format_bytes(disktest::needed_free_bytes(d))
                    ))
                };
                if resp.clicked() {
                    temp::set(ui.ctx(), Id::new(CONFIRM), Some(d.index));
                }
            }
            if let Some((_, at)) = last {
                ui.weak(format!("Last run {}", ago(*at)));
            }
        });
        if running_here {
            let (phase, frac) = state.progress.read();
            ui.add(
                egui::ProgressBar::new(frac)
                    .text(phase.label())
                    .desired_width(360.0),
            );
            // Keep the progress bar moving while the worker thread runs.
            ui.ctx().request_repaint_after(Duration::from_millis(200));
        }
        match last {
            Some((Ok(r), _)) => results(ui, d, r, link),
            Some((Err(e), _)) => {
                ui.colored_label(crate::heat::ink(ui.visuals(), RED), e);
            }
            None if !running_here => {
                ui.weak(
                    "Measures sequential and 4K random read/write speed and compares it with \
                     what this drive should deliver.",
                );
            }
            None => {}
        }
        if let Some((_, events)) = state.hw_errors.get(&d.index) {
            super::hw_error_warning(ui, events);
        }
    });
}

fn mbs(v: f64) -> String {
    if v < 10.0 {
        format!("{v:.1} MB/s")
    } else {
        format!("{v:.0} MB/s")
    }
}

fn results(ui: &mut egui::Ui, d: &Drive, r: &DiskTestResult, link: Option<&PcieLink>) {
    ui.add_space(4.0);
    egui::Grid::new(("disk_test", d.index))
        .num_columns(2)
        .spacing([16.0, 4.0])
        .show(ui, |ui| {
            for (k, v) in [
                ("Sequential read", r.seq_read_mbs),
                ("Sequential write", r.seq_write_mbs),
                ("4K random read", r.rnd4k_read_mbs),
                ("4K random write", r.rnd4k_write_mbs),
                ("4K random read (QD32)", r.rnd4k_read_q32_mbs),
            ] {
                ui.label(k);
                ui.label(RichText::new(mbs(v)).strong().monospace());
                ui.end_row();
            }
            ui.label("Typical sequential read");
            ui.label(match &r.expected {
                Some(e) => format!(
                    "{:.0}-{:.0} MB/s ({})",
                    e.seq_read_mbs.0, e.seq_read_mbs.1, e.reason
                ),
                None => "Depends on the port or enclosure".into(),
            });
            ui.end_row();
        });
    if let Some(v) = r.verdict() {
        ui.horizontal_wrapped(|ui| {
            let (text, color) = match v {
                Verdict::Good => ("Normal", GREEN),
                Verdict::Low => ("Slow", ORANGE),
                Verdict::VeryLow => ("Very slow", RED),
            };
            pill(ui, text, color, false);
            ui.label(verdict_text(d, v));
        });
    }
    if let Some(note) = link_note(d, link) {
        ui.colored_label(crate::heat::ink(ui.visuals(), ORANGE), note);
    }
    ui.weak(format!(
        "{} test file, deleted afterwards. 4K random tests run at queue depth 1 unless noted.",
        format_bytes(r.file_mb << 20)
    ));
}

fn verdict_text(d: &Drive, v: Verdict) -> &'static str {
    match (v, d.bus, d.ssd) {
        (Verdict::Good, ..) => "Sequential read is in the normal range for this drive.",
        (_, BusType::Nvme, _) => {
            "Slower than this drive's link allows. A nearly full drive, heat throttling or \
             background disk activity can cause this."
        }
        (_, _, Some(false)) => {
            "Slower than a typical hard drive. A failing drive, a very full disk or background \
             disk activity can cause this."
        }
        _ => {
            "Slower than a SATA SSD usually reads. A nearly full or worn drive, the SATA cable \
             or port, or background disk activity can cause this."
        }
    }
}

/// A warning when the drive's link runs below what it supports.
fn link_note(d: &Drive, link: Option<&PcieLink>) -> Option<String> {
    if let Some(l) = link {
        let ((cg, cw), (mg, mw)) = (l.current, l.max);
        if cg > 0 && cw > 0 && (cg < mg || cw < mw) {
            return Some(format!(
                "The PCIe link runs at {cg}.0 x{cw} but the drive supports {mg}.0 x{mw}; a \
                 different M.2 slot may make it faster."
            ));
        }
    }
    let s = d.sata_link?;
    (s.current_gen < s.max_gen).then(|| {
        format!(
            "The SATA link runs at {} but the drive supports {}; try another SATA port or cable.",
            sata_gen_speed(s.current_gen),
            sata_gen_speed(s.max_gen)
        )
    })
}

/// Show the speed test confirmation when a button asked for one, and start the test once
/// confirmed. Call once per frame.
pub fn dispatch(ctx: &egui::Context, sampler: &Sampler) {
    let Some(index) = temp::get::<Option<u32>>(ctx, Id::new(CONFIRM)).flatten() else {
        return;
    };
    let target = sampler.info().and_then(|info| {
        let d = info.drives.iter().find(|d| d.index == index)?;
        let v = disktest::pick_volume(d)?;
        Some((format_bytes(disktest::file_bytes(d)), v.mount.clone()))
    });
    let Some((size, mount)) = target else {
        temp::take::<Option<u32>>(ctx, Id::new(CONFIRM));
        return;
    };
    let mut close = None;
    let modal = egui::Modal::new(Id::new("disk_test_modal")).show(ctx, |ui| {
        ui.set_max_width(420.0);
        ui.label(RichText::new("Run speed test?").size(16.0).strong());
        ui.add_space(6.0);
        ui.add(
            egui::Label::new(format!(
                "Writes a temporary {size} file to {mount} and deletes it afterwards. Takes \
                 about 20 seconds; other disk activity will skew results."
            ))
            .wrap(),
        );
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if ui.button(RichText::new("Run test").strong()).clicked() {
                close = Some(true);
            }
            if ui.button("Cancel").clicked() {
                close = Some(false);
            }
        });
    });
    if modal.should_close() {
        close.get_or_insert(false);
    }
    if let Some(run) = close {
        temp::take::<Option<u32>>(ctx, Id::new(CONFIRM));
        if run {
            sampler.run_disk_test(index);
        }
    }
}
