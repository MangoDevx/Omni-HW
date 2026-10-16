use crate::sampler::Sampler;
use egui::RichText;
use omnihw_core::sensors::Tracked;

pub fn show(ui: &mut egui::Ui, sampler: &Sampler) {
    ui.horizontal(|ui| {
        if ui.button("Reset min/max").clicked() {
            sampler.reset_stats();
        }
        let mut ms = sampler.interval_ms();
        egui::ComboBox::from_label("Interval")
            .selected_text(format!("{ms} ms"))
            .show_ui(ui, |ui| {
                for v in crate::settings::INTERVALS_MS {
                    ui.selectable_value(&mut ms, v, format!("{v} ms"));
                }
            });
        if ms != sampler.interval_ms() {
            sampler.set_interval_ms(ms);
        }
    });
    ui.separator();

    let snap = sampler.snapshot();
    let entries = snap.table.entries();
    if entries.is_empty() {
        ui.label("Waiting for first sample...");
        return;
    }

    // Groups in first-seen order.
    let mut groups: Vec<&str> = Vec::new();
    for t in entries {
        if !groups.contains(&t.reading.group.as_str()) {
            groups.push(&t.reading.group);
        }
    }
    for g in groups {
        egui::CollapsingHeader::new(RichText::new(g).strong())
            .default_open(true)
            .show(ui, |ui| {
                sensor_grid(ui, g, entries.iter().filter(|t| t.reading.group == g));
            });
    }
}

/// Sensor | Current | Min | Max | Average table.
pub fn sensor_grid<'a>(ui: &mut egui::Ui, id_salt: &str, rows: impl Iterator<Item = &'a Tracked>) {
    egui::Grid::new(id_salt)
        .striped(true)
        .num_columns(5)
        .spacing([16.0, 2.0])
        .min_col_width(70.0)
        .show(ui, |ui| {
            super::header_row(ui, &["Sensor", "Current", "Min", "Max", "Average"]);
            for t in rows {
                let u = t.reading.unit;
                ui.label(&t.reading.label);
                ui.label(
                    RichText::new(u.format(t.reading.value))
                        .strong()
                        .monospace(),
                );
                ui.label(RichText::new(u.format(t.min)).monospace());
                ui.label(RichText::new(u.format(t.max)).monospace());
                ui.label(RichText::new(u.format(t.avg())).monospace());
                ui.end_row();
            }
        });
}
