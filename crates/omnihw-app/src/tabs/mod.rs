//! Pages, plus the small widgets and helpers they share.

pub mod about;
pub mod alerts;
pub mod bottleneck;
pub mod caches;
pub mod compare;
pub mod cpu;
pub mod eventlog;
pub mod gpu;
pub mod health;
pub mod lansec;
pub mod mainboard;
pub mod memory;
pub mod network;
pub mod overview;
pub mod power;
pub mod recording;
pub mod sensors;
pub mod settings;
pub mod storage;

use egui::{Color32, RichText, Ui};

/// Large bold page title ("Health Check", "Power", ...).
pub fn page_title(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(20.0).strong());
}

/// A full-width group box: `title`, a `title_gap`, then `body`; `gap_after` below the box.
pub fn group_box(
    ui: &mut Ui,
    title: RichText,
    title_gap: f32,
    gap_after: f32,
    body: impl FnOnce(&mut Ui),
) {
    ui.group(|ui| {
        ui.set_width(ui.available_width());
        ui.label(title);
        if title_gap > 0.0 {
            ui.add_space(title_gap);
        }
        body(ui);
    });
    ui.add_space(gap_after);
}

/// A titled group box containing a two-column label/value grid (empty values show "-").
pub fn section(ui: &mut Ui, title: &str, rows: &[(&str, String)]) {
    group_box(ui, RichText::new(title).strong(), 0.0, 6.0, |ui| {
        egui::Grid::new(title)
            .num_columns(2)
            .spacing([16.0, 4.0])
            .show(ui, |ui| {
                for (k, v) in rows {
                    ui.label(*k);
                    ui.add(egui::Label::new(RichText::new(or_dash(v)).strong()).wrap());
                    ui.end_row();
                }
            });
    });
}

/// `s`, or "-" when it's empty.
pub fn or_dash(s: &str) -> &str {
    if s.is_empty() { "-" } else { s }
}

/// Underlined column headings as the first row of a grid.
pub fn header_row(ui: &mut Ui, headings: &[&str]) {
    for h in headings {
        ui.label(RichText::new(*h).underline());
    }
    ui.end_row();
}

/// Small rounded tag with a tinted fill, used for counts, levels and markers.
pub fn pill(ui: &mut Ui, text: impl Into<String>, color: Color32, small: bool) {
    let ink = crate::heat::ink(ui.visuals(), color);
    let mut t = RichText::new(text.into()).strong().color(ink);
    if small {
        t = t.small();
    }
    egui::Frame::new()
        .fill(color.gamma_multiply(0.16))
        .corner_radius(10)
        .inner_margin(egui::Margin::symmetric(7, 1))
        .show(ui, |ui| ui.label(t));
}

/// "1 warning", "2 warnings".
pub fn plural(n: usize, noun: &str) -> String {
    format!("{n} {noun}{}", if n == 1 { "" } else { "s" })
}

/// "just now" or "N min ago" for a test or scan that finished at `t`.
pub fn ago(t: std::time::SystemTime) -> String {
    let s = t.elapsed().map(|d| d.as_secs()).unwrap_or(0);
    if s < 60 {
        "just now".into()
    } else {
        format!("{} min ago", s / 60)
    }
}

/// Per-page values kept in egui's temporary memory (dialog flags, edit buffers, last errors).
pub mod temp {
    use egui::{Context, Id};

    pub fn get<T: Clone + Send + Sync + 'static>(ctx: &Context, id: Id) -> Option<T> {
        ctx.data(|d| d.get_temp(id))
    }

    pub fn set<T: Clone + Send + Sync + 'static>(ctx: &Context, id: Id, value: T) {
        ctx.data_mut(|d| d.insert_temp(id, value));
    }

    /// Remove and return the value.
    pub fn take<T: Clone + Default + Send + Sync + 'static>(ctx: &Context, id: Id) -> Option<T> {
        ctx.data_mut(|d| d.remove_temp(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plurals_and_dashes() {
        assert_eq!(plural(1, "warning"), "1 warning");
        assert_eq!(plural(0, "issue"), "0 issues");
        assert_eq!(plural(3, "suggestion"), "3 suggestions");
        assert_eq!(or_dash(""), "-");
        assert_eq!(or_dash("x"), "x");
    }
}
