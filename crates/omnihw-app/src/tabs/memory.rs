use super::{or_dash, section};
use crate::sampler::Snapshot;
use omnihw_core::SystemInfo;

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

    if dimms.is_empty() {
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
