//! The Overview's building blocks: cards, big-number rows, sparklines, bars and the
//! severity icon.

use crate::app::OmniApp;
use crate::heat::{self, BLACK, ORANGE, RED, Severity, WHITE};
use crate::ui;
use gpui_kit::component::{ActiveTheme, h_flex, v_flex};
use gpui_kit::{
    App, Context, Div, ElementId, InteractiveElement, ParentElement, PathBuilder, Rgba, Stateful,
    Styled, Window, canvas, div, point, px, relative, rems,
};
use std::collections::VecDeque;

/// The theme colour behind sparklines and bars.
pub(super) fn track(cx: &App) -> Rgba {
    crate::theme::palette(cx).extreme
}

/// A framed card with an accent marker, title and subtitle. Click the header (or its arrow)
/// to collapse/expand; the state is remembered for as long as the app runs. `body` is only
/// built while the card is open.
pub(super) fn card(
    id: impl Into<ElementId>,
    accent: Rgba,
    title: &str,
    subtitle: &str,
    is_open: &dyn Fn(&str) -> bool,
    cx: &mut Context<OmniApp>,
    body: impl FnOnce(&mut Context<OmniApp>) -> Div,
) -> Stateful<Div> {
    let key = format!("overview_card/{title}");
    let open = is_open(&key);
    let heading = h_flex()
        .gap_2()
        .child(
            div()
                .flex_none()
                .w(px(4.0))
                .h(rems(18.0 / 16.0))
                .rounded(px(2.0))
                .bg(accent),
        )
        .child(ui::strong(title.to_string()).text_base());
    let border = cx.theme().border;
    let mut card = v_flex()
        .id(id)
        .w_full()
        .p(px(12.0))
        .mb(px(8.0))
        .gap(px(3.0))
        .border_1()
        .border_color(border)
        .rounded(px(8.0))
        .child(ui::collapse_header(key, heading, open, cx));
    if open {
        let mut inner = v_flex().w_full().gap(px(3.0));
        if !subtitle.is_empty() {
            inner = inner.child(ui::weak(subtitle.to_string(), cx));
        }
        inner = inner.child(ui::space(3.0)).child(body(cx));
        card = card.child(ui::indented(inner, cx));
    }
    card
}

/// One big number with a small caption; `color` overrides the value's text color.
pub(super) struct Metric<'a> {
    pub(super) caption: &'a str,
    pub(super) value: String,
    pub(super) color: Option<Rgba>,
}

pub(super) fn m(caption: &str, value: String) -> Metric<'_> {
    Metric {
        caption,
        value,
        color: None,
    }
}

/// A row of big numbers with small captions, sharing the width equally.
pub(super) fn metrics(items: &[Metric], cx: &App) -> Div {
    h_flex()
        .w_full()
        .items_start()
        .gap(px(8.0))
        .children(items.iter().enumerate().map(|(i, it)| {
            let mut value = ui::strong(it.value.clone()).text_size(rems(20.0 / 16.0));
            if let Some(c) = it.color {
                value = value.text_color(ui::ink(cx, c));
            }
            v_flex()
                .id(i)
                .flex_1()
                .min_w_0()
                .min_h(rems(44.0 / 16.0))
                .child(ui::small_weak(it.caption.to_string(), cx))
                .child(value)
        }))
}

/// Warning triangle or critical circle with "!", drawn so it doesn't depend on font glyphs.
/// None for normal values.
pub(super) fn severity_icon(sev: Severity) -> Option<Div> {
    let mark = |fg: Rgba| {
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .text_size(rems(10.0 / 16.0))
            .child(ui::label("!").text_color(fg))
    };
    let size = rems(12.0 / 16.0);
    match sev {
        Severity::Normal => None,
        Severity::Warning => Some(
            div()
                .relative()
                .flex_none()
                .size(size)
                .child(
                    canvas(
                        |_, _, _| (),
                        |b, _, window, _| {
                            let c = b.center();
                            let r = b.size.width / 2.0;
                            let mut path = PathBuilder::fill();
                            path.add_polygon(
                                &[
                                    point(c.x, c.y - r),
                                    point(c.x + r * 1.05, c.y + r * 0.8),
                                    point(c.x - r * 1.05, c.y + r * 0.8),
                                ],
                                true,
                            );
                            if let Ok(path) = path.build() {
                                window.paint_path(path, ORANGE);
                            }
                        },
                    )
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
                )
                .child(mark(BLACK)),
        ),
        Severity::Critical => Some(
            div()
                .relative()
                .flex_none()
                .size(size)
                .rounded_full()
                .bg(RED)
                .border_1()
                .border_color(WHITE)
                .child(mark(WHITE)),
        ),
    }
}

/// Line chart of recent values scaled to `0..=max` (or to the data's max when `max` is None).
pub(super) fn sparkline(
    caption: &str,
    data: Option<&VecDeque<f32>>,
    max: Option<f32>,
    color: Rgba,
    cx: &App,
) -> Div {
    sparkline_colored(caption, data, max, |_| color, cx)
}

/// Like [`sparkline`], but each segment is colored by `color_of` its higher endpoint,
/// so a heatmap scale shows where in the history a value got hot.
pub(super) fn sparkline_colored(
    caption: &str,
    data: Option<&VecDeque<f32>>,
    max: Option<f32>,
    color_of: impl Fn(f32) -> Rgba,
    cx: &App,
) -> Div {
    let chart = div()
        .relative()
        .flex_none()
        .w_full()
        .h(rems(46.0 / 16.0))
        .rounded(px(4.0))
        .bg(track(cx))
        .child(
            div()
                .absolute()
                .top(px(4.0))
                .left(px(6.0))
                .text_size(rems(10.0 / 16.0))
                .child(ui::weak(caption.to_string(), cx)),
        );
    let Some(data) = data.filter(|d| d.len() >= 2) else {
        return chart;
    };
    let top = max
        .unwrap_or_else(|| data.iter().copied().fold(1.0, f32::max))
        .max(1e-3);
    let cap = omnihw_core::sensors::HISTORY_LEN as f32 - 1.0;
    let start = cap - (data.len() as f32 - 1.0);
    // Points as fractions of the plot area (x from the left, y from the bottom), and the
    // colour of each segment, worked out now so the paint step needs nothing but bounds.
    let pts: Vec<(f32, f32)> = data
        .iter()
        .enumerate()
        .map(|(i, v)| ((start + i as f32) / cap, (v / top).clamp(0.0, 1.0)))
        .collect();
    let colors: Vec<Rgba> = (0..data.len() - 1)
        .map(|i| color_of(data[i].max(data[i + 1])))
        .collect();
    chart.child(
        canvas(
            |_, _, _| (),
            move |b, _, window, _| {
                let k = window.rem_size() / px(16.0);
                let (left, top) = (b.left() + px(2.0) * k, b.top() + px(4.0) * k);
                let w = b.size.width - px(4.0) * k;
                let h = b.size.height - px(8.0) * k;
                let at = |&(fx, fy): &(f32, f32)| point(left + w * fx, top + h * (1.0 - fy));
                // Merge consecutive segments of the same color into one polyline (one path
                // for flat colors).
                let stroke = |run: &[(f32, f32)], color: Rgba, window: &mut Window| {
                    let mut path = PathBuilder::stroke(px(1.5) * k);
                    path.move_to(at(&run[0]));
                    for p in &run[1..] {
                        path.line_to(at(p));
                    }
                    if let Ok(path) = path.build() {
                        window.paint_path(path, color);
                    }
                };
                let mut from = 0;
                for i in 1..colors.len() {
                    if colors[i] != colors[from] {
                        stroke(&pts[from..=i], colors[from], window);
                        from = i;
                    }
                }
                stroke(&pts[from..], colors[from], window);
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full(),
    )
}

/// A filled bar with `text` on it. The text is in the theme's normal text colour on a
/// theme-aware fill, so it stays readable on saturated colours in light mode.
pub(super) fn bar(frac: f32, text: String, color: Rgba, cx: &App) -> Div {
    let fill = heat::soft_fill(ui::dark(cx), color);
    let ink = crate::theme::palette(cx).strong;
    div()
        .relative()
        .flex_none()
        .w_full()
        .min_h(rems(18.0 / 16.0))
        .rounded_full()
        .overflow_hidden()
        .bg(track(cx))
        .child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .h_full()
                .w(relative(frac.clamp(0.0, 1.0)))
                .rounded_full()
                .bg(fill),
        )
        .child(
            h_flex()
                .relative()
                .w_full()
                .h(rems(18.0 / 16.0))
                .px(px(8.0))
                .child(ui::label(text).text_color(ink)),
        )
}
