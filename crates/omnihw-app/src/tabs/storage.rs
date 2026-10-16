use super::section;
use crate::sampler::Snapshot;
use egui::RichText;
use omnihw_core::smart::AtaSmart;
use omnihw_core::storage::{BusType, Drive};
use omnihw_core::{SystemInfo, format_bytes};

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
        drive(ui, d, snap);
        ui.add_space(10.0);
    }
}

fn drive(ui: &mut egui::Ui, d: &Drive, snap: &Snapshot) {
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
            ui.weak("NVMe health log unavailable (try running as administrator).");
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
            ui.weak("SATA SMART data unavailable (try running as administrator).");
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
    if !rows.is_empty() {
        ui.label(RichText::new("Sensors").strong());
        super::sensors::sensor_grid(ui, &prefix, rows.into_iter());
    }
}
