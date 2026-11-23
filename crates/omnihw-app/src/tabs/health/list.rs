//! The category tiles above the panes and the list pane: findings grouped by what to do about
//! them, then each category's passed checks and the dismissed findings.

use super::{
    GRAY, GREEN, Item, Report, Sel, View, category_status, findings_of, level_color, select,
};
use crate::app::OmniApp;
use crate::ui::{self, plural};
use gpui_kit::component::separator::Separator;
use gpui_kit::component::{ActiveTheme, StyledExt};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AnyElement, App, Context, Div, ElementId, Hsla, InteractiveElement, IntoElement, ParentElement,
    Rgba, StatefulInteractiveElement, Styled, div, px, rems,
};
use omnihw_core::diagnostics::{Category, Level};

/// One tile per category with its status, plus "All". Clicking one filters the list to its
/// category (clicking the active tile clears the filter).
pub(super) fn tiles(report: &Report, filter: Option<Category>, cx: &mut Context<OmniApp>) -> Div {
    let total = report
        .results
        .iter()
        .flat_map(findings_of)
        .map(|f| f.level)
        .collect::<Vec<Level>>();
    let all_status = match total.iter().max() {
        Some(&worst) => (plural(total.len(), "finding"), level_color(worst)),
        None => ("OK".to_string(), GREEN),
    };
    let mut row = ui::wrap_row().gap_1p5().child(tile(
        0,
        "All",
        &all_status,
        filter.is_none(),
        "Show every category".to_string(),
        None,
        cx,
    ));
    for (i, cat) in Category::ALL.into_iter().enumerate() {
        let Some(status) = category_status(&report.results, cat) else {
            continue;
        };
        let active = filter == Some(cat);
        let hover = if active {
            "Click again to show every category".to_string()
        } else {
            format!("Show only {}", cat.name())
        };
        row = row.child(tile(
            i + 1,
            cat.name(),
            &status,
            active,
            hover,
            (!active).then_some(cat),
            cx,
        ));
    }
    row
}

/// A small rounded tile: the name above a coloured status. Outlined when active. A click
/// sets the filter to `pick`.
fn tile(
    index: usize,
    name: &'static str,
    (status, color): &(String, Rgba),
    active: bool,
    hover: String,
    pick: Option<Category>,
    cx: &mut Context<OmniApp>,
) -> ui::Clickable {
    let t = cx.theme();
    let (fill, accent, hover_border, radius) = (t.muted, t.primary, t.border, t.radius);
    let ink = ui::ink(cx, *color);
    ui::clickable(("health-tile", index))
        .v_flex()
        .px(px(10.0))
        .py(px(4.0))
        .rounded(radius)
        .border_1()
        .cursor_pointer()
        .map(|tile| {
            if active {
                tile.bg(accent.opacity(0.25)).border_color(accent)
            } else {
                tile.bg(fill)
                    .border_color(gpui_kit::transparent_black())
                    .hover(move |s| s.border_color(hover_border))
            }
        })
        .child(ui::strong(name))
        .child(
            ui::label(status.clone())
                .text_size(crate::ui::small_size())
                .text_color(ink),
        )
        .tooltip(ui::tip(hover))
        .on_click(cx.listener(move |this, _, _, cx| {
            this.views.health.filter = pick;
            cx.notify();
        }))
}

/// The id of a list entry's row, so it can be clicked by id.
pub(super) fn row_id(sel: &Sel) -> ElementId {
    match sel {
        Sel::Finding(k) => ElementId::Name(format!("health-item/{k}").into()),
        Sel::Passed(c) => ElementId::Name(format!("health-passed/{}", c.name()).into()),
        Sel::Dismissed | Sel::DismissedFinding(_) => "health-dismissed".into(),
    }
}

/// The list pane's rows, and which of them is the selected entry (to scroll it into view
/// after an arrow key moved it). Clicking an entry selects it.
pub(super) fn list(
    view: &View,
    sel: Option<&Sel>,
    cx: &mut Context<OmniApp>,
) -> (Vec<AnyElement>, Option<usize>) {
    let mut out = Vec::new();
    let mut at = None;
    if view.fix.is_empty() && view.suggestions.is_empty() {
        let text = match view.filter {
            Some(c) => format!("Nothing to fix in {}", c.name()),
            None => "Everything looks good".to_string(),
        };
        out.push(
            ui::strong(text)
                .text_size(rems(15.0 / 16.0))
                .text_color(ui::ink(cx, GREEN))
                .px(px(6.0))
                .py_1()
                .into_any_element(),
        );
    }
    findings_group(&mut out, &mut at, "Fix these", &view.fix, sel, None, cx);
    findings_group(
        &mut out,
        &mut at,
        "Suggestions",
        &view.suggestions,
        sel,
        None,
        cx,
    );
    findings_group(
        &mut out,
        &mut at,
        "For gaming",
        &view.gaming,
        sel,
        Some("Settings that mainly affect games: frame rates, latency and GPU features."),
        cx,
    );
    if !view.passed.is_empty() {
        out.push(heading("Passed checks".to_string(), None, cx));
        for p in &view.passed {
            let s = Sel::Passed(p.cat);
            let meta = match (p.passed, p.skipped) {
                (0, _) => "not checked".to_string(),
                (n, 0) => format!("{n} passed"),
                (n, s) => format!("{n} passed · {s} n/a"),
            };
            let dot = if p.passed == 0 { GRAY } else { GREEN };
            let selected = sel == Some(&s);
            if selected {
                at = Some(out.len());
            }
            let r = row(row_id(&s), selected, dot, p.cat.name(), &meta, None, cx);
            out.push(r.on_click(select(s, cx)).into_any_element());
        }
    }
    if !view.dismissed.is_empty() {
        out.push(ui::space(6.0).into_any_element());
        out.push(Separator::horizontal().into_any_element());
        let selected = matches!(sel, Some(Sel::Dismissed | Sel::DismissedFinding(_)));
        if selected {
            at = Some(out.len());
        }
        let title = format!("Dismissed ({})", view.dismissed.len());
        let r = row(
            row_id(&Sel::Dismissed),
            selected,
            GRAY,
            &title,
            "",
            Some("Findings you dismissed: read them again or restore them"),
            cx,
        );
        out.push(r.on_click(select(Sel::Dismissed, cx)).into_any_element());
    }
    out.push(ui::space(4.0).into_any_element());
    (out, at)
}

/// A heading and one row per finding; nothing when there are none. Notes in `at` which row is
/// the selected one.
fn findings_group(
    out: &mut Vec<AnyElement>,
    at: &mut Option<usize>,
    title: &str,
    items: &[Item],
    sel: Option<&Sel>,
    note: Option<&'static str>,
    cx: &mut Context<OmniApp>,
) {
    if items.is_empty() {
        return;
    }
    out.push(heading(format!("{title} · {}", items.len()), note, cx));
    for i in items {
        let s = Sel::Finding(i.key.clone());
        let selected = sel == Some(&s);
        if selected {
            *at = Some(out.len());
        }
        let r = row(
            row_id(&s),
            selected,
            level_color(i.f.level),
            &i.f.title,
            i.f.category.name(),
            Some(&i.f.title),
            cx,
        );
        out.push(r.on_click(select(s, cx)).into_any_element());
    }
}

/// Small caps-style group heading, with hover text when given.
fn heading(text: String, hover: Option<&'static str>, cx: &App) -> AnyElement {
    let label = ui::label(text.to_uppercase())
        .text_size(crate::ui::small_size())
        .text_color(crate::theme::palette(cx).strong);
    let box_ = h_box().pt_2().pb_0p5().px(px(6.0));
    match hover {
        Some(h) => box_
            .child(ui::with_tip(ElementId::Name(text.into()), label, h))
            .into_any_element(),
        None => box_.child(label).into_any_element(),
    }
}

/// A row that keeps its children at their natural width.
fn h_box() -> Div {
    div().flex().flex_none()
}

/// A selectable full-width row: a coloured dot, the title (cut short with … when it doesn't
/// fit) and muted text on the right. `tip` is its hover text (the full title for findings,
/// which may be cut short).
pub(super) fn row(
    id: impl Into<ElementId>,
    selected: bool,
    dot: Rgba,
    title: &str,
    meta: &str,
    tip: Option<&str>,
    cx: &App,
) -> ui::Clickable {
    let (t, p) = (cx.theme(), crate::theme::palette(cx));
    let (selected_fill, hover_fill, radius) = (p.selection, t.accent, t.radius);
    let (text, muted): (Hsla, Hsla) = if selected {
        (p.selection_text.into(), p.selection_text.into())
    } else {
        (t.foreground, t.muted_foreground)
    };
    ui::clickable(id)
        .h_flex()
        .flex_none()
        .w_full()
        .h(rems(24.0 / 16.0))
        .gap_2()
        .px_2()
        .rounded(radius)
        .cursor_pointer()
        .map(|row| {
            if selected {
                row.bg(selected_fill)
            } else {
                row.hover(move |s| s.bg(hover_fill))
            }
        })
        .child(div().flex_none().size(px(8.0)).rounded_full().bg(dot))
        .child(
            ui::label(title.to_string())
                .flex_1()
                .min_w_0()
                .truncate()
                .text_color(text),
        )
        .when(!meta.is_empty(), |row| {
            row.child(
                ui::label(meta.to_string())
                    .flex_none()
                    .text_size(crate::ui::small_size())
                    .text_color(muted),
            )
        })
        .when_some(tip, |row, tip| row.tooltip(ui::tip(tip.to_string())))
}
