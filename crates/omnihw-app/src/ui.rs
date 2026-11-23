//! Building blocks every page shares: text styles, group boxes, label/value sections, grids,
//! pills and hover text.
//!
//! Pages are plain functions that build an element tree from the latest snapshot; gpui calls
//! them again only when the app is notified (new data, a click), so nothing here polls or
//! animates. Sizes are in rems where it matters, so the UI scale setting (which sets the rem
//! size) scales pages as a whole.
//!
//! Text goes through [`label`] (and its styled variants), never a bare string child: a
//! [`Label`] is what screen readers and the page tests can see.

use crate::app::OmniApp;
use crate::heat;
use gpui_kit::base::{ObservedElement, TestSupportExt};
use gpui_kit::component::button::Button;
use gpui_kit::component::label::Label;
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{
    ActiveTheme, Icon, IconName, Sizable, StyledExt, WindowExt, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AnyElement, AnyView, App, ClickEvent, Context, Div, ElementId, FontWeight, Hsla,
    InteractiveElement, IntoElement, ParentElement, Rgba, SharedString, Stateful,
    StatefulInteractiveElement, Styled, Window, div, px, rems,
};
use std::hash::{Hash, Hasher};
use std::panic::Location;
use std::rc::Rc;

/// Whether the active theme is dark (status colours are darkened on light themes).
pub fn dark(cx: &App) -> bool {
    cx.theme().is_dark()
}

/// Secondary ("weak") text colour of the active theme.
pub fn weak_color(cx: &App) -> Hsla {
    cx.theme().muted_foreground
}

/// A status colour as readable text on the page background.
pub fn ink(cx: &App, c: Rgba) -> Rgba {
    heat::ink(dark(cx), c)
}

/// Plain text. Wraps when the line is too long for its container. It takes the size and
/// weight of its container but its own colour (the theme's text colour unless set with
/// `.text_color(..)`).
///
/// Each label needs an id that is unique among its siblings; it is made from the calling
/// line and the text. Where one line builds several labels with the same text (cells in a
/// loop), give their containers ids, as [`Table`] and [`key_values`] do.
#[track_caller]
pub fn label(text: impl Into<SharedString>) -> Label {
    let text: SharedString = text.into();
    let at = Location::caller();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (at.file(), at.line(), at.column(), text.as_ref()).hash(&mut hasher);
    Label::new(text).id(ElementId::NamedInteger("text".into(), hasher.finish()))
}

/// Bold text.
#[track_caller]
pub fn strong(text: impl Into<SharedString>) -> Label {
    label(text).font_weight(FontWeight::SEMIBOLD)
}

/// Secondary text in the theme's muted colour.
#[track_caller]
pub fn weak(text: impl Into<SharedString>, cx: &App) -> Label {
    label(text).text_color(weak_color(cx))
}

/// Small secondary text (hints, footnotes).
#[track_caller]
pub fn small_weak(text: impl Into<SharedString>, cx: &App) -> Label {
    weak(text, cx).text_xs()
}

/// Text in a status colour, darkened on light themes so it stays readable.
#[track_caller]
pub fn colored(text: impl Into<SharedString>, color: Rgba, cx: &App) -> Label {
    label(text).text_color(ink(cx, color))
}

/// Large bold page title ("Health Check", "Power", ...).
#[track_caller]
pub fn page_title(text: impl Into<SharedString>) -> Label {
    label(text).text_xl().font_weight(FontWeight::BOLD)
}

/// Vertical gap of `points` between two children of a column.
pub fn space(points: f32) -> Div {
    div().flex_none().h(rems(points / 16.0))
}

/// A column of children with the standard small gap; the usual root of a page.
pub fn column() -> Div {
    v_flex().gap_1()
}

/// A row of children, vertically centred, with the standard gap. Does not wrap.
pub fn row() -> Div {
    h_flex().gap_2()
}

/// A row that wraps onto more lines when the window is narrow.
pub fn wrap_row() -> Div {
    h_flex().flex_wrap().gap_2()
}

/// A full-width bordered box; add the title and the body as children.
pub fn group_box(cx: &App) -> Div {
    let t = cx.theme();
    v_flex()
        .w_full()
        .gap_1()
        .p_2()
        .mb_1p5()
        .border_1()
        .border_color(t.border)
        .rounded(t.radius)
}

/// A group box titled `title` holding a label/value list (empty values show "-").
#[track_caller]
pub fn section(title: &str, rows: &[(&str, String)], cx: &App) -> Div {
    group_box(cx)
        .child(strong(title.to_string()))
        .child(key_values(rows))
}

/// Label/value rows: labels in a fixed-width column, values bold and wrapping.
pub fn key_values(rows: &[(&str, String)]) -> Div {
    v_flex()
        .gap_0p5()
        .children(rows.iter().enumerate().map(|(i, (k, v))| {
            h_flex()
                .id(i)
                .items_start()
                .gap_4()
                .child(label(k.to_string()).flex_none().w(rems(12.5)))
                .child(strong(or_dash(v).to_string()).flex_1().min_w_0())
        }))
}

/// Text in the theme's monospace font (numbers that should line up).
#[track_caller]
pub fn mono(text: impl Into<SharedString>, cx: &App) -> Label {
    label(text).font_family(cx.theme().mono_font_family.clone())
}

/// A free-form grid whose `cols` columns are as wide as their widest cell. Add one child per
/// cell, row by row. For a plain table with headings use [`Table`].
pub fn grid(cols: u16) -> Div {
    div().grid().grid_cols_max_content(cols).gap_x_4().gap_y_1()
}

/// Wraps a [`grid`] so it keeps its natural width instead of stretching across the page.
pub fn hug(grid: impl IntoElement) -> Div {
    div().flex().child(grid)
}

/// A table with underlined headings, columns as wide as their widest cell and (optionally)
/// striped rows. It keeps its natural width.
pub struct Table {
    cols: u16,
    cells: Vec<AnyElement>,
    rows: usize,
    striped: bool,
}

impl Table {
    pub fn new(headings: &[&str]) -> Self {
        Table {
            cols: headings.len() as u16,
            cells: headings
                .iter()
                .map(|h| label(h.to_string()).underline().into_any_element())
                .collect(),
            rows: 0,
            striped: false,
        }
    }

    /// Tint every other row.
    pub fn striped(mut self) -> Self {
        self.striped = true;
        self
    }

    /// Add a row; give one element per column.
    pub fn row(mut self, cells: impl IntoIterator<Item = AnyElement>) -> Self {
        self.push_row(cells);
        self
    }

    /// Add a row of plain text cells.
    pub fn text_row<S: Into<SharedString>>(self, cells: impl IntoIterator<Item = S>) -> Self {
        self.row(cells.into_iter().map(|c| label(c).into_any_element()))
    }

    /// [`Table::row`] for use in a loop.
    pub fn push_row(&mut self, cells: impl IntoIterator<Item = AnyElement>) {
        let before = self.cells.len();
        self.cells.extend(cells);
        debug_assert_eq!(self.cells.len() - before, self.cols as usize);
        self.rows += 1;
    }

    pub fn is_empty(&self) -> bool {
        self.rows == 0
    }

    pub fn build(self, cx: &App) -> Div {
        let stripe = cx.theme().muted;
        let cols = self.cols as usize;
        let striped = self.striped;
        let cells = self.cells.into_iter().enumerate().map(move |(i, cell)| {
            // Row 0 is the headings; tint data rows 2, 4, ...
            let row = i / cols;
            div()
                .id(i)
                .px_2()
                .py_0p5()
                .when(striped && row > 0 && row.is_multiple_of(2), |c| {
                    c.bg(stripe)
                })
                .child(cell)
        });
        hug(div()
            .grid()
            .grid_cols_max_content(self.cols)
            .children(cells))
    }
}

/// A drop-down list: a button showing `current` that opens a menu of `options`. `pick` runs
/// with the chosen index (the page is redrawn afterwards). `id` must be unique on the page.
pub fn dropdown(
    id: impl Into<ElementId>,
    current: impl Into<SharedString>,
    options: Vec<SharedString>,
    selected: Option<usize>,
    cx: &mut Context<OmniApp>,
    pick: impl Fn(&mut OmniApp, usize, &mut Window, &mut Context<OmniApp>) + 'static,
) -> AnyElement {
    let app = cx.weak_entity();
    let pick = Rc::new(pick);
    Button::new(id)
        .small()
        .label(current)
        .dropdown_caret(true)
        .dropdown_menu(move |mut menu, _, _| {
            for (i, option) in options.iter().enumerate() {
                let (app, pick) = (app.clone(), pick.clone());
                menu = menu.item(
                    PopupMenuItem::new(option.clone())
                        .checked(selected == Some(i))
                        .on_click(move |_: &ClickEvent, window, cx| {
                            let _ = app.update(cx, |this, cx| {
                                pick(this, i, window, cx);
                                cx.notify();
                            });
                        }),
                );
            }
            menu
        })
        .into_any_element()
}

/// Ask before a disruptive change: a dialog with `title`, `text`, an `ok` button and Cancel.
/// `apply` runs when `ok` is chosen.
pub fn confirm(
    title: impl Into<SharedString>,
    text: impl Into<SharedString>,
    ok: &'static str,
    window: &mut Window,
    cx: &mut Context<OmniApp>,
    apply: impl Fn(&mut OmniApp, &mut Window, &mut Context<OmniApp>) + 'static,
) {
    let app = cx.weak_entity();
    let (title, text): (SharedString, SharedString) = (title.into(), text.into());
    let apply = Rc::new(apply);
    window.open_alert_dialog(cx, move |alert, _, _| {
        let (app, apply) = (app.clone(), apply.clone());
        alert
            .title(label(title.clone()))
            .description(label(text.clone()))
            .show_cancel(true)
            .ok_text(ok)
            .on_ok(move |_, window, cx| {
                let _ = app.update(cx, |this, cx| {
                    apply(this, window, cx);
                    cx.notify();
                });
                true
            })
    });
}

/// Whether the collapsible section `key` is open. Sections remember being toggled for as long
/// as the app runs.
pub fn is_open(app: &OmniApp, key: &str, default_open: bool) -> bool {
    app.views.toggled.contains(key) != default_open
}

/// The clickable heading of a collapsible section: a chevron and `title`. Show the body below
/// it when [`is_open`] says so (build the body only then). `key` must be unique in the app.
pub fn collapse_header(
    key: impl Into<SharedString>,
    title: impl IntoElement,
    open: bool,
    cx: &mut Context<OmniApp>,
) -> Clickable {
    let key: SharedString = key.into();
    let icon = if open {
        IconName::ChevronDown
    } else {
        IconName::ChevronRight
    };
    clickable(ElementId::Name(key.clone()))
        .h_flex()
        .gap_1()
        .py_0p5()
        .cursor_pointer()
        .child(Icon::new(icon).small().text_color(weak_color(cx)))
        .child(title)
        .on_click(cx.listener(move |this, _, _, cx| {
            if !this.views.toggled.remove(&key) {
                this.views.toggled.insert(key.clone());
            }
            cx.notify();
        }))
}

/// Small rounded tag with a tinted fill, used for counts, levels and markers.
#[track_caller]
pub fn pill(text: impl Into<SharedString>, color: Rgba, small: bool, cx: &App) -> Div {
    let tag = h_flex()
        .flex_none()
        .px(px(7.0))
        .py(px(1.0))
        .rounded(px(10.0))
        .bg(heat::fade(color, 0.16))
        .font_weight(FontWeight::SEMIBOLD)
        .child(label(text).text_color(ink(cx, color)));
    if small { tag.text_xs() } else { tag }
}

/// A container with an id: the base of anything clickable, hoverable or scrollable. Page
/// tests can find it by that id (`window.click(id, cx)`), so prefer it to a bare
/// `div().id(..)`.
pub type Clickable = ObservedElement<Stateful<Div>>;

/// An empty [`Clickable`]; style it and add `.on_click(..)`, `.tooltip(..)`, children.
pub fn clickable(id: impl Into<ElementId>) -> Clickable {
    div().id(id).test_support()
}

/// Builds the hover text for `.tooltip(...)` on an element that has an id.
pub fn tip(text: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let text: SharedString = text.into();
    move |window, cx| Tooltip::new(text.clone()).build(window, cx)
}

/// `element` with hover text; `id` must be unique among its siblings.
pub fn with_tip(
    id: impl Into<gpui_kit::ElementId>,
    element: impl IntoElement,
    text: impl Into<SharedString>,
) -> Clickable {
    clickable(id).child(element).tooltip(tip(text))
}

/// `s`, or "-" when it's empty.
pub fn or_dash(s: &str) -> &str {
    if s.is_empty() { "-" } else { s }
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

/// Warning under a memory or disk test when WHEA hardware errors were logged during it: red if
/// any was fatal, else orange. `None` when there were none.
#[track_caller]
pub fn whea_warning(
    id: impl Into<gpui_kit::ElementId>,
    events: &[omnihw_core::whea::WheaEvent],
    cx: &App,
) -> Option<AnyElement> {
    let text = omnihw_core::whea::during_test_text(events)?;
    let color = if events.iter().any(|e| e.fatal) {
        heat::RED
    } else {
        heat::ORANGE
    };
    let hover = events
        .iter()
        .rev()
        .take(10)
        .map(|e| e.summary.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    Some(with_tip(id, colored(text, color, cx), hover).into_any_element())
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
