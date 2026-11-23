//! Sensors page: every reading with its current, minimum, maximum and average value.

use crate::app::OmniApp;
use crate::ui::LineSized;
use crate::ui::{self, Table};
use gpui_kit::component::button::Button;
use gpui_kit::component::separator::Separator;
use gpui_kit::{
    AnyElement, App, Context, Div, ElementId, IntoElement, ParentElement, SharedString, Styled,
    Window,
};
use omnihw_core::sensors::Tracked;

pub fn show(app: &mut OmniApp, _window: &mut Window, cx: &mut Context<OmniApp>) -> AnyElement {
    let ms = app.sampler.interval_ms();
    let intervals = crate::settings::INTERVALS_MS;
    let toolbar = ui::row()
        .child(
            Button::new("reset-stats")
                .line_sized()
                .caption("Reset min/max")
                .on_click(cx.listener(|this, _, _, _| this.sampler.reset_stats())),
        )
        .child(ui::dropdown(
            "interval",
            format!("{ms} ms"),
            intervals
                .iter()
                .map(|v| SharedString::from(format!("{v} ms")))
                .collect(),
            intervals.iter().position(|&v| v == ms),
            cx,
            move |this, i, _, _| this.sampler.set_interval_ms(intervals[i]),
        ))
        .child(ui::label("Interval"));
    let mut page = ui::column().child(toolbar).child(Separator::horizontal());

    let snap = app.sampler.snapshot();
    let entries = snap.table.entries();
    if entries.is_empty() {
        return page
            .child(ui::label("Waiting for first sample..."))
            .into_any_element();
    }

    // Groups in first-seen order.
    let mut groups: Vec<&str> = Vec::new();
    for t in entries {
        if !groups.contains(&t.reading.group.as_str()) {
            groups.push(&t.reading.group);
        }
    }
    for g in groups {
        let key = format!("sensors/{g}");
        let open = ui::is_open(app, &key, true);
        page = page.child(ui::collapse_header(
            key,
            ui::strong(g.to_string()),
            open,
            cx,
        ));
        if open {
            let rows = entries.iter().filter(|t| t.reading.group == g);
            let grid = sensor_grid(ElementId::Name(format!("sensors-{g}").into()), rows, cx);
            page = page.child(ui::indented(grid, cx));
        }
    }
    page.into_any_element()
}

/// Sensor | Current | Min | Max | Average table.
/// `id` must be unique on the page.
pub fn sensor_grid<'a>(
    id: impl Into<ElementId>,
    rows: impl Iterator<Item = &'a Tracked>,
    cx: &App,
) -> Div {
    let mut table = Table::new(&["Sensor", "Current", "Min", "Max", "Average"])
        .id(id)
        .striped();
    for t in rows {
        let u = t.reading.unit;
        table.push_row([
            ui::label(t.reading.label.clone()).into_any_element(),
            ui::mono(u.format(t.reading.value), cx)
                .text_color(crate::theme::palette(cx).strong)
                .into_any_element(),
            ui::mono(u.format(t.min), cx).into_any_element(),
            ui::mono(u.format(t.max), cx).into_any_element(),
            ui::mono(u.format(t.avg()), cx).into_any_element(),
        ]);
    }
    table.build(cx)
}
