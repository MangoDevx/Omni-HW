//! Power page: power plans, timers, buttons, processor limits, device power saving, wake
//! sources and power requests, with in-place editing. The page only reads the snapshot; its
//! buttons and drop-downs call [`request`] (or the sampler), which applies changes on a worker
//! thread. Results arrive as status bar messages.
//!
//! The helpers shared with the Health Check page ([`request`], [`fix_button`],
//! [`admin_hint`]) live here too.

use crate::app::OmniApp;
use crate::elevate::elevated;
use crate::heat::{GREEN, ORANGE};
use crate::ui::{self, Table};
use gpui_kit::component::button::Button;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme, Disableable, Selectable, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AnyElement, App, Context, Div, ElementId, InteractiveElement, IntoElement, ParentElement,
    SharedString, Stateful, Styled, Window, div,
};
use omnihw_core::power::{
    Action, AdapterKind, Kind, PERCENT_PRESETS, PowerInfo, Setting, Source, TIMEOUT_PRESETS,
};

/// Selections, filters and edit buffers of this page. It needs none: changes go straight to
/// [`request`] (whose confirmation is a dialog) and the wake history's open/closed state is
/// kept with the other collapsible sections (`ui::is_open`).
#[derive(Default)]
pub struct ViewState {}

/// Key of the collapsible wake history.
const WAKE_HISTORY: &str = "power/wake-history";

pub fn show(app: &mut OmniApp, _window: &mut Window, cx: &mut Context<OmniApp>) -> AnyElement {
    let history_open = ui::is_open(app, WAKE_HISTORY, false);
    let snap = app.sampler.snapshot();
    let busy = snap.power_busy;
    let mut header = ui::row().child(ui::page_title("Power"));
    if busy {
        header = header
            .child(Spinner::new().small())
            .child(ui::weak("Working...", cx));
    } else {
        header = header.child(
            Button::new("power-refresh")
                .small()
                .label("Refresh")
                .tooltip("Read power settings, devices and wake history again")
                .on_click(cx.listener(|this, _, _, _| this.sampler.refresh_power())),
        );
    }
    let page = ui::column().child(header);
    let Some(p) = &snap.power else {
        return page
            .child(
                ui::row()
                    .child(Spinner::new().small())
                    .child(ui::label("Reading power settings...")),
            )
            .into_any_element();
    };
    let mut page = page
        .child(ui::weak(
            "Changes apply to the active power plan right away. Some need administrator \
             permission; Windows asks when needed.",
            cx,
        ))
        .child(ui::space(8.0))
        .child(plans(p, busy, cx));
    page = page.child(settings_group(
        p,
        busy,
        "Sleep and display",
        &[
            Setting::DisplayOff,
            Setting::Sleep,
            Setting::Hibernate,
            Setting::DiskOff,
        ]
        .into_iter()
        .filter(|s| *s != Setting::Hibernate || p.caps.hibernate)
        .collect::<Vec<_>>(),
        cx,
    ));
    let mut buttons = vec![Setting::PowerButton];
    if p.caps.sleep_button {
        buttons.push(Setting::SleepButton);
    }
    if p.caps.lid {
        buttons.push(Setting::LidAction);
    }
    page = page.child(settings_group(
        p,
        busy,
        "Power buttons and lid",
        &buttons,
        cx,
    ));
    page = page.child(settings_group(
        p,
        busy,
        "Processor",
        &[Setting::ProcessorMin, Setting::ProcessorMax],
        cx,
    ));
    let mut adv = vec![
        Setting::UsbSelectiveSuspend,
        Setting::PcieAspm,
        Setting::WakeTimers,
    ];
    if p.adapters.iter().any(|a| a.kind == AdapterKind::WiFi) {
        adv.push(Setting::WirelessPowerSaving);
    }
    if p.is_portable() {
        adv.push(Setting::BatterySaverThreshold);
    }
    page = page.child(settings_group(p, busy, "Advanced", &adv, cx));
    if !p.sensitive_usb.is_empty() {
        let names: Vec<String> = p
            .sensitive_usb
            .iter()
            .map(|(n, k)| format!("{n} ({})", k.name()))
            .collect();
        page = page
            .child(ui::weak(
                format!(
                    "USB devices that may drop out with selective suspend: {}",
                    names.join(", ")
                ),
                cx,
            ))
            .child(ui::space(6.0));
    }
    page = page.child(startup(p, busy, cx));
    if !p.adapters.is_empty() {
        page = page.child(adapters(p, busy, cx));
    }
    page.child(waking(p, busy, history_open, cx))
        .child(requests(p, busy, cx))
        .into_any_element()
}

/// A titled group box. The title doubles as its id, so groups' identical labels ("-") stay
/// apart.
fn group(title: &'static str, cx: &App) -> Stateful<Div> {
    ui::group_box(cx).id(title).child(ui::strong(title))
}

/// Cells of a free-form grid, each with its own id so equal texts ("-") in one grid don't
/// clash.
fn grid_of(cols: u16, cells: Vec<AnyElement>) -> Div {
    ui::hug(
        ui::grid(cols).children(
            cells
                .into_iter()
                .enumerate()
                .map(|(i, c)| div().id(i).child(c)),
        ),
    )
}

fn plans(p: &PowerInfo, busy: bool, cx: &mut Context<OmniApp>) -> impl IntoElement {
    let g = group("Power plan", cx);
    if p.schemes.is_empty() {
        return g.child(ui::weak("No power plans found.", cx));
    }
    let mut row = ui::wrap_row();
    for (i, s) in p.schemes.iter().enumerate() {
        let active = p.active.as_deref() == Some(s.guid.as_str());
        let name = if s.name.is_empty() {
            "Unnamed plan"
        } else {
            &s.name
        };
        let guid = s.guid.clone();
        row = row.child(
            Button::new(("plan", i))
                .small()
                .label(name.to_string())
                .selected(active)
                .disabled(busy)
                .tooltip(if active {
                    "Active plan"
                } else {
                    "Switch to this plan"
                })
                .on_click(cx.listener(move |this, _, window, cx| {
                    if !active {
                        request(this, Action::SetScheme(guid.clone()), window, cx);
                    }
                })),
        );
    }
    g.child(row)
}

/// Rows of AC (and battery, on laptops) value pickers.
fn settings_group(
    p: &PowerInfo,
    busy: bool,
    title: &'static str,
    settings: &[Setting],
    cx: &mut Context<OmniApp>,
) -> impl IntoElement {
    let portable = p.is_portable();
    let mut cells: Vec<AnyElement> = Vec::new();
    if portable {
        cells.push(div().into_any_element());
        cells.push(ui::label("Plugged in").underline().into_any_element());
        cells.push(ui::label("On battery").underline().into_any_element());
    }
    for &s in settings {
        let v = p.get(s);
        if v.ac.is_none() && v.dc.is_none() {
            continue;
        }
        cells.push(ui::label(s.label()).into_any_element());
        // Battery saver only matters on battery.
        if s != Setting::BatterySaverThreshold {
            cells.push(value_combo(s, Source::Ac, v.ac, busy, cx));
        } else {
            cells.push(ui::weak("-", cx).into_any_element());
        }
        if portable {
            cells.push(value_combo(s, Source::Dc, v.dc, busy, cx));
        }
    }
    group(title, cx).child(grid_of(if portable { 3 } else { 2 }, cells))
}

/// Choices offered for a setting, including the current value if it isn't a preset.
fn options(s: Setting, current: u32) -> Vec<u32> {
    let mut v: Vec<u32> = match s.kind() {
        Kind::Seconds => TIMEOUT_PRESETS.to_vec(),
        Kind::Percent => PERCENT_PRESETS.to_vec(),
        Kind::Choice(c) => c.iter().map(|(v, _)| *v).collect(),
    };
    if !v.contains(&current) {
        v.push(current);
    }
    if s.kind() == Kind::Seconds {
        // Ascending, with "Never" (0) last.
        v.sort_by_key(|&x| if x == 0 { u32::MAX } else { x });
    } else {
        v.sort_unstable();
    }
    v
}

fn value_combo(
    s: Setting,
    source: Source,
    current: Option<u32>,
    busy: bool,
    cx: &mut Context<OmniApp>,
) -> AnyElement {
    let Some(cur) = current else {
        return ui::weak("-", cx).into_any_element();
    };
    let id = ElementId::Name(
        format!(
            "power-value-{}-{}",
            s.key(),
            if source == Source::Ac { "ac" } else { "dc" }
        )
        .into(),
    );
    if busy {
        // A change is being applied: show the value without a menu.
        return Button::new(id)
            .small()
            .label(s.format(cur))
            .dropdown_caret(true)
            .disabled(true)
            .into_any_element();
    }
    let values = options(s, cur);
    ui::dropdown(
        id,
        s.format(cur),
        values
            .iter()
            .map(|&v| SharedString::from(s.format(v)))
            .collect(),
        values.iter().position(|&v| v == cur),
        cx,
        move |this, i, window, cx| {
            let value = values[i];
            if value != cur {
                request(
                    this,
                    Action::SetValue {
                        setting: s,
                        source,
                        value,
                    },
                    window,
                    cx,
                );
            }
        },
    )
}

fn on_off(on: bool, on_text: &'static str, off_text: &'static str, cx: &App) -> AnyElement {
    if on {
        ui::colored(on_text, ORANGE, cx).into_any_element()
    } else {
        ui::colored(off_text, GREEN, cx).into_any_element()
    }
}

fn action_button(
    id: impl Into<ElementId>,
    text: &'static str,
    action: Action,
    busy: bool,
    cx: &mut Context<OmniApp>,
) -> AnyElement {
    Button::new(id)
        .small()
        .label(text)
        .tooltip(admin_hint(&action))
        .disabled(busy)
        .on_click(cx.listener(move |this, _, window, cx| request(this, action.clone(), window, cx)))
        .into_any_element()
}

fn startup(p: &PowerInfo, busy: bool, cx: &mut Context<OmniApp>) -> impl IntoElement {
    let mut cells: Vec<AnyElement> = vec![ui::label("Fast Startup").into_any_element()];
    match p.fast_startup {
        Some(on) => {
            let effective = p.fast_startup_effective() == Some(true);
            cells.push(
                ui::label(match (on, effective) {
                    (true, true) => "On",
                    (true, false) => "On (unused: hibernation is off)",
                    _ => "Off",
                })
                .into_any_element(),
            );
            cells.push(action_button(
                "fast-startup",
                if on { "Turn off" } else { "Turn on" },
                Action::FastStartup(!on),
                busy,
                cx,
            ));
        }
        None => {
            cells.push(ui::weak("-", cx).into_any_element());
            cells.push(div().into_any_element());
        }
    }
    cells.push(ui::label("Hibernation").into_any_element());
    cells.push(
        ui::label(match (p.caps.hibernate, p.caps.hiberfile) {
            (true, true) => "On",
            (true, false) => "Off",
            _ => "Not supported",
        })
        .into_any_element(),
    );
    cells.push(div().into_any_element());
    cells.push(ui::label("Standby").into_any_element());
    cells.push(
        ui::label(if p.caps.modern_standby {
            "Modern Standby (S0 low power idle)"
        } else if p.caps.s3 {
            "Sleep (S3)"
        } else {
            "-"
        })
        .into_any_element(),
    );
    cells.push(div().into_any_element());
    group("Startup and standby", cx).child(grid_of(3, cells))
}

fn adapters(p: &PowerInfo, busy: bool, cx: &mut Context<OmniApp>) -> impl IntoElement {
    let mut table = Table::new(&[
        "Adapter",
        "Windows may turn it off",
        "Energy-Efficient Ethernet",
    ])
    .striped();
    for (i, a) in p.adapters.iter().enumerate() {
        let turn_off = match a.allow_turn_off {
            Some(on) => ui::row()
                .child(on_off(on, "Yes", "No", cx))
                .child(action_button(
                    ("allow-turn-off", i),
                    if on { "Stop" } else { "Allow" },
                    Action::AllowTurnOff {
                        instance_id: a.instance_id.clone(),
                        allow: !on,
                    },
                    busy,
                    cx,
                ))
                .into_any_element(),
            None => ui::weak("-", cx).into_any_element(),
        };
        let eee = match (&a.eee, &a.class_key) {
            (Some(e), Some(k)) => ui::row()
                .child(on_off(e.enabled, "On", "Off", cx))
                .child(action_button(
                    ("eee", i),
                    if e.enabled { "Turn off" } else { "Turn on" },
                    Action::NicProperty {
                        class_key: k.clone(),
                        value: e.value.clone(),
                        data: if e.enabled { "0" } else { "1" }.into(),
                    },
                    busy,
                    cx,
                ))
                .into_any_element(),
            _ => ui::weak("-", cx).into_any_element(),
        };
        table.push_row([
            ui::label(format!("{} ({})", a.name, a.kind.name())).into_any_element(),
            turn_off,
            eee,
        ]);
    }
    group("Network and Bluetooth adapters", cx).child(table.build(cx))
}

fn waking(
    p: &PowerInfo,
    busy: bool,
    history_open: bool,
    cx: &mut Context<OmniApp>,
) -> impl IntoElement {
    let v = p.get(Setting::WakeTimers);
    let cells = vec![
        ui::label("Last wake from sleep").into_any_element(),
        ui::label(match p.last_wake() {
            Some(w) => format!("{} ({} UTC)", w.source.describe(), w.time),
            None => "None in the last 2 weeks".into(),
        })
        .into_any_element(),
        ui::label("Allow wake timers").into_any_element(),
        ui::label(match (v.ac, v.dc, p.is_portable()) {
            (Some(a), Some(d), true) => format!(
                "{} plugged in, {} on battery",
                Setting::WakeTimers.format(a),
                Setting::WakeTimers.format(d)
            ),
            (Some(a), _, _) => Setting::WakeTimers.format(a),
            _ => "-".into(),
        })
        .into_any_element(),
    ];
    let mut g = group("Waking", cx)
        .child(grid_of(2, cells))
        .child(ui::space(4.0))
        .child(ui::strong("Devices that can wake the PC"));
    if p.wake_armed.is_empty() {
        g = g.child(ui::weak("None", cx));
    }
    for (i, d) in p.wake_armed.iter().enumerate() {
        g = g.child(
            ui::row()
                .id(("wake-armed", i))
                .child(ui::label(d.clone()))
                .child(action_button(
                    ("disarm", i),
                    "Disarm",
                    Action::DisarmWake(d.clone()),
                    busy,
                    cx,
                )),
        );
    }
    g = g
        .child(ui::space(4.0))
        .child(ui::strong("Active wake timers"));
    match &p.wake_timers {
        Some(t) if t.is_empty() => g = g.child(ui::weak("None", cx)),
        Some(t) => {
            g = g.children(
                t.iter()
                    .enumerate()
                    .map(|(i, x)| div().id(("wake-timer", i)).child(ui::label(x.clone()))),
            )
        }
        None => g = g.child(check_now("check-wake-timers", busy, cx)),
    }
    if !p.wakes.is_empty() {
        g = g.child(ui::collapse_header(
            WAKE_HISTORY,
            ui::label(format!("Wake history, last 2 weeks ({})", p.wakes.len())),
            history_open,
            cx,
        ));
        if history_open {
            let mut cells = Vec::new();
            for w in &p.wakes {
                cells.push(ui::label(format!("{} UTC", w.time)).into_any_element());
                cells.push(
                    ui::label(if w.fast_startup {
                        "Start-up from Fast Startup shutdown".to_string()
                    } else {
                        w.source.describe()
                    })
                    .into_any_element(),
                );
            }
            g = g.child(striped(2, cells, cx).pl_4());
        }
    }
    g
}

/// A striped grid without headings ([`Table`] always has them).
fn striped(cols: u16, cells: Vec<AnyElement>, cx: &App) -> Div {
    let stripe = cx.theme().muted;
    let per_row = cols as usize;
    ui::hug(
        div()
            .grid()
            .grid_cols_max_content(cols)
            .children(cells.into_iter().enumerate().map(|(i, c)| {
                div()
                    .id(i)
                    .px_2()
                    .py_0p5()
                    .when((i / per_row) % 2 == 1, |d| d.bg(stripe))
                    .child(c)
            })),
    )
}

/// "Needs administrator rights to read." and a button that reads them now.
fn check_now(id: &'static str, busy: bool, cx: &mut Context<OmniApp>) -> impl IntoElement {
    ui::row()
        .id(id)
        .child(ui::weak("Needs administrator rights to read.", cx))
        .child(
            Button::new(id)
                .small()
                .label("Check now")
                .tooltip(if elevated() {
                    "Read now"
                } else {
                    "Asks for administrator permission"
                })
                .disabled(busy)
                .on_click(cx.listener(|this, _, _, _| this.sampler.power_query_admin())),
        )
}

fn check_again(busy: bool, cx: &mut Context<OmniApp>) -> impl IntoElement {
    Button::new("check-requests-again")
        .small()
        .label("Check again")
        .disabled(busy)
        .on_click(cx.listener(|this, _, _, _| this.sampler.power_query_admin()))
}

fn requests(p: &PowerInfo, busy: bool, cx: &mut Context<OmniApp>) -> impl IntoElement {
    let g = group("What's keeping the PC awake", cx);
    match &p.requests {
        None => g.child(check_now("check-requests", busy, cx)),
        Some(r) if r.is_empty() => g
            .child(ui::label("Nothing: no app or driver is preventing sleep."))
            .child(check_again(busy, cx)),
        Some(r) => {
            let mut table = Table::new(&["Keeps awake", "Requested by", "Reason"]).striped();
            for (i, x) in r.iter().enumerate() {
                let kind = match x.kind.as_str() {
                    "DISPLAY" => "Display",
                    "SYSTEM" => "System",
                    "AWAYMODE" => "Away mode",
                    "EXECUTION" => "Execution",
                    k => k,
                };
                table.push_row([
                    ui::label(kind.to_string()).into_any_element(),
                    ui::with_tip(
                        ("request", i),
                        ui::label(format!("{} ({})", x.short_name(), x.origin.to_lowercase())),
                        x.name.clone(),
                    )
                    .into_any_element(),
                    ui::weak(x.reason.clone(), cx).into_any_element(),
                ]);
            }
            g.child(table.build(cx)).child(check_again(busy, cx))
        }
    }
}

/// Apply `action`, after a confirmation dialog if it is disruptive. The change runs on a
/// worker thread; its result arrives as a status bar message.
pub fn request(app: &mut OmniApp, action: Action, window: &mut Window, cx: &mut Context<OmniApp>) {
    let Some(text) = action.confirm_text() else {
        app.sampler.power_action(action);
        return;
    };
    let mut text = text.to_string();
    if action.needs_admin() && !elevated() {
        text.push_str("\n\nWindows will ask for administrator permission.");
    }
    ui::confirm(
        action.label(),
        text,
        "Apply",
        window,
        cx,
        move |this, _, _| this.sampler.power_action(action.clone()),
    );
}

/// Hover text of a button that applies `a`.
pub fn admin_hint(a: &Action) -> &'static str {
    if a.needs_admin() && !elevated() {
        "Asks for administrator permission"
    } else {
        "Applies the change now"
    }
}

/// A "Fix" button for a Health Check finding. `id` must be unique on the page.
pub fn fix_button(
    id: impl Into<ElementId>,
    action: &Action,
    cx: &mut Context<OmniApp>,
) -> AnyElement {
    let apply = action.clone();
    Button::new(id)
        .label(format!("Fix: {}", action.label()))
        .tooltip(admin_hint(action))
        .on_click(cx.listener(move |this, _, window, cx| request(this, apply.clone(), window, cx)))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::options;
    use crate::app::Tab;
    use crate::sampler::Snapshot;
    use crate::settings::Settings;
    use crate::tabs::page_tests::{assert_shows, go, open, page, snapshot, texts};
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{AppContext, TestAppContext};
    use omnihw_core::power::{AcDc, BALANCED, Capabilities, PowerInfo, Scheme, Setting};
    use omnihw_core::testkit::{desktop, laptop};
    use omnihw_core::win::PlanKind;

    #[test]
    fn options_keep_custom_values() {
        let o = options(Setting::Sleep, 420);
        assert!(o.contains(&420));
        assert_eq!(o.last(), Some(&0), "Never is last");
        assert!(o.windows(2).take(o.len() - 2).all(|w| w[0] < w[1]));
        assert_eq!(options(Setting::PcieAspm, 1), vec![0, 1, 2]);
        assert!(options(Setting::ProcessorMax, 42).contains(&42));
    }

    /// A laptop's power settings: two plans, Balanced active, Fast Startup on.
    fn power_info() -> PowerInfo {
        PowerInfo {
            schemes: vec![
                Scheme {
                    guid: BALANCED.into(),
                    name: "Balanced".into(),
                    kind: PlanKind::Balanced,
                },
                Scheme {
                    guid: "8c5e7fda-e8bf-4a96-9a85-a6e23a8c635c".into(),
                    name: "High performance".into(),
                    kind: PlanKind::HighPerformance,
                },
            ],
            active: Some(BALANCED.into()),
            values: vec![
                (
                    Setting::Sleep,
                    AcDc {
                        ac: Some(1800),
                        dc: Some(900),
                    },
                ),
                (
                    Setting::ProcessorMax,
                    AcDc {
                        ac: Some(100),
                        dc: Some(100),
                    },
                ),
            ],
            caps: Capabilities {
                battery: true,
                lid: true,
                hibernate: true,
                hiberfile: true,
                modern_standby: true,
                ..Default::default()
            },
            fast_startup: Some(true),
            wake_armed: vec!["HID Keyboard Device".into()],
            ..Default::default()
        }
    }

    #[gpui_kit::test]
    fn empty_state(cx: &mut TestAppContext) {
        let info = desktop();
        let shown = page(cx, &info, Snapshot::default(), Tab::Power);
        assert_shows(&shown, &["Power", "Refresh", "Reading power settings..."]);
    }

    #[gpui_kit::test]
    fn shows_plans_and_settings(cx: &mut TestAppContext) {
        let info = laptop();
        let snap = Snapshot {
            power: Some(power_info()),
            ..snapshot(&info)
        };
        let shown = page(cx, &info, snap, Tab::Power);
        assert_shows(
            &shown,
            &[
                "Balanced",
                "High performance",
                "Plugged in",
                "On battery",
                "Fast Startup",
                "Turn off",
                "Modern Standby (S0 low power idle)",
                "HID Keyboard Device",
                "Needs administrator rights to read.",
                "What's keeping the PC awake",
            ],
        );
    }

    #[gpui_kit::test]
    fn busy_shows_working(cx: &mut TestAppContext) {
        let info = desktop();
        let snap = Snapshot {
            power: Some(power_info()),
            power_busy: true,
            ..snapshot(&info)
        };
        let shown = page(cx, &info, snap, Tab::Power);
        assert_shows(&shown, &["Working...", "Balanced"]);
        assert!(!shown.iter().any(|t| t == "Refresh"));
    }

    #[gpui_kit::test]
    fn disruptive_change_asks_first(cx: &mut TestAppContext) {
        let info = laptop();
        let snap = Snapshot {
            power: Some(power_info()),
            ..snapshot(&info)
        };
        let (window, app) = open(cx, info, snap, Settings::default());
        go(cx, &app, Tab::Power);
        assert!(!texts(cx, window).iter().any(|t| t == "Apply"));
        // Turning Fast Startup off asks first; the dialog is never confirmed here. (The
        // dialog's title and text are not labels the tests can see, only its buttons.)
        cx.update_window(window, |_, window, cx| window.click("fast-startup", cx))
            .unwrap();
        assert_shows(&texts(cx, window), &["Apply", "Cancel"]);
        assert!(!cx.read(|cx| app.read(cx).sampler.snapshot().power_busy));
    }
}
