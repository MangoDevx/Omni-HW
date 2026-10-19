//! The category tiles above the panes and the list pane: findings grouped by what to do about
//! them, then each category's passed checks and the dismissed findings.

use super::{GREEN, Item, Report, Sel, View, category_status, level_color, plural, select};
use egui::{Color32, RichText, Sense, TextWrapMode, Ui, WidgetText, vec2};
use omnihw_core::diagnostics::{Category, Level};

/// One tile per category with its status, plus "All". Returns the new filter when one is
/// clicked (clicking the active tile clears it).
pub(super) fn tiles(
    ui: &mut Ui,
    report: &Report,
    filter: Option<Category>,
) -> Option<Option<Category>> {
    let mut clicked = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
        let total = report
            .results
            .iter()
            .flat_map(super::findings_of)
            .map(|f| f.level)
            .collect::<Vec<Level>>();
        let all_status = match total.iter().max() {
            Some(&worst) => (plural(total.len(), "finding"), level_color(worst)),
            None => ("OK".to_string(), GREEN),
        };
        if tile(ui, "All", &all_status, filter.is_none())
            .on_hover_text("Show every category")
            .clicked()
        {
            clicked = Some(None);
        }
        for cat in Category::ALL {
            let Some(status) = category_status(&report.results, cat) else {
                continue;
            };
            let active = filter == Some(cat);
            let hover = if active {
                "Click again to show every category".to_string()
            } else {
                format!("Show only {}", cat.name())
            };
            if tile(ui, cat.name(), &status, active)
                .on_hover_text(hover)
                .clicked()
            {
                clicked = Some((!active).then_some(cat));
            }
        }
    });
    clicked
}

/// A small rounded tile: the name above a coloured status. Outlined when active.
fn tile(
    ui: &mut Ui,
    name: &str,
    (status, color): &(String, Color32),
    active: bool,
) -> egui::Response {
    ui.push_id(("health_tile", name), |ui| {
        let v = ui.visuals();
        let stroke_color = v.selection.stroke.color;
        let hover_color = v.widgets.hovered.bg_stroke.color;
        let ink = crate::heat::ink(v, *color);
        let mut frame = egui::Frame::new()
            .fill(v.faint_bg_color)
            .corner_radius(6)
            .inner_margin(egui::Margin::symmetric(10, 4));
        if active {
            frame = frame
                .fill(v.selection.bg_fill.gamma_multiply(0.35))
                .stroke(egui::Stroke::new(1.0, stroke_color));
        }
        let resp = frame
            .show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.add(egui::Label::new(RichText::new(name).strong()).selectable(false));
                    ui.add(
                        egui::Label::new(RichText::new(status).small().color(ink))
                            .selectable(false),
                    );
                });
            })
            .response
            .interact(Sense::click());
        if resp.hovered() && !active {
            ui.painter().rect_stroke(
                resp.rect,
                6,
                egui::Stroke::new(1.0, hover_color),
                egui::StrokeKind::Inside,
            );
        }
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    })
    .inner
}

/// The list pane. Clicking an entry selects it; with `scroll` the selected entry is scrolled
/// into view (after an arrow key moved it).
pub(super) fn list(ui: &mut Ui, view: &View, sel: Option<&Sel>, scroll: bool) {
    ui.spacing_mut().item_spacing.y = 1.0;
    if view.fix.is_empty() && view.suggestions.is_empty() {
        let text = match view.filter {
            Some(c) => format!("Nothing to fix in {}", c.name()),
            None => "Everything looks good".to_string(),
        };
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            ui.label(
                RichText::new(text)
                    .size(15.0)
                    .strong()
                    .color(crate::heat::ink(ui.visuals(), GREEN)),
            );
        });
        ui.add_space(4.0);
    }
    findings_group(ui, "Fix these", &view.fix, sel, scroll, None);
    findings_group(ui, "Suggestions", &view.suggestions, sel, scroll, None);
    findings_group(
        ui,
        "For gaming",
        &view.gaming,
        sel,
        scroll,
        Some("Settings that mainly affect games: frame rates, latency and GPU features."),
    );
    if !view.passed.is_empty() {
        heading(ui, "Passed checks", None);
        for p in &view.passed {
            let s = Sel::Passed(p.cat);
            let meta = match (p.passed, p.skipped) {
                (0, _) => "not checked".to_string(),
                (n, 0) => format!("{n} passed"),
                (n, s) => format!("{n} passed · {s} n/a"),
            };
            let dot = if p.passed == 0 { Color32::GRAY } else { GREEN };
            let r = ui.push_id(("health_passed", p.cat.name()), |ui| {
                row(ui, sel == Some(&s), dot, p.cat.name(), &meta)
            });
            reveal(&r.inner, scroll && sel == Some(&s));
            if r.inner.clicked() {
                select(ui.ctx(), s);
            }
        }
    }
    if !view.dismissed.is_empty() {
        ui.add_space(6.0);
        ui.separator();
        let selected = matches!(sel, Some(Sel::Dismissed | Sel::DismissedFinding(_)));
        let title = format!("Dismissed ({})", view.dismissed.len());
        let r = ui.push_id("health_dismissed_entry", |ui| {
            row(ui, selected, Color32::GRAY, &title, "")
        });
        reveal(&r.inner, scroll && selected);
        if r.inner
            .on_hover_text("Findings you dismissed: read them again or restore them")
            .clicked()
        {
            select(ui.ctx(), Sel::Dismissed);
        }
    }
    ui.add_space(4.0);
}

/// A heading and one row per finding; nothing when there are none.
fn findings_group(
    ui: &mut Ui,
    title: &str,
    items: &[Item],
    sel: Option<&Sel>,
    scroll: bool,
    note: Option<&str>,
) {
    if items.is_empty() {
        return;
    }
    heading(ui, &format!("{title} · {}", items.len()), note);
    for i in items {
        let s = Sel::Finding(i.key.clone());
        let r = ui.push_id(("health_item", &i.key), |ui| {
            row(
                ui,
                sel == Some(&s),
                level_color(i.f.level),
                &i.f.title,
                i.f.category.name(),
            )
        });
        reveal(&r.inner, scroll && sel == Some(&s));
        if r.inner.clicked() {
            select(ui.ctx(), s);
        }
    }
}

/// Scrolls `row` into view when `on`.
fn reveal(row: &egui::Response, on: bool) {
    if on {
        row.scroll_to_me(None);
    }
}

/// Small caps-style group heading.
fn heading(ui: &mut Ui, text: &str, hover: Option<&str>) {
    ui.add_space(8.0);
    let r = ui.horizontal(|ui| {
        ui.add_space(6.0);
        ui.label(RichText::new(text.to_uppercase()).small().strong().weak())
    });
    if let Some(h) = hover {
        r.inner.on_hover_text(h);
    }
    ui.add_space(2.0);
}

/// A selectable full-width row: a coloured dot, the title (cut short with … when it doesn't
/// fit, in full on hover) and muted text on the right.
pub(super) fn row(
    ui: &mut Ui,
    selected: bool,
    dot: Color32,
    title: &str,
    meta: &str,
) -> egui::Response {
    let height = 26.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
    resp.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, title)
    });
    let v = ui.visuals();
    let (fill, text, muted) = if selected {
        let c = v.selection.stroke.color;
        (Some(v.selection.bg_fill), c, c.gamma_multiply(0.75))
    } else {
        let fill = resp.hovered().then_some(v.widgets.hovered.weak_bg_fill);
        (fill, v.text_color(), v.weak_text_color())
    };
    let mut elided = false;
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        if let Some(fill) = fill {
            painter.rect_filled(rect, 4, fill);
        }
        let pad = 8.0;
        painter.circle_filled(
            egui::pos2(rect.left() + pad + 4.0, rect.center().y),
            4.0,
            dot,
        );
        let meta = WidgetText::from(RichText::new(meta).small().color(muted)).into_galley(
            ui,
            Some(TextWrapMode::Extend),
            f32::INFINITY,
            egui::TextStyle::Body,
        );
        let left = rect.left() + pad + 16.0;
        let title_w = (rect.right() - pad - meta.size().x - 10.0 - left).max(20.0);
        let title = WidgetText::from(RichText::new(title).color(text)).into_galley(
            ui,
            Some(TextWrapMode::Truncate),
            title_w,
            egui::TextStyle::Body,
        );
        elided = title.elided;
        let y = |g: &egui::Galley| rect.center().y - g.size().y / 2.0;
        painter.galley(egui::pos2(left, y(&title)), title, text);
        painter.galley(
            egui::pos2(rect.right() - pad - meta.size().x, y(&meta)),
            meta,
            muted,
        );
    }
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    if elided {
        resp.on_hover_text(title)
    } else {
        resp
    }
}
