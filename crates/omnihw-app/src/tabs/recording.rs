//! Recording tab: start/stop, pick sensors, and chart them over the whole session.

use crate::sampler::Sampler;
use crate::{export, heat};
use egui::{Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, Ui, pos2, vec2};
use omnihw_core::recording::{Column, Recording, format_elapsed};
use omnihw_core::sensors::Unit;
use std::collections::BTreeSet;

pub(super) const PALETTE: [Color32; 8] = [
    heat::BLUE,
    Color32::from_rgb(38, 166, 154),
    Color32::from_rgb(245, 156, 26),
    Color32::from_rgb(171, 71, 188),
    Color32::from_rgb(239, 83, 80),
    Color32::from_rgb(124, 179, 66),
    Color32::from_rgb(92, 107, 192),
    Color32::from_rgb(141, 110, 99),
];

/// Sensors pre-selected when nothing has been chosen yet.
const DEFAULT_SELECTION: [&str; 7] = [
    "cpu/load/total",
    "cpu/clock/avg",
    "power/RAPL_Package0_PKG",
    "cpu/power/package",
    "cpu/temp/package",
    "mem/load",
    "/temp",
];

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum Mode {
    #[default]
    Session,
    /// Compare two recordings.
    Compare,
    /// CPU vs GPU bottleneck analysis of one recording.
    Bottleneck,
}

#[derive(Default)]
pub struct ViewState {
    selected: BTreeSet<String>,
    filter: String,
    initialized: bool,
    mode: Mode,
    compare: super::compare::CompareState,
    analysis: super::bottleneck::AnalysisCache,
}

/// Start or stop recording; returns a status message for the tab bar.
pub fn toggle(sampler: &Sampler) -> String {
    if sampler.snapshot().recording.is_some() {
        match sampler.stop_recording() {
            Some(p) => format!("Recording saved to {}", p.display()),
            None => "Not recording".into(),
        }
    } else {
        match sampler.start_recording(&export::report_dir(), &export::timestamp()) {
            Ok(p) => format!("Recording to {}", p.display()),
            Err(e) => e,
        }
    }
}

/// Returns true if the start/stop button was clicked (the caller toggles once the snapshot lock is released).
pub fn show(ui: &mut Ui, sampler: &Sampler, state: &mut ViewState) -> bool {
    if state.mode != Mode::Session {
        state.compare.load_files();
    }
    let snap = sampler.snapshot();
    let active = snap.recording.is_some();

    let mut clicked = false;
    ui.horizontal(|ui| {
        ui.selectable_value(&mut state.mode, Mode::Session, "Session");
        ui.selectable_value(&mut state.mode, Mode::Compare, "Compare");
        ui.selectable_value(&mut state.mode, Mode::Bottleneck, "Bottleneck");
        ui.separator();
        let label = if active { "Stop recording" } else { "Start recording" };
        let button = egui::Button::new(RichText::new(label).strong()).fill(if active {
            heat::RED.gamma_multiply(0.6)
        } else {
            ui.visuals().widgets.inactive.weak_bg_fill
        });
        clicked = ui.add(button).clicked();
        match snap.viewed_recording() {
            Some((rec, path, live)) => {
                let what = if live { "Recording" } else { "Last recording" };
                ui.label(format!(
                    "{what}: {}  ·  {} samples  ·  {} sensors",
                    format_elapsed(rec.duration_secs()),
                    rec.len(),
                    rec.columns.len()
                ));
                if ui.button("Show CSV").on_hover_text(path.display().to_string()).clicked() {
                    export::open_in_explorer(path, true);
                }
                if !live && ui.button("Open CSV").clicked() {
                    export::open_in_explorer(path, false);
                }
            }
            None => {
                ui.weak("Records every sensor at the sampling interval until you stop. Data is saved as CSV in Documents.");
            }
        }
        if let Some(e) = snap.recording.as_ref().and_then(|r| r.write_error.as_ref()) {
            ui.colored_label(crate::heat::ink(ui.visuals(), heat::ORANGE), format!("CSV write failed: {e} (still recording in memory)"));
        }
    });

    match state.mode {
        Mode::Compare => {
            ui.separator();
            super::compare::show(ui, &snap, &mut state.compare);
            return clicked;
        }
        Mode::Bottleneck => {
            ui.separator();
            let gpus = sampler.info().map_or(&[][..], |i| &i.gpus[..]);
            super::bottleneck::show(ui, &snap, &mut state.compare, &mut state.analysis, gpus);
            return clicked;
        }
        Mode::Session => {}
    }
    if let Some((rec, _, _)) = snap.viewed_recording() {
        ui.separator();
        if !state.initialized {
            for c in &rec.columns {
                if DEFAULT_SELECTION.iter().any(|d| {
                    if d.starts_with('/') {
                        c.id.ends_with(d)
                    } else {
                        c.id == *d
                    }
                }) {
                    state.selected.insert(c.id.clone());
                }
            }
            state.initialized = true;
        }
        body(ui, rec, state);
    }
    clicked
}

const PICKER_WIDTH: f32 = 260.0;

/// Sensor picker on the left, charts on the right. Plain child areas rather than nested
/// panels: panels inside a `Ui` cover the whole parent rect and would swallow the button row's clicks.
fn body(ui: &mut Ui, rec: &Recording, state: &mut ViewState) {
    let h = ui.available_height();
    ui.horizontal_top(|ui| {
        let down = egui::Layout::top_down(egui::Align::Min);
        ui.allocate_ui_with_layout(vec2(PICKER_WIDTH, h), down, |ui| {
            ui.set_width(PICKER_WIDTH);
            picker(ui, rec, state);
        });
        ui.separator();
        ui.allocate_ui_with_layout(vec2(ui.available_width(), h), down, |ui| {
            charts(ui, rec, state)
        });
    });
}

fn charts(ui: &mut Ui, rec: &Recording, state: &ViewState) {
    let cols: Vec<usize> = (0..rec.columns.len())
        .filter(|&i| state.selected.contains(&rec.columns[i].id))
        .collect();
    if cols.is_empty() {
        ui.weak("Select sensors on the left to chart them.");
        return;
    }
    if rec.len() < 2 {
        ui.weak("Collecting samples…");
        return;
    }
    // One chart per unit so values share a meaningful axis.
    let mut units: Vec<Unit> = Vec::new();
    for &c in &cols {
        if !units.contains(&rec.columns[c].unit) {
            units.push(rec.columns[c].unit);
        }
    }
    egui::ScrollArea::vertical()
        .id_salt("rec_charts")
        .auto_shrink(false)
        .show(ui, |ui| {
            for u in units {
                let group: Vec<usize> = cols
                    .iter()
                    .copied()
                    .filter(|&c| rec.columns[c].unit == u)
                    .collect();
                let title = match u.symbol() {
                    "" => "Values".to_string(),
                    s => s.to_string(),
                };
                ui.label(RichText::new(title).strong());
                chart(ui, rec, &group, &cols, u);
                stats_table(ui, rec, &group, &cols, u);
                ui.add_space(12.0);
            }
        });
}

fn picker(ui: &mut Ui, rec: &Recording, state: &mut ViewState) {
    let cols: Vec<&Column> = rec.columns.iter().collect();
    sensor_picker(
        ui,
        "rec_picker",
        &cols,
        |c| c.id.clone(),
        &mut state.selected,
        &mut state.filter,
    );
}

/// Grouped, filterable checkbox list of columns. `key` gives the id stored in `selected`.
pub(super) fn sensor_picker(
    ui: &mut Ui,
    salt: &str,
    cols: &[&Column],
    key: impl Fn(&Column) -> String,
    selected: &mut BTreeSet<String>,
    filter: &mut String,
) {
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(filter)
                .hint_text("Filter sensors")
                .desired_width(150.0),
        );
        if ui.small_button("Clear").clicked() {
            selected.clear();
        }
    });
    ui.separator();
    let filter = filter.to_lowercase();
    egui::ScrollArea::vertical()
        .id_salt(salt)
        .auto_shrink(false)
        .show(ui, |ui| {
            let mut groups: Vec<&str> = Vec::new();
            for c in cols {
                if !groups.contains(&c.group.as_str()) {
                    groups.push(&c.group);
                }
            }
            for g in groups {
                let matching: Vec<_> = cols
                    .iter()
                    .filter(|c| c.group == g)
                    .filter(|c| {
                        filter.is_empty()
                            || c.label.to_lowercase().contains(&filter)
                            || g.to_lowercase().contains(&filter)
                    })
                    .collect();
                if matching.is_empty() {
                    continue;
                }
                let any_selected = matching.iter().any(|c| selected.contains(&key(c)));
                egui::CollapsingHeader::new(g)
                    .id_salt((salt, g))
                    .default_open(any_selected || !filter.is_empty())
                    .show(ui, |ui| {
                        for c in matching {
                            let k = key(c);
                            let mut on = selected.contains(&k);
                            if ui.checkbox(&mut on, &c.label).changed() {
                                if on {
                                    selected.insert(k);
                                } else {
                                    selected.remove(&k);
                                }
                            }
                        }
                    });
            }
        });
}

/// Color for a selected column: palette entry by its position among all selected columns,
/// so series in the same chart don't share a color (until more than the palette size are picked).
fn series_color(selected: &[usize], col: usize) -> Color32 {
    let i = selected.iter().position(|&c| c == col).unwrap_or(0);
    PALETTE[i % PALETTE.len()]
}

/// A chart area: background, plot rect inside the axis margins, painter and hover response.
pub(super) struct ChartArea {
    pub plot: Rect,
    pub painter: egui::Painter,
    pub resp: egui::Response,
    pub weak: Color32,
}

pub(super) fn chart_area(ui: &mut Ui) -> ChartArea {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 180.0), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4, ui.visuals().extreme_bg_color);
    let plot = rect
        .shrink2(vec2(8.0, 10.0))
        .with_min_x(rect.left() + 64.0)
        .with_max_y(rect.bottom() - 20.0);
    ChartArea {
        plot,
        painter,
        resp,
        weak: ui.visuals().weak_text_color(),
    }
}

/// Padded y range for raw data bounds; percentages always include 0..100.
pub(super) fn y_range(lo: f64, hi: f64, unit: Unit) -> (f64, f64) {
    if unit == Unit::Percent {
        return (0.0, hi.max(100.0));
    }
    let pad = ((hi - lo) * 0.08).max(hi.abs() * 0.02).max(1e-6);
    let lo = (lo - pad).max(if lo >= 0.0 { 0.0 } else { f64::NEG_INFINITY });
    (lo, hi + pad)
}

/// Grid lines with value labels, and elapsed-time labels along the bottom.
pub(super) fn draw_axes(c: &ChartArea, lo: f64, hi: f64, dur: f64, unit: Unit) {
    let (p, plot, weak) = (&c.painter, c.plot, c.weak);
    let x_of = |t: f64| plot.left() + plot.width() * (t / dur) as f32;
    let y_of = |v: f64| plot.bottom() - plot.height() * ((v - lo) / (hi - lo)) as f32;
    for k in 0..=4 {
        let v = lo + (hi - lo) * k as f64 / 4.0;
        let y = y_of(v);
        p.line_segment(
            [pos2(plot.left(), y), pos2(plot.right(), y)],
            Stroke::new(0.5, weak.gamma_multiply(0.4)),
        );
        p.text(
            pos2(plot.left() - 6.0, y),
            Align2::RIGHT_CENTER,
            unit.format(v),
            FontId::proportional(10.0),
            weak,
        );
    }
    for k in 0..=4 {
        let t = dur * k as f64 / 4.0;
        let align = match k {
            0 => Align2::LEFT_TOP,
            4 => Align2::RIGHT_TOP,
            _ => Align2::CENTER_TOP,
        };
        p.text(
            pos2(x_of(t), plot.bottom() + 4.0),
            align,
            format_elapsed(t),
            FontId::proportional(10.0),
            weak,
        );
    }
}

/// Points for one series, downsampled to about two per pixel column (min and max) so long
/// recordings stay fast and spikes stay visible.
pub(super) fn series_points(
    times: &[f64],
    vals: &[f32],
    buckets: usize,
    x_of: impl Fn(f64) -> f32,
    y_of: impl Fn(f64) -> f32,
) -> Vec<Pos2> {
    let mut pts = Vec::with_capacity(buckets * 2);
    let n = vals.len();
    let per = n.div_ceil(buckets.max(1)).max(1);
    let mut i = 0;
    while i < n {
        let end = (i + per).min(n);
        let (mut mn, mut mx, mut tmn, mut tmx) = (f32::INFINITY, f32::NEG_INFINITY, 0.0, 0.0);
        for (&v, &t) in vals[i..end].iter().zip(&times[i..end]) {
            if v.is_finite() {
                if v < mn {
                    mn = v;
                    tmn = t;
                }
                if v > mx {
                    mx = v;
                    tmx = t;
                }
            }
        }
        if mn.is_finite() {
            let (a, b) = if tmn <= tmx {
                ((tmn, mn), (tmx, mx))
            } else {
                ((tmx, mx), (tmn, mn))
            };
            pts.push(pos2(x_of(a.0), y_of(a.1 as f64)));
            if per > 1 {
                pts.push(pos2(x_of(b.0), y_of(b.1 as f64)));
            }
        }
        i = end;
    }
    pts
}

/// Line chart of the selected columns over the whole recording, with a hover readout.
fn chart(ui: &mut Ui, rec: &Recording, cols: &[usize], selected: &[usize], unit: Unit) {
    let area = chart_area(ui);
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for &c in cols {
        if let Some(s) = rec.stats(c) {
            lo = lo.min(s.min);
            hi = hi.max(s.max);
        }
    }
    if !lo.is_finite() {
        return;
    }
    let (lo, hi) = y_range(lo, hi, unit);
    let dur = rec.duration_secs().max(1e-6);
    draw_axes(&area, lo, hi, dur, unit);
    let ChartArea {
        plot,
        painter: p,
        resp,
        weak,
    } = area;
    let x_of = |t: f64| plot.left() + plot.width() * (t / dur) as f32;
    let y_of = |v: f64| plot.bottom() - plot.height() * ((v - lo) / (hi - lo)) as f32;
    for &c in cols {
        let pts = series_points(
            &rec.times,
            &rec.values[c],
            plot.width() as usize,
            x_of,
            y_of,
        );
        if pts.len() >= 2 {
            p.line(pts, Stroke::new(1.5, series_color(selected, c)));
        }
    }

    // Hover: vertical cursor and values at the nearest sample.
    if let Some(pos) = resp.hover_pos().filter(|p| plot.x_range().contains(p.x)) {
        let t = ((pos.x - plot.left()) / plot.width()) as f64 * dur;
        let i = rec.times.partition_point(|&x| x < t).min(rec.len() - 1);
        let x = x_of(rec.times[i]);
        p.line_segment(
            [pos2(x, plot.top()), pos2(x, plot.bottom())],
            Stroke::new(1.0, weak),
        );
        resp.on_hover_ui_at_pointer(|ui| {
            ui.strong(format_elapsed(rec.times[i]));
            for &c in cols {
                let v = rec.values[c][i];
                let text = if v.is_finite() {
                    unit.format(v as f64)
                } else {
                    "-".into()
                };
                ui.colored_label(
                    crate::heat::ink(ui.visuals(), series_color(selected, c)),
                    format!("{}: {text}", rec.columns[c].label),
                );
            }
        });
    }
}

/// Per-sensor summary over the recording: where it started and ended, and its range.
fn stats_table(ui: &mut Ui, rec: &Recording, cols: &[usize], selected: &[usize], unit: Unit) {
    egui::Grid::new(("rec_stats", cols.first().copied().unwrap_or(0)))
        .striped(true)
        .num_columns(7)
        .spacing([14.0, 2.0])
        .show(ui, |ui| {
            super::header_row(
                ui,
                &["Sensor", "First", "Last", "Change", "Min", "Avg", "Max"],
            );
            for &c in cols {
                let Some(s) = rec.stats(c) else { continue };
                ui.colored_label(
                    crate::heat::ink(ui.visuals(), series_color(selected, c)),
                    format!("{} · {}", rec.columns[c].group, rec.columns[c].label),
                );
                let f = |v: f64| RichText::new(unit.format(v)).monospace();
                ui.label(f(s.first));
                ui.label(f(s.last));
                let d = s.last - s.first;
                let sign = if d > 0.0 { "+" } else { "" };
                ui.label(RichText::new(format!("{sign}{}", unit.format(d))).monospace());
                ui.label(f(s.min));
                ui.label(f(s.avg));
                ui.label(f(s.max));
                ui.end_row();
            }
        });
}
