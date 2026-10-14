pub mod about;
pub mod alerts;
pub mod bottleneck;
pub mod caches;
pub mod compare;
pub mod cpu;
pub mod gpu;
pub mod health;
pub mod mainboard;
pub mod memory;
pub mod overview;
pub mod power;
pub mod recording;
pub mod sensors;
pub mod settings;
pub mod storage;

use egui::{RichText, Ui};

/// A titled group box containing a two-column label/value grid.
pub fn section(ui: &mut Ui, title: &str, rows: &[(&str, String)]) {
    ui.group(|ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(title).strong());
        egui::Grid::new(title)
            .num_columns(2)
            .spacing([16.0, 4.0])
            .show(ui, |ui| {
                for (k, v) in rows {
                    ui.label(*k);
                    ui.add(
                        egui::Label::new(
                            RichText::new(if v.is_empty() { "-" } else { v }).strong(),
                        )
                        .wrap(),
                    );
                    ui.end_row();
                }
            });
    });
    ui.add_space(6.0);
}

pub fn or_dash(s: &str) -> String {
    if s.is_empty() {
        "-".into()
    } else {
        s.to_string()
    }
}
