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

/// egui-memory flag: the user asked to read the root-only data this frame.
const ROOT_READ: &str = "root_read";

/// "Read as root..." for data only root can read, with the last read's outcome. Nothing when
/// Omni-HW already runs as root. `what` starts the hover text ("Memory module details").
pub fn root_read_button(ui: &mut Ui, snap: &crate::sampler::Snapshot, what: &str) {
    if crate::elevate::elevated() {
        return;
    }
    let state = &snap.root_read;
    ui.horizontal_wrapped(|ui| {
        if state.busy {
            ui.spinner();
            ui.label("Waiting for root access...");
        } else if ui
            .button("Read as root...")
            .on_hover_text(format!(
                "{what} can only be read by root. This asks for your password (pkexec), reads \
                 them once and stops; Omni-HW itself keeps running as you."
            ))
            .clicked()
        {
            temp::set(ui.ctx(), egui::Id::new(ROOT_READ), true);
        }
        match &state.result {
            Some(Err(e)) => {
                ui.colored_label(crate::heat::ink(ui.visuals(), crate::heat::ORANGE), e);
            }
            Some(Ok(0)) => {
                ui.weak("Nothing more could be read, even as root.");
            }
            _ => {}
        }
    });
}

/// Start the root read if a [`root_read_button`] was clicked. Call once per frame.
pub fn dispatch_root_read(ctx: &egui::Context, sampler: &crate::sampler::Sampler) {
    if temp::take::<bool>(ctx, egui::Id::new(ROOT_READ)).unwrap_or(false) {
        sampler.read_as_root();
    }
}

/// Placeholder for a page that isn't available on Linux yet.
pub fn unsupported(ui: &mut Ui, page: &str) {
    page_title(ui, page);
    ui.add_space(8.0);
    ui.label(format!("The {page} page isn't available on Linux yet."));
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

/// Warning under a memory or disk test when the kernel logged hardware errors during it: red
/// if any was critical, else orange. Shows nothing when there were none.
pub fn hw_error_warning(ui: &mut Ui, events: &[omnihw_core::eventlog::LogEntry]) {
    let Some(text) = omnihw_core::eventlog::during_test_text(events) else {
        return;
    };
    let critical = omnihw_core::eventlog::EventLevel::Critical;
    let color = if events.iter().any(|e| e.level == critical) {
        crate::heat::RED
    } else {
        crate::heat::ORANGE
    };
    let hover = events
        .iter()
        .rev()
        .take(10)
        .map(|e| e.detail.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    ui.add(
        egui::Label::new(RichText::new(text).color(crate::heat::ink(ui.visuals(), color))).wrap(),
    )
    .on_hover_text(hover);
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

#[cfg(test)]
mod ui_tests {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    fn cpu_section(ui: &mut egui::Ui) {
        super::section(
            ui,
            "Processor",
            &[
                ("Name", "AMD Ryzen 7 9800X3D".into()),
                ("Stepping", String::new()),
            ],
        );
    }

    #[test]
    fn section_shows_values_and_dashes() {
        let harness = Harness::new_ui(cpu_section);
        harness.get_by_label("Processor");
        harness.get_by_label("AMD Ryzen 7 9800X3D");
        // Empty values show a dash.
        harness.get_by_label("-");
    }

    #[test]
    fn section_snapshot() {
        let mut harness = Harness::builder()
            .with_size(egui::vec2(360.0, 90.0))
            .build_ui(cpu_section);
        harness.run();
        harness.snapshot("section");
    }
}
