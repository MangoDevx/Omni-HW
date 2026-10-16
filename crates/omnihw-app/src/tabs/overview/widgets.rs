//! The Overview's building blocks: cards, big-number rows, sparklines, bars and the
//! severity icon.

use crate::heat::{ORANGE, RED, Severity};
use egui::{Align2, Color32, FontId, RichText, Sense, Stroke, Ui, vec2};
use std::collections::VecDeque;

/// A framed card with an accent marker, title and subtitle.
pub(super) fn card(
    ui: &mut Ui,
    accent: Color32,
    title: &str,
    subtitle: &str,
    add: impl FnOnce(&mut Ui),
) {
    egui::Frame::group(ui.style())
        .inner_margin(12)
        .corner_radius(8)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            // Click the header (or its arrow) to collapse/expand; egui remembers the state.
            let id = egui::Id::new(("overview_card", title));
            egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true)
                .show_header(ui, |ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(4.0, 18.0), Sense::hover());
                    ui.painter().rect_filled(r, 2, accent);
                    ui.label(RichText::new(title).size(16.0).strong());
                })
                .body(|ui| {
                    if !subtitle.is_empty() {
                        ui.add(egui::Label::new(RichText::new(subtitle).weak()).wrap());
                    }
                    ui.add_space(6.0);
                    add(ui);
                });
        });
    ui.add_space(8.0);
}

/// One big number with a small caption; `color` overrides the value's text color.
pub(super) struct Metric<'a> {
    pub(super) caption: &'a str,
    pub(super) value: String,
    pub(super) color: Option<Color32>,
}

pub(super) fn m(caption: &str, value: String) -> Metric<'_> {
    Metric {
        caption,
        value,
        color: None,
    }
}

/// A row of big numbers with small captions.
pub(super) fn metrics(ui: &mut Ui, items: &[Metric]) {
    let n = items.len().max(1) as f32;
    let w = (ui.available_width() - ui.spacing().item_spacing.x * (n - 1.0)) / n;
    ui.horizontal(|ui| {
        for it in items {
            ui.allocate_ui_with_layout(
                vec2(w, 44.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_width(w);
                    ui.label(RichText::new(it.caption).small().weak());
                    let mut text = RichText::new(&it.value).size(20.0).strong();
                    if let Some(c) = it.color {
                        text = text.color(crate::heat::ink(ui.visuals(), c));
                    }
                    ui.label(text);
                },
            );
        }
    });
}

/// Warning triangle or critical circle with "!", drawn so it doesn't depend on font glyphs.
pub(super) fn severity_icon(p: &egui::Painter, center: egui::Pos2, sev: Severity) {
    let r = 6.0;
    match sev {
        Severity::Normal => return,
        Severity::Warning => {
            let pts = vec![
                center + vec2(0.0, -r),
                center + vec2(r * 1.05, r * 0.8),
                center + vec2(-r * 1.05, r * 0.8),
            ];
            p.add(egui::Shape::convex_polygon(pts, ORANGE, Stroke::NONE));
        }
        Severity::Critical => {
            p.circle_filled(center, r, RED);
            p.circle_stroke(center, r, Stroke::new(1.2, Color32::WHITE));
        }
    }
    let fg = if sev == Severity::Warning {
        Color32::BLACK
    } else {
        Color32::WHITE
    };
    p.text(
        center + vec2(0.0, 0.8),
        Align2::CENTER_CENTER,
        "!",
        FontId::proportional(10.0),
        fg,
    );
}

/// Line chart of recent values scaled to `0..=max` (or to the data's max when `max` is None).
pub(super) fn sparkline(
    ui: &mut Ui,
    caption: &str,
    data: Option<&VecDeque<f32>>,
    max: Option<f32>,
    color: Color32,
) {
    sparkline_colored(ui, caption, data, max, |_| color);
}

/// Like [`sparkline`], but each segment is colored by `color_of` its higher endpoint,
/// so a heatmap scale shows where in the history a value got hot.
pub(super) fn sparkline_colored(
    ui: &mut Ui,
    caption: &str,
    data: Option<&VecDeque<f32>>,
    max: Option<f32>,
    color_of: impl Fn(f32) -> Color32,
) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 46.0), Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 4, ui.visuals().extreme_bg_color);
    p.text(
        rect.left_top() + vec2(6.0, 4.0),
        Align2::LEFT_TOP,
        caption,
        FontId::proportional(10.0),
        ui.visuals().weak_text_color(),
    );
    let Some(data) = data.filter(|d| d.len() >= 2) else {
        return;
    };
    let top = max
        .unwrap_or_else(|| data.iter().copied().fold(1.0, f32::max))
        .max(1e-3);
    let cap = omnihw_core::sensors::HISTORY_LEN as f32 - 1.0;
    let start = cap - (data.len() as f32 - 1.0);
    let inner = rect.shrink2(vec2(2.0, 4.0));
    let pts: Vec<_> = data
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let x = inner.left() + inner.width() * (start + i as f32) / cap;
            let y = inner.bottom() - inner.height() * (v / top).clamp(0.0, 1.0);
            egui::pos2(x, y)
        })
        .collect();
    // Merge consecutive segments of the same color into one polyline (one shape for flat colors).
    let mut run: Vec<egui::Pos2> = vec![pts[0]];
    let mut run_color = None;
    for (i, w) in pts.windows(2).enumerate() {
        let c = color_of(data[i].max(data[i + 1]));
        if let Some(rc) = run_color.filter(|&rc| rc != c) {
            p.line(
                std::mem::replace(&mut run, vec![w[0]]),
                Stroke::new(1.5, rc),
            );
        }
        run_color = Some(c);
        run.push(w[1]);
    }
    if let Some(c) = run_color {
        p.line(run, Stroke::new(1.5, c));
    }
}

pub(super) fn bar(ui: &mut Ui, frac: f32, text: String, color: Color32) {
    // egui draws bar text in the selection color, which is unreadable on saturated fills in
    // light mode; use the normal text color on a theme-aware fill instead.
    let v = ui.visuals();
    let (fill, ink) = (crate::heat::soft_fill(v, color), v.strong_text_color());
    ui.add(
        egui::ProgressBar::new(frac.clamp(0.0, 1.0))
            .text(RichText::new(text).color(ink))
            .fill(fill)
            .desired_height(18.0),
    );
}
