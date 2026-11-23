//! Alerts tab: create and edit alert rules, see which are firing, and review history.

use crate::app::OmniApp;
use crate::heat::{ORANGE, RED};
use crate::sampler::Snapshot;
use crate::ui::LineSized;
use crate::ui::{self, Table};
use gpui_kit::component::button::Button;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::{ActiveTheme, Disableable, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AnyElement, App, AppContext, ClickEvent, Context, ElementId, Entity, InteractiveElement,
    IntoElement, ParentElement, SharedString, Styled, Subscription, Window, div, px, rems,
};
use omnihw_core::SystemInfo;
use omnihw_core::alerts::{AlertRule, Condition};
use omnihw_core::sensors::{Tracked, Unit};
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

/// Status message from the last save attempt, and the text fields of the rule rows.
#[derive(Default)]
pub struct ViewState {
    save_error: Option<String>,
    /// Limit and duration fields, one entry per rule, in rule order.
    inputs: Vec<RuleInputs>,
}

/// The two number fields of one rule row. Dropping it ends its subscriptions.
struct RuleInputs {
    threshold: Entity<InputState>,
    duration: Entity<InputState>,
    _subscriptions: [Subscription; 2],
}

/// Which number of a rule a text field edits.
#[derive(Clone, Copy)]
enum Field {
    Threshold,
    Duration,
}

impl Field {
    fn value(self, r: &AlertRule) -> f64 {
        match self {
            Field::Threshold => r.threshold,
            Field::Duration => r.duration_secs,
        }
    }

    /// Store a typed value; durations are kept within 0..=3600 s as before.
    fn set(self, r: &mut AlertRule, v: f64) {
        match self {
            Field::Threshold => r.threshold = v,
            Field::Duration => r.duration_secs = v.clamp(0.0, 3600.0),
        }
    }
}

fn sensor_name(t: &Tracked) -> String {
    format!("{} / {}", t.reading.group, t.reading.label)
}

fn rule_for(t: &Tracked, condition: Condition, threshold: f64, duration_secs: f64) -> AlertRule {
    AlertRule {
        enabled: true,
        sensor_id: t.reading.id.clone(),
        sensor_name: sensor_name(t),
        unit: t.reading.unit,
        condition,
        threshold,
        duration_secs,
    }
}

/// Common alerts for the sensors this machine has.
fn presets(info: &SystemInfo, snap: &Snapshot) -> Vec<(String, AlertRule)> {
    let mut out = Vec::new();
    let mut add = |label: String, id: &str, cond, threshold, dur| {
        if let Some(t) = snap.table.get(id) {
            out.push((label, rule_for(t, cond, threshold, dur)));
        }
    };
    for g in &info.gpus {
        let pre = g.sensor_prefix();
        add(
            format!("{} over 85 °C", g.name),
            &format!("{pre}/temp"),
            Condition::Above,
            85.0,
            30.0,
        );
    }
    add(
        "CPU usage over 95 % for 2 min".into(),
        "cpu/load/total",
        Condition::Above,
        95.0,
        120.0,
    );
    add(
        "Memory over 90 % for 1 min".into(),
        "mem/load",
        Condition::Above,
        90.0,
        60.0,
    );
    for d in &info.drives {
        add(
            format!("{} over 70 °C", d.name()),
            &format!("{}/temp", d.sensor_prefix()),
            Condition::Above,
            70.0,
            30.0,
        );
    }
    add(
        "Battery below 20 %".into(),
        "battery/charge",
        Condition::Below,
        20.0,
        0.0,
    );
    out
}

fn ago(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    match s {
        0..60 => "just now".into(),
        60..3600 => format!("{} min ago", s / 60),
        3600..86400 => format!("{} h ago", s / 3600),
        _ => format!("{} days ago", s / 86400),
    }
}

/// Every sensor, sorted by group and label (the sensor drop-down and "Add custom alert").
fn sorted_sensors(snap: &Snapshot) -> Vec<&Tracked> {
    let mut sensors: Vec<&Tracked> = snap.table.entries().iter().collect();
    sensors.sort_by(|a, b| {
        (&a.reading.group, &a.reading.label).cmp(&(&b.reading.group, &b.reading.label))
    });
    sensors
}

/// A number as typed in a field: "85", "0.5".
fn number_text(v: f64) -> String {
    format!("{v}")
}

/// A typed number, or `None` while the text isn't one (the rule keeps its last value).
fn parse_number(text: &str) -> Option<f64> {
    text.trim().parse::<f64>().ok().filter(|v| v.is_finite())
}

/// Edit the alert rules and, if they changed, save them (keeping any save error to show).
fn edit_rules(
    this: &mut OmniApp,
    cx: &mut Context<OmniApp>,
    edit: impl FnOnce(&mut Vec<AlertRule>),
) {
    let mut rules = this.sampler.snapshot().alerts.rules().to_vec();
    let original = rules.clone();
    edit(&mut rules);
    if rules != original {
        this.views.alerts.save_error = this.sampler.set_alert_rules(rules).err();
    }
    cx.notify();
}

/// A text field for one number of a rule. It updates the rule as you type (while the text is
/// a number) and shows the stored value again when it loses focus.
fn number_field(
    value: f64,
    field: Field,
    window: &mut Window,
    cx: &mut Context<OmniApp>,
) -> (Entity<InputState>, Subscription) {
    let state = cx.new(|cx| InputState::new(window, cx).default_value(number_text(value)));
    let sub = cx.subscribe_in(
        &state,
        window,
        move |this: &mut OmniApp, state: &Entity<InputState>, event: &InputEvent, window, cx| {
            let Some(i) = this
                .views
                .alerts
                .inputs
                .iter()
                .position(|r| r.threshold == *state || r.duration == *state)
            else {
                return;
            };
            match event {
                InputEvent::Change => {
                    if let Some(v) = parse_number(&state.read(cx).value()) {
                        edit_rules(this, cx, |rules| {
                            if let Some(r) = rules.get_mut(i) {
                                field.set(r, v);
                            }
                        });
                    }
                }
                InputEvent::Blur => {
                    let stored = this
                        .sampler
                        .snapshot()
                        .alerts
                        .rules()
                        .get(i)
                        .map(|r| field.value(r));
                    if let Some(v) = stored {
                        state.update(cx, |s, cx| s.set_value(number_text(v), window, cx));
                    }
                }
                _ => {}
            }
        },
    );
    (state, sub)
}

/// A drop-down like [`ui::dropdown`] whose menu scrolls, for long lists (every sensor).
fn scroll_dropdown(
    id: impl Into<ElementId>,
    current: impl Into<SharedString>,
    options: Rc<Vec<SharedString>>,
    selected: Option<usize>,
    cx: &mut Context<OmniApp>,
    pick: impl Fn(&mut OmniApp, usize, &mut Window, &mut Context<OmniApp>) + 'static,
) -> AnyElement {
    let app = cx.weak_entity();
    let pick = Rc::new(pick);
    Button::new(id)
        .line_sized()
        .caption(current)
        .dropdown_caret(true)
        .dropdown_menu(move |mut menu, _, _| {
            menu = menu.scrollable(true).max_h(px(420.0)).max_w(px(400.0));
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

/// History rows (newest first): when, and what fired. Striped, like the rules table.
fn history(rows: Vec<(String, String)>, cx: &App) -> AnyElement {
    let stripe = cx.theme().muted;
    let weak = ui::weak_color(cx);
    let cells = rows.into_iter().enumerate().flat_map(|(i, (when, msg))| {
        let cell = move |col: usize| {
            div()
                .id(("alert-log", i * 2 + col))
                .px_2()
                .py_0p5()
                .when(i % 2 == 1, |c| c.bg(stripe))
        };
        [
            cell(0).child(ui::label(when).text_color(weak)),
            cell(1).child(ui::label(msg)),
        ]
    });
    ui::hug(div().grid().grid_cols_max_content(2).children(cells)).into_any_element()
}

pub fn show(app: &mut OmniApp, window: &mut Window, cx: &mut Context<OmniApp>) -> AnyElement {
    let Some(info) = app.sampler.info() else {
        return ui::column().into_any_element();
    };
    let snap = app.sampler.snapshot();
    let rules: Vec<AlertRule> = snap.alerts.rules().to_vec();
    let active: Vec<bool> = (0..rules.len()).map(|i| snap.alerts.is_active(i)).collect();
    let sensors = sorted_sensors(&snap);
    // (id, name, unit) of every sensor, shared by the sensor drop-downs of all rows.
    let choices: Rc<Vec<(String, String, Unit)>> = Rc::new(
        sensors
            .iter()
            .map(|t| (t.reading.id.clone(), sensor_name(t), t.reading.unit))
            .collect(),
    );
    let names: Rc<Vec<SharedString>> = Rc::new(
        choices
            .iter()
            .map(|(_, name, _)| SharedString::from(name.clone()))
            .collect(),
    );

    let mut page = ui::column()
        .child(ui::page_title("Alerts"))
        .child(ui::weak(
            "Get a Windows notification when a sensor stays above or below a limit. Alerts are checked on every sample while Omni-HW is running, on any tab.",
            cx,
        ))
        .child(ui::space(6.0));

    let mut quick = ui::wrap_row().child(ui::label("Quick add:"));
    for (n, (label, rule)) in presets(info, &snap).into_iter().enumerate() {
        let exists = rules
            .iter()
            .any(|r| r.sensor_id == rule.sensor_id && r.condition == rule.condition);
        quick = quick.child(
            Button::new(("preset", n))
                .line_sized()
                .caption(label)
                .disabled(exists)
                .on_click(cx.listener(move |this, _, _, cx| {
                    let rule = rule.clone();
                    edit_rules(this, cx, |rules| rules.push(rule));
                })),
        );
    }
    page = page.child(quick).child(
        ui::row()
            .child(
                Button::new("add-custom")
                    .line_sized()
                    .caption("Add custom alert")
                    .on_click(cx.listener(|this, _, _, cx| {
                        let rule = {
                            let snap = this.sampler.snapshot();
                            sorted_sensors(&snap).first().map(|t| {
                                rule_for(t, Condition::Above, t.reading.value.round(), 10.0)
                            })
                        };
                        if let Some(rule) = rule {
                            edit_rules(this, cx, |rules| rules.push(rule));
                        }
                    })),
            )
            .child(
                Button::new("test-notification")
                    .line_sized()
                    .caption("Test notification")
                    .tooltip("Shows a sample Windows notification")
                    .on_click(cx.listener(|this, _, _, _| this.sampler.test_notification())),
            ),
    );
    page = page.child(ui::space(8.0));

    // One pair of number fields per rule, created when a rule first shows.
    let inputs = &mut app.views.alerts.inputs;
    inputs.truncate(rules.len());
    while inputs.len() < rules.len() {
        let r = &rules[inputs.len()];
        let (threshold, a) = number_field(r.threshold, Field::Threshold, window, cx);
        let (duration, b) = number_field(r.duration_secs, Field::Duration, window, cx);
        inputs.push(RuleInputs {
            threshold,
            duration,
            _subscriptions: [a, b],
        });
    }

    if rules.is_empty() {
        page = page.child(ui::weak(
            "No alerts yet. Use Quick add or Add custom alert.",
            cx,
        ));
    } else {
        let mut table =
            Table::new(&["On", "Status", "Sensor", "When", "Limit", "For", ""]).striped();
        let red = ui::ink(cx, RED);
        for (i, r) in rules.iter().enumerate() {
            let on = Checkbox::new(("alert-on", i))
                .accessibility_label("On")
                .checked(r.enabled)
                .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                    let checked = *checked;
                    edit_rules(this, cx, |rules| {
                        if let Some(r) = rules.get_mut(i) {
                            r.enabled = checked;
                        }
                    });
                }));
            let status = if !r.enabled {
                ui::weak("off", cx).into_any_element()
            } else if active.get(i).copied().unwrap_or(false) {
                ui::label("FIRING").text_color(red).into_any_element()
            } else {
                let current = snap
                    .table
                    .get(&r.sensor_id)
                    .map(|t| r.unit.format(t.reading.value));
                ui::weak(
                    current.map_or("no data".to_string(), |c| format!("ok ({c})")),
                    cx,
                )
                .into_any_element()
            };
            let picks = choices.clone();
            let sensor = scroll_dropdown(
                ("alert-sensor", i),
                r.sensor_name.clone(),
                names.clone(),
                choices.iter().position(|(id, _, _)| *id == r.sensor_id),
                cx,
                move |this, n, _, cx| {
                    let Some((id, name, unit)) = picks.get(n).cloned() else {
                        return;
                    };
                    edit_rules(this, cx, |rules| {
                        if let Some(r) = rules.get_mut(i) {
                            r.sensor_id = id;
                            r.sensor_name = name;
                            r.unit = unit;
                        }
                    });
                },
            );
            let conditions = [Condition::Above, Condition::Below];
            let condition = ui::dropdown(
                ("alert-cond", i),
                match r.condition {
                    Condition::Above => "above",
                    Condition::Below => "below",
                },
                vec!["above".into(), "below".into()],
                conditions.iter().position(|c| *c == r.condition),
                cx,
                move |this, n, _, cx| {
                    edit_rules(this, cx, |rules| {
                        if let Some(r) = rules.get_mut(i) {
                            r.condition = conditions[n];
                        }
                    });
                },
            );
            let fields = &app.views.alerts.inputs[i];
            let limit = Input::new(&fields.threshold)
                .small()
                .w(rems(7.5))
                .suffix(ui::weak(r.unit.symbol(), cx));
            let duration = Input::new(&fields.duration)
                .small()
                .w(rems(6.0))
                .suffix(ui::weak("s", cx));
            let remove = Button::new(("alert-remove", i))
                .line_sized()
                .caption("Remove")
                .on_click(cx.listener(move |this, _, _, cx| {
                    edit_rules(this, cx, |rules| {
                        if i < rules.len() {
                            rules.remove(i);
                        }
                    });
                    if i < this.views.alerts.inputs.len() {
                        this.views.alerts.inputs.remove(i);
                    }
                }));
            table.push_row([
                on.into_any_element(),
                status,
                sensor,
                condition,
                limit.into_any_element(),
                duration.into_any_element(),
                remove.into_any_element(),
            ]);
        }
        page = page.child(table.build(cx));
    }
    if let Some(e) = &app.views.alerts.save_error {
        page = page.child(ui::colored(e.clone(), ORANGE, cx));
    }

    page = page.child(ui::space(12.0)).child(ui::strong("History"));
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    if snap.alert_log.is_empty() {
        page = page.child(ui::weak("No alerts have fired since Omni-HW started.", cx));
    } else {
        let rows = snap
            .alert_log
            .iter()
            .rev()
            .take(50)
            .map(|e| (ago(now - e.at_unix), e.message()))
            .collect();
        page = page.child(history(rows, cx));
    }
    page.into_any_element()
}

/// Number of alerts currently firing (for the actions bar indicator).
pub fn firing_count(snap: &Snapshot) -> usize {
    (0..snap.alerts.rules().len())
        .filter(|&i| snap.alerts.is_active(i))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Tab;
    use crate::settings::Settings;
    use crate::tabs::page_tests::{assert_shows, go, machines, open, page, snapshot, texts};
    use gpui_kit::TestAppContext;
    use omnihw_core::alerts::Alerts;

    #[gpui_kit::test]
    fn alerts_page(cx: &mut TestAppContext) {
        for info in machines() {
            let shown = page(cx, &info, snapshot(&info), Tab::Alerts);
            assert_shows(
                &shown,
                &[
                    "Alerts",
                    "Quick add:",
                    "CPU usage over 95 % for 2 min",
                    "Add custom alert",
                    "Test notification",
                    "No alerts yet",
                    "History",
                    "No alerts have fired",
                ],
            );
        }
    }

    #[gpui_kit::test]
    fn empty_alerts_page(cx: &mut TestAppContext) {
        let info = machines()[0].clone();
        let shown = page(cx, &info, Snapshot::default(), Tab::Alerts);
        assert_shows(&shown, &["Alerts", "No alerts yet"]);
    }

    /// Rules come from the snapshot; one is made to fire, so the table, the FIRING status and
    /// the history show (nothing is saved: the rules are never edited).
    #[gpui_kit::test]
    fn rules_and_history(cx: &mut TestAppContext) {
        let info = machines()[0].clone();
        let mut snap = snapshot(&info);
        let load = snap.table.get("cpu/load/total").expect("CPU load sensor");
        let firing = rule_for(load, Condition::Above, -1.0, 0.0);
        let mut off = rule_for(load, Condition::Below, 5.0, 10.0);
        off.enabled = false;
        let mut alerts = Alerts::new(vec![firing, off]);
        let fired = alerts.evaluate(1.0, 1_000.0, &snap.table);
        assert_eq!(fired.len(), 1);
        snap.alerts = alerts;
        snap.alert_log = fired;
        assert_eq!(firing_count(&snap), 1);

        let (window, app) = open(cx, info, snap, Settings::default());
        go(cx, &app, Tab::Alerts);
        let shown = texts(cx, window);
        assert_shows(
            &shown,
            &[
                "Status", "FIRING", "off", "above", "below", "Remove", "days ago",
            ],
        );
        // Each rule got its pair of number fields.
        assert_eq!(cx.read(|cx| app.read(cx).views.alerts.inputs.len()), 2);
    }

    #[test]
    fn ago_and_numbers() {
        assert_eq!(ago(5.0), "just now");
        assert_eq!(ago(150.0), "2 min ago");
        assert_eq!(ago(7_300.0), "2 h ago");
        assert_eq!(ago(200_000.0), "2 days ago");
        assert_eq!(parse_number(" 85 "), Some(85.0));
        assert_eq!(parse_number("0.5"), Some(0.5));
        assert_eq!(parse_number("abc"), None);
        assert_eq!(number_text(85.0), "85");
    }
}
