//! Event Log page: crashes, hardware errors, GPU driver timeouts and disk problems from the
//! System and Application logs, in plain English, with filters. The logs are read on a worker
//! thread (see [`crate::sampler::Sampler::read_event_log`]); this page only draws the last read.
//!
//! The reader keeps at most [`omnihw_core::eventlog::TOTAL_CAP`] entries, and the page draws
//! only the ones that pass the filters.

use crate::app::OmniApp;
use crate::heat::{ORANGE, RED, YELLOW};
use crate::ui::{self, Table};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::separator::Separator;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{Disableable, Selectable, Sizable};
use gpui_kit::{
    AnyElement, AppContext, ClipboardItem, Context, Entity, InteractiveElement, IntoElement,
    ParentElement, Rgba, SharedString, StatefulInteractiveElement, Styled, Subscription, Window,
    div, rems,
};
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
    /// The search field, created when the filters first show; it writes `filter.text`.
    search: Option<(Entity<InputState>, Subscription)>,
}

/// Date range choices: (label, seconds).
const RANGES: [(&str, i64); 3] = [
    ("24 h", 86_400),
    ("7 days", 7 * 86_400),
    ("30 days", VIEW_DAYS as i64 * 86_400),
];

fn level_color(l: EventLevel) -> Rgba {
    match l {
        EventLevel::Critical => RED,
        EventLevel::Error => ORANGE,
        EventLevel::Warning => YELLOW,
    }
}

/// A coloured dot and the level name.
fn level_cell(l: EventLevel) -> AnyElement {
    ui::row()
        .gap_1p5()
        .child(
            div()
                .flex_none()
                .size(rems(0.56))
                .rounded_full()
                .bg(level_color(l)),
        )
        .child(ui::label(l.name()))
        .into_any_element()
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// "Analyze crash dumps" for the logged blue screens, and what the dumps point at. The same
/// results feed the Health Check's blue screen finding.
fn crash_dumps(app: &OmniApp, cx: &mut Context<OmniApp>) -> AnyElement {
    let (dumps, busy) = {
        let snap = app.sampler.snapshot();
        (snap.dumps.clone(), snap.dumps_busy)
    };
    let (label, hover) = if crate::elevate::elevated() {
        ("Analyze crash dumps", "Reads C:\\Windows\\Minidump.")
    } else {
        (
            "Analyze crash dumps (asks for administrator)",
            "Reads C:\\Windows\\Minidump. Windows asks for administrator permission because only administrators can read crash dumps.",
        )
    };
    let mut row = ui::wrap_row().child(
        Button::new("analyze-dumps")
            .label(label)
            .tooltip(hover)
            .disabled(busy)
            .on_click(cx.listener(|this, _, _, _| this.sampler.read_dumps())),
    );
    if busy {
        row = row.child(Spinner::new().small());
    }
    let mut out = ui::column().child(ui::space(4.0)).child(row);
    match dumps {
        _ if busy => {}
        Some(Err(e)) => {
            out = out.child(ui::colored(
                format!("Couldn't read crash dumps: {e}"),
                RED,
                cx,
            ));
        }
        Some(Ok(d)) if d.is_empty() => {
            out = out.child(ui::weak("No crash dumps found", cx));
        }
        Some(Ok(d)) => {
            for (i, line) in d
                .iter()
                .map(omnihw_core::diagnostics::dump_line)
                .enumerate()
            {
                out = out.child(div().id(("dump", i)).child(ui::label(format!("• {line}"))));
            }
        }
        None => {}
    }
    out.into_any_element()
}

/// The search field, created on first use. Typing filters the list at once.
fn search_field(
    app: &mut OmniApp,
    window: &mut Window,
    cx: &mut Context<OmniApp>,
) -> Entity<InputState> {
    if let Some((state, _)) = &app.views.eventlog.search {
        return state.clone();
    }
    let text = app.views.eventlog.filter.text.clone();
    let state = cx.new(|cx| {
        InputState::new(window, cx)
            .placeholder("Search")
            .default_value(text)
    });
    let sub = cx.subscribe_in(
        &state,
        window,
        |this: &mut OmniApp, state: &Entity<InputState>, event: &InputEvent, _, cx| {
            if let InputEvent::Change = event {
                this.views.eventlog.filter.text = state.read(cx).value().to_string();
                cx.notify();
            }
        },
    );
    app.views.eventlog.search = Some((state.clone(), sub));
    state
}

pub fn show(app: &mut OmniApp, window: &mut Window, cx: &mut Context<OmniApp>) -> AnyElement {
    // Only an Arc clone under the lock.
    let (log, busy) = {
        let snap = app.sampler.snapshot();
        (snap.event_log.clone(), snap.event_log_busy)
    };

    let mut page = ui::column()
        .child(ui::page_title("Event Log"))
        .child(ui::weak(
            format!(
                "Crashes, hardware errors, graphics driver timeouts and disk problems Windows logged in the last {VIEW_DAYS} days, in plain English. Click an event's meaning for its details."
            ),
            cx,
        ))
        .child(ui::space(6.0));
    let mut refresh = ui::row().child(
        Button::new("refresh")
            .small()
            .label("Refresh")
            .tooltip("Read the System and Application event logs again")
            .disabled(busy)
            .on_click(cx.listener(|this, _, _, cx| {
                this.sampler.read_event_log();
                cx.notify();
            })),
    );
    if busy {
        refresh = refresh
            .child(Spinner::new().small())
            .child(ui::weak("Reading the event logs...", cx));
    } else if let Some((_, at)) = &log {
        refresh = refresh.child(ui::weak(format!("Read {}", ui::ago(*at)), cx));
    }
    page = page.child(refresh);
    let Some((log, at)) = log else {
        return page.into_any_element();
    };
    let state = &mut app.views.eventlog;
    if state.loaded != Some(at) {
        state.expanded.clear();
        state.loaded = Some(at);
    }
    for e in &log.errors {
        page = page.child(ui::colored(format!("Couldn't read the {e}"), ORANGE, cx));
    }
    if log
        .entries
        .iter()
        .any(|e| e.category == EventCategory::BlueScreen)
    {
        page = page.child(crash_dumps(app, cx));
    }
    page = page.child(ui::space(4.0));

    // Filters: category and level drop-downs, date range, search.
    let search = search_field(app, window, cx);
    let f = &app.views.eventlog.filter;
    let mut categories: Vec<SharedString> = vec!["All categories".into()];
    for c in EventCategory::ALL {
        let n = log.entries.iter().filter(|e| e.category == c).count();
        categories.push(format!("{} ({n})", c.name()).into());
    }
    let mut levels: Vec<SharedString> = vec!["All levels".into()];
    levels.extend(EventLevel::ALL.iter().map(|l| SharedString::from(l.name())));
    let mut filters = ui::wrap_row()
        .child(ui::dropdown(
            "eventlog-category",
            f.category.map_or("All categories", EventCategory::name),
            categories,
            Some(f.category.map_or(0, |c| {
                1 + EventCategory::ALL.iter().position(|&x| x == c).unwrap_or(0)
            })),
            cx,
            |this, i, _, _| {
                this.views.eventlog.filter.category =
                    i.checked_sub(1).map(|i| EventCategory::ALL[i]);
            },
        ))
        .child(ui::dropdown(
            "eventlog-level",
            f.level.map_or("All levels", EventLevel::name),
            levels,
            Some(f.level.map_or(0, |l| {
                1 + EventLevel::ALL.iter().position(|&x| x == l).unwrap_or(0)
            })),
            cx,
            |this, i, _, _| {
                this.views.eventlog.filter.level = i.checked_sub(1).map(|i| EventLevel::ALL[i]);
            },
        ))
        .child(Separator::vertical().h(rems(1.2)));
    for (n, (label, secs)) in RANGES.into_iter().enumerate() {
        filters = filters.child(
            Button::new(("range", n))
                .small()
                .ghost()
                .label(label)
                .selected(f.within_secs == secs)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.views.eventlog.filter.within_secs = secs;
                    cx.notify();
                })),
        );
    }
    filters = filters
        .child(Separator::vertical().h(rems(1.2)))
        .child(Input::new(&search).small().w(rems(11.25)));
    page = page.child(filters);

    let now = now_unix();
    let state = &app.views.eventlog;
    let shown: Vec<(usize, &LogEntry)> = log
        .entries
        .iter()
        .enumerate()
        .filter(|(_, e)| state.filter.matches(e, now))
        .collect();
    page = page.child(
        ui::row()
            .child(ui::label(format!(
                "{} of {}",
                shown.len(),
                ui::plural(log.entries.len(), "event")
            )))
            .child(
                Button::new("copy-events")
                    .small()
                    .label("Copy")
                    .tooltip("Copy the events shown as plain text")
                    .disabled(shown.is_empty())
                    .on_click(cx.listener(|this, _, _, cx| {
                        let log = this.sampler.snapshot().event_log.clone();
                        if let Some((log, _)) = log {
                            let (filter, now) = (&this.views.eventlog.filter, now_unix());
                            let text =
                                to_text(log.entries.iter().filter(|e| filter.matches(e, now)));
                            cx.write_to_clipboard(ClipboardItem::new_string(text));
                        }
                    })),
            ),
    );
    page = page.child(ui::space(4.0));
    if log.entries.is_empty() {
        return page
            .child(ui::weak(
                format!("No critical events in the last {VIEW_DAYS} days"),
                cx,
            ))
            .into_any_element();
    }
    if shown.is_empty() {
        return page
            .child(ui::weak("No events match these filters.", cx))
            .into_any_element();
    }

    let weak = ui::weak_color(cx);
    let mut table = Table::new(&["Time", "Level", "Category", "Source", "Meaning"]).striped();
    for (i, e) in shown {
        let open = state.expanded.contains(&i);
        let meaning = if e.detail.is_empty() {
            ui::clickable(("event", i)).child(ui::label(e.title))
        } else {
            let text = format!("{} {}", if open { "⏷" } else { "⏵" }, e.title);
            let mut cell = ui::clickable(("event", i))
                .cursor_pointer()
                .child(ui::label(text))
                .on_click(cx.listener(move |this, _, _, cx| {
                    let expanded = &mut this.views.eventlog.expanded;
                    if !expanded.remove(&i) {
                        expanded.insert(i);
                    }
                    cx.notify();
                }));
            if open {
                cell = cell.child(ui::label(e.detail.clone()).text_color(weak));
            }
            cell
        };
        table.push_row([
            ui::label(e.time_text()).into_any_element(),
            level_cell(e.level),
            ui::label(e.category.name()).into_any_element(),
            ui::label(format!("{} ({})", e.source(), e.channel.name())).into_any_element(),
            // Long details wrap instead of widening the table past the page.
            meaning
                .flex()
                .flex_col()
                .max_w(rems(36.0))
                .into_any_element(),
        ]);
    }
    page.child(table.build(cx)).into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Tab;
    use crate::sampler::Snapshot;
    use crate::settings::Settings;
    use crate::tabs::page_tests::{assert_shows, go, machines, open, page, snapshot, texts};
    use gpui_kit::TestAppContext;
    use gpui_kit::test::TestWindowExt;
    use omnihw_core::eventlog::{Channel, EventLog};
    use std::sync::Arc;

    fn entry(
        age_secs: i64,
        level: EventLevel,
        category: EventCategory,
        title: &'static str,
        detail: &str,
    ) -> LogEntry {
        LogEntry {
            unix: now_unix() - age_secs,
            utc_offset: Some(0),
            channel: Channel::System,
            provider: "Microsoft-Windows-Kernel-Power".into(),
            id: 41,
            level,
            category,
            title,
            detail: detail.into(),
        }
    }

    fn with_log(info: &omnihw_core::SystemInfo) -> Snapshot {
        let log = EventLog {
            entries: vec![
                entry(
                    3_600,
                    EventLevel::Critical,
                    EventCategory::UnexpectedShutdown,
                    "The PC lost power or stopped responding",
                    "BugcheckCode 0",
                ),
                entry(
                    3 * 86_400,
                    EventLevel::Error,
                    EventCategory::Disk,
                    "A disk reported a bad block",
                    "",
                ),
            ],
            errors: vec!["Application log: Access is denied.".into()],
        };
        Snapshot {
            event_log: Some((Arc::new(log), SystemTime::now())),
            ..snapshot(info)
        }
    }

    #[gpui_kit::test]
    fn event_log_page(cx: &mut TestAppContext) {
        for info in machines() {
            let shown = page(cx, &info, with_log(&info), Tab::EventLog);
            assert_shows(
                &shown,
                &[
                    "Event Log",
                    "Refresh",
                    "2 of 2 events",
                    "Kernel-Power 41 (System)",
                    "Couldn't read the Application log",
                    "All categories",
                    "All levels",
                    "30 days",
                    "Copy",
                ],
            );
        }
    }

    #[gpui_kit::test]
    fn unread_and_empty_logs(cx: &mut TestAppContext) {
        let info = machines()[0].clone();
        // Not read yet: only the heading and Refresh.
        let shown = page(cx, &info, Snapshot::default(), Tab::EventLog);
        assert_shows(&shown, &["Event Log", "Refresh"]);
        // Read, nothing logged.
        let snap = Snapshot {
            event_log: Some((Arc::new(EventLog::default()), SystemTime::now())),
            ..Default::default()
        };
        let shown = page(cx, &info, snap, Tab::EventLog);
        assert_shows(&shown, &["No critical events in the last 30 days"]);
    }

    #[gpui_kit::test]
    fn range_filter_and_expanding_a_row(cx: &mut TestAppContext) {
        let info = machines()[0].clone();
        let (window, app) = open(cx, info.clone(), with_log(&info), Settings::default());
        go(cx, &app, Tab::EventLog);
        texts(cx, window);
        // The detail shows only once the row is opened.
        assert!(!texts(cx, window).iter().any(|t| t == "BugcheckCode 0"));
        cx.update_window(window, |_, window, cx| window.click(("event", 0usize), cx))
            .unwrap();
        assert_shows(&texts(cx, window), &["BugcheckCode 0"]);
        // "24 h" hides the three-day-old disk event.
        cx.update_window(window, |_, window, cx| window.click(("range", 0usize), cx))
            .unwrap();
        let shown = texts(cx, window);
        assert_shows(&shown, &["1 of 2 events"]);
        assert!(!shown.iter().any(|t| t.contains("bad block")));
    }
}
