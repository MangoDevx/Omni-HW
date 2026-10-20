//! Event Log page: crashes, hardware errors, GPU driver timeouts and disk problems from the
//! System and Application logs, in plain English, with filters. The logs are read on a worker
//! thread (see [`Sampler::read_event_log`]); this page only draws the last read.

use crate::heat::{ORANGE, RED, YELLOW, ink};
use crate::sampler::Sampler;
use egui::{Button, Color32, RichText, Sense, Ui};
use omnihw_core::eventlog::{EventCategory, EventLevel, Filter, LogEntry, VIEW_DAYS, to_text};
use std::collections::HashSet;
use std::time::{SystemTime, UNIX_EPOCH};

/// Filters and expanded rows, kept while the app runs.
#[derive(Default)]
pub struct ViewState {
    filter: Filter,
    /// Rows showing their detail (indices into the entries of the read finished at `loaded`).
    expanded: HashSet<usize>,
    loaded: Option<SystemTime>,
}

/// Date range choices: (label, seconds).
const RANGES: [(&str, i64); 3] = [
    ("24 h", 86_400),
    ("7 days", 7 * 86_400),
    ("30 days", VIEW_DAYS as i64 * 86_400),
];

fn level_color(l: EventLevel) -> Color32 {
    match l {
        EventLevel::Critical => RED,
        EventLevel::Error => ORANGE,
        EventLevel::Warning => YELLOW,
    }
}

/// A coloured dot and the level name.
fn level_cell(ui: &mut Ui, l: EventLevel) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), Sense::hover());
        ui.painter()
            .circle_filled(rect.center(), 4.5, level_color(l));
        ui.label(l.name());
    });
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// "Analyze crash dumps" for the logged blue screens, and what the dumps point at. The same
/// results feed the Health Check's blue screen finding.
fn crash_dumps(ui: &mut Ui, sampler: &Sampler) {
    let (dumps, busy) = {
        let snap = sampler.snapshot();
        (snap.dumps.clone(), snap.dumps_busy)
    };
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        let (label, hover) = if crate::elevate::elevated() {
            ("Analyze crash dumps", "Reads C:\\Windows\\Minidump.")
        } else {
            (
                "Analyze crash dumps (asks for administrator)",
                "Reads C:\\Windows\\Minidump. Windows asks for administrator permission because only administrators can read crash dumps.",
            )
        };
        if ui
            .add_enabled(!busy, Button::new(label))
            .on_hover_text(hover)
            .clicked()
        {
            sampler.read_dumps();
        }
        if busy {
            ui.spinner();
        }
    });
    match dumps {
        _ if busy => {}
        Some(Err(e)) => {
            ui.colored_label(
                ink(ui.visuals(), RED),
                format!("Couldn't read crash dumps: {e}"),
            );
        }
        Some(Ok(d)) if d.is_empty() => {
            ui.weak("No crash dumps found");
        }
        Some(Ok(d)) => {
            for line in d.iter().map(omnihw_core::diagnostics::dump_line) {
                ui.label(format!("• {line}"));
            }
        }
        None => {}
    }
}

pub fn show(ui: &mut Ui, sampler: &Sampler, state: &mut ViewState) {
    // Only an Arc clone under the lock.
    let (log, busy) = {
        let snap = sampler.snapshot();
        (snap.event_log.clone(), snap.event_log_busy)
    };

    super::page_title(ui, "Event Log");
    ui.weak(format!(
        "Crashes, hardware errors, graphics driver timeouts and disk problems Windows logged in the last {VIEW_DAYS} days, in plain English. Click an event's meaning for its details."
    ));
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if ui
            .add_enabled(!busy, Button::new("Refresh"))
            .on_hover_text("Read the System and Application event logs again")
            .clicked()
        {
            sampler.read_event_log();
        }
        if busy {
            ui.spinner();
            ui.weak("Reading the event logs...");
        } else if let Some((_, at)) = &log {
            ui.weak(format!("Read {}", super::ago(*at)));
        }
    });
    let Some((log, at)) = log else {
        return;
    };
    if state.loaded != Some(at) {
        state.expanded.clear();
        state.loaded = Some(at);
    }
    for e in &log.errors {
        ui.colored_label(ink(ui.visuals(), ORANGE), format!("Couldn't read the {e}"));
    }
    if log
        .entries
        .iter()
        .any(|e| e.category == EventCategory::BlueScreen)
    {
        crash_dumps(ui, sampler);
    }
    ui.add_space(4.0);

    let f = &mut state.filter;
    ui.horizontal_wrapped(|ui| {
        egui::ComboBox::from_id_salt("eventlog_category")
            .width(170.0)
            .selected_text(f.category.map_or("All categories", EventCategory::name))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut f.category, None, "All categories");
                for c in EventCategory::ALL {
                    let n = log.entries.iter().filter(|e| e.category == c).count();
                    ui.selectable_value(&mut f.category, Some(c), format!("{} ({n})", c.name()));
                }
            });
        egui::ComboBox::from_id_salt("eventlog_level")
            .width(100.0)
            .selected_text(f.level.map_or("All levels", EventLevel::name))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut f.level, None, "All levels");
                for l in EventLevel::ALL {
                    ui.selectable_value(&mut f.level, Some(l), l.name());
                }
            });
        ui.separator();
        for (label, secs) in RANGES {
            ui.selectable_value(&mut f.within_secs, secs, label);
        }
        ui.separator();
        ui.add(
            egui::TextEdit::singleline(&mut f.text)
                .hint_text("Search")
                .desired_width(180.0),
        );
    });

    let now = now_unix();
    let shown: Vec<(usize, &LogEntry)> = log
        .entries
        .iter()
        .enumerate()
        .filter(|(_, e)| state.filter.matches(e, now))
        .collect();
    ui.horizontal(|ui| {
        ui.label(format!(
            "{} of {}",
            shown.len(),
            super::plural(log.entries.len(), "event")
        ));
        if ui
            .add_enabled(!shown.is_empty(), Button::new("Copy"))
            .on_hover_text("Copy the events shown as plain text")
            .clicked()
        {
            ui.ctx().copy_text(to_text(shown.iter().map(|&(_, e)| e)));
        }
    });
    ui.add_space(4.0);
    if log.entries.is_empty() {
        ui.weak(format!("No critical events in the last {VIEW_DAYS} days"));
        return;
    }
    if shown.is_empty() {
        ui.weak("No events match these filters.");
        return;
    }

    egui::Grid::new("eventlog_rows")
        .num_columns(5)
        .spacing([14.0, 4.0])
        .striped(true)
        .show(ui, |ui| {
            super::header_row(ui, &["Time", "Level", "Category", "Source", "Meaning"]);
            for (i, e) in shown {
                ui.label(e.time_text());
                level_cell(ui, e.level);
                ui.label(e.category.name());
                ui.label(format!("{} ({})", e.source(), e.channel.name()));
                ui.vertical(|ui| {
                    let open = state.expanded.contains(&i);
                    let text = if e.detail.is_empty() {
                        e.title.to_string()
                    } else {
                        format!("{} {}", if open { "⏷" } else { "⏵" }, e.title)
                    };
                    let r = ui.add(egui::Label::new(text).sense(Sense::click()));
                    if !e.detail.is_empty() {
                        let r = r.on_hover_cursor(egui::CursorIcon::PointingHand);
                        if r.clicked() && !state.expanded.remove(&i) {
                            state.expanded.insert(i);
                        }
                    }
                    if open && !e.detail.is_empty() {
                        ui.add(egui::Label::new(RichText::new(&e.detail).weak()).wrap());
                    }
                });
                ui.end_row();
            }
        });
}
