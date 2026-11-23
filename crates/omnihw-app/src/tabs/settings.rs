//! Settings page: appearance, startup, monitoring and file locations.
//!
//! Every control edits `app.settings` in its handler and then calls [`changed`], which hands
//! the new values to the sampler and the export module, applies theme and scale, and saves the
//! file shortly after.

use crate::app::OmniApp;
use crate::autostart;
use crate::folder_picker::FolderPicker;
use crate::heat::ORANGE;
use crate::pawnio::{self, Job};
use crate::settings::{INTERVALS_MS, NavStyle, PAGES, Settings, Theme, UI_SCALE_RANGE};
use crate::ui;
use crate::update::{RELEASES_URL, Status};
use gpui_kit::component::button::Button;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::link::Link;
use gpui_kit::component::radio::Radio;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{Disableable, Sizable, h_flex, v_flex};
use gpui_kit::{
    AnyElement, AppContext, Context, Div, Entity, InteractiveElement, IntoElement, ParentElement,
    SharedString, Styled, Subscription, Window, div, rems,
};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Edit buffers, errors, open dialogs and widget state of this page.
#[derive(Default)]
pub struct ViewState {
    /// Why the last "launch at sign-in" change failed (empty when it worked).
    autostart_err: String,
    /// Why the last data folder change failed.
    data_dir_err: String,
    /// The Windows folder dialog for the data folder (runs on its own thread).
    picker: FolderPicker,
    /// Where reports go, for which chosen folder, and when it was resolved (see [`report_dir`]).
    report_dir: Option<(Option<PathBuf>, PathBuf, Instant)>,
    /// The "Paste a folder path" field.
    data_dir_edit: Option<Entity<InputState>>,
    /// The UI scale slider, and its subscription.
    scale: Option<(Entity<SliderState>, Subscription)>,
    /// The scale shown while the slider is dragged (applied when it is released).
    scale_preview: Option<f32>,
    /// The field for a typed scale percentage, and its subscription.
    scale_edit: Option<(Entity<InputState>, Subscription)>,
}

/// A titled group box.
#[track_caller]
fn group(title: &str, cx: &gpui_kit::App) -> Div {
    ui::group_box(cx)
        .gap_2()
        .child(ui::strong(title.to_string()).text_lg().pb_1())
}

/// One labelled row of a group: the name on the left, the controls (and their notes) stacked
/// on the right.
#[track_caller]
fn field(name: &str, controls: impl IntoIterator<Item = AnyElement>) -> Div {
    h_flex()
        .items_start()
        .gap_4()
        .child(ui::label(name.to_string()).flex_none().w(rems(10.0)))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .items_start()
                .gap_1()
                .children(controls),
        )
}

/// Where reports go ([`crate::export::report_dir`]). Resolving it checks the file system, so the
/// answer is kept for a couple of seconds (or until the chosen folder changes) instead of being
/// re-checked on every redraw.
fn report_dir(state: &mut ViewState, chosen: &Option<PathBuf>) -> PathBuf {
    const FRESH: Duration = Duration::from_secs(2);
    if let Some((for_dir, dir, at)) = &state.report_dir
        && for_dir == chosen
        && at.elapsed() < FRESH
    {
        return dir.clone();
    }
    let dir = crate::export::report_dir();
    state.report_dir = Some((chosen.clone(), dir.clone(), Instant::now()));
    dir
}

/// Parse a typed UI-scale percentage ("120", "120%", "120 %") into a scale factor.
fn parse_percent(text: &str) -> Option<f64> {
    let v: f64 = text.trim().trim_end_matches('%').trim().parse().ok()?;
    Some(v / 100.0)
}

/// A scale from the slider or typed: within [`UI_SCALE_RANGE`], on its 5 % steps.
fn snap_scale(v: f32) -> f32 {
    let v = v.clamp(*UI_SCALE_RANGE.start(), *UI_SCALE_RANGE.end());
    (v * 20.0).round() / 20.0
}

/// A setting changed: pass the values the sampler and the exports use on, then apply theme and
/// scale and save the settings shortly.
fn changed(this: &mut OmniApp, window: &mut Window, cx: &mut Context<OmniApp>) {
    let s = &this.settings;
    this.sampler.set_interval_ms(s.sample_interval_ms);
    this.sampler.set_alert_notifications(s.alert_notifications);
    this.sampler.set_use_pawnio(s.use_pawnio);
    crate::export::set_data_dir(s.data_dir.clone());
    this.settings_changed(window, cx);
}

/// A check box for one on/off setting.
fn setting_checkbox(
    id: &'static str,
    label: &'static str,
    checked: bool,
    cx: &mut Context<OmniApp>,
    set: impl Fn(&mut Settings, bool) + 'static,
) -> AnyElement {
    Checkbox::new(id)
        .label(label)
        .checked(checked)
        .on_click(cx.listener(move |this, checked: &bool, window, cx| {
            set(&mut this.settings, *checked);
            changed(this, window, cx);
        }))
        .into_any_element()
}

/// Set the UI scale from a control other than the slider, and move the slider to it.
fn set_scale(this: &mut OmniApp, scale: f32, window: &mut Window, cx: &mut Context<OmniApp>) {
    this.settings.ui_scale = scale;
    this.views.settings.scale_preview = None;
    if let Some((slider, _)) = &this.views.settings.scale {
        slider.update(cx, |s, cx| s.set_value(scale, window, cx));
    }
    changed(this, window, cx);
}

/// The UI scale slider, created on first use. Dragging shows the new value; the scale is
/// applied when the slider is released, so the page doesn't resize under the pointer.
fn scale_slider(
    app: &mut OmniApp,
    window: &mut Window,
    cx: &mut Context<OmniApp>,
) -> Entity<SliderState> {
    if let Some((state, _)) = &app.views.settings.scale {
        return state.clone();
    }
    let state = cx.new(|_| {
        SliderState::new()
            .min(*UI_SCALE_RANGE.start())
            .max(*UI_SCALE_RANGE.end())
            .step(0.05)
            .default_value(app.settings.ui_scale)
    });
    let sub = cx.subscribe_in(
        &state,
        window,
        |this: &mut OmniApp, _, event: &SliderEvent, window, cx| match event {
            SliderEvent::Change(v) => {
                this.views.settings.scale_preview = Some(snap_scale(v.end()));
                cx.notify();
            }
            SliderEvent::Release(v) => {
                this.views.settings.scale_preview = None;
                let scale = snap_scale(v.end());
                if scale != this.settings.ui_scale {
                    this.settings.ui_scale = scale;
                    changed(this, window, cx);
                } else {
                    cx.notify();
                }
            }
        },
    );
    app.views.settings.scale = Some((state.clone(), sub));
    state
}

/// The field for typing a scale percentage; Enter applies it.
fn scale_field(
    app: &mut OmniApp,
    window: &mut Window,
    cx: &mut Context<OmniApp>,
) -> Entity<InputState> {
    if let Some((state, _)) = &app.views.settings.scale_edit {
        return state.clone();
    }
    let state = cx.new(|cx| InputState::new(window, cx).placeholder("Type %"));
    let sub = cx.subscribe_in(
        &state,
        window,
        |this: &mut OmniApp, state: &Entity<InputState>, event: &InputEvent, window, cx| {
            if let InputEvent::PressEnter { .. } = event
                && let Some(v) = parse_percent(&state.read(cx).value())
            {
                // Typed values are percentages too ("120", "120%", "120 %").
                state.update(cx, |s, cx| s.set_value("", window, cx));
                set_scale(this, snap_scale(v as f32), window, cx);
            }
        },
    );
    app.views.settings.scale_edit = Some((state.clone(), sub));
    state
}

fn appearance(app: &mut OmniApp, window: &mut Window, cx: &mut Context<OmniApp>) -> Div {
    let s = &app.settings;
    let themes = [
        (Theme::System, "Match Windows"),
        (Theme::Dark, "Dark"),
        (Theme::Light, "Light"),
        (Theme::Retro, "Retro (2000s)"),
    ];
    let current = themes.iter().position(|(t, _)| *t == s.theme);
    let theme = ui::dropdown(
        "theme",
        current.map_or("", |i| themes[i].1),
        themes.iter().map(|(_, n)| SharedString::from(*n)).collect(),
        current,
        cx,
        move |this, i, window, cx| {
            this.settings.theme = themes[i].0;
            changed(this, window, cx);
        },
    );

    let shown_scale = app.views.settings.scale_preview.unwrap_or(s.ui_scale);
    let slider = scale_slider(app, window, cx);
    let typed = scale_field(app, window, cx);
    let scale = ui::row()
        .child(div().w(rems(13.0)).child(Slider::new(&slider)))
        .child(ui::label(format!("{:.0} %", shown_scale * 100.0)).w(rems(3.5)))
        .child(Input::new(&typed).small().w(rems(5.5)))
        .child(
            Button::new("scale-reset")
                .small()
                .label("Reset")
                .on_click(cx.listener(|this, _, window, cx| set_scale(this, 1.0, window, cx))),
        );

    let s = &app.settings;
    let mut nav = Vec::new();
    for (style, name, hover, note) in [
        (
            NavStyle::Sidebar,
            "Sidebar",
            "The default. Arrow keys: Left/Right categories, Up/Down pages.",
            "Every page listed down the left side, sorted into categories.",
        ),
        (
            NavStyle::Tabs,
            "Tabs",
            "Two rows at the top. Arrow keys: Left/Right categories, Up/Down pages.",
            "Category tabs, with the open category's pages on a second row.",
        ),
        (
            NavStyle::Classic,
            "Classic",
            "One row of tabs. Arrow keys: Left/Right step through every page.",
            "Every page as its own tab (may wrap onto two lines in small windows).",
        ),
    ] {
        nav.push(
            Radio::new(("nav-style", style as usize))
                .label(name)
                .tooltip(hover)
                .checked(s.nav_style == style)
                .on_click(cx.listener(move |this, _: &bool, window, cx| {
                    this.settings.nav_style = style;
                    changed(this, window, cx);
                }))
                .into_any_element(),
        );
        nav.push(ui::weak(note, cx).into_any_element());
    }

    group("Appearance", cx)
        .child(field("Theme", [theme]))
        .child(field("UI scale", [scale.into_any_element()]))
        .child(field("Navigation bar", nav))
}

fn startup(app: &mut OmniApp, cx: &mut Context<OmniApp>) -> Div {
    let s = &app.settings;
    let pages: Vec<SharedString> = PAGES.iter().map(|(_, n)| SharedString::from(*n)).collect();
    let start = ui::dropdown(
        "start-page",
        s.start_page_name(),
        pages,
        PAGES.iter().position(|(k, _)| *k == s.start_page),
        cx,
        |this, i, window, cx| {
            this.settings.start_page = PAGES[i].0.to_string();
            changed(this, window, cx);
        },
    );

    // Launching at sign-in is a registry Run entry; the settings follow only when writing it
    // worked.
    let set_autostart = |this: &mut OmniApp,
                         launch: bool,
                         minimized: bool,
                         window: &mut Window,
                         cx: &mut Context<OmniApp>| {
        match autostart::set(launch, minimized) {
            Ok(()) => {
                this.settings.launch_at_startup = launch;
                this.settings.start_minimized = minimized;
                this.views.settings.autostart_err.clear();
                changed(this, window, cx);
            }
            Err(e) => {
                this.views.settings.autostart_err = e;
                cx.notify();
            }
        }
    };
    let mut windows = vec![
        Checkbox::new("launch-at-startup")
            .label("Launch Omni-HW when I sign in to Windows")
            .checked(s.launch_at_startup)
            .on_click(cx.listener(move |this, launch: &bool, window, cx| {
                let minimized = this.settings.start_minimized;
                set_autostart(this, *launch, minimized, window, cx);
            }))
            .into_any_element(),
        Checkbox::new("start-minimized")
            .label("Start minimized")
            .checked(s.start_minimized)
            .disabled(!s.launch_at_startup)
            .on_click(cx.listener(move |this, minimized: &bool, window, cx| {
                let launch = this.settings.launch_at_startup;
                set_autostart(this, launch, *minimized, window, cx);
            }))
            .into_any_element(),
    ];
    let err = &app.views.settings.autostart_err;
    if !err.is_empty() {
        windows.push(ui::colored(err.clone(), ORANGE, cx).into_any_element());
    }
    windows.push(
        ui::weak(
            "Launches at sign-in never show the administrator prompt.",
            cx,
        )
        .into_any_element(),
    );

    let tray = [
        setting_checkbox(
            "minimize-to-tray",
            "Minimize to the system tray",
            s.minimize_to_tray,
            cx,
            |s, v| s.minimize_to_tray = v,
        ),
        ui::weak("Minimizing hides Omni-HW next to the clock; click its icon to bring it back. Monitoring and alerts keep running.", cx).into_any_element(),
    ];
    let admin = [
        setting_checkbox(
            "ask-admin",
            "Ask for administrator rights at start when SATA drives are present",
            s.ask_admin_at_start,
            cx,
            |s, v| s.ask_admin_at_start = v,
        ),
        ui::weak(
            "Reading SATA drive health (SMART) needs administrator rights. NVMe drives don't.",
            cx,
        )
        .into_any_element(),
    ];

    group("Startup", cx)
        .child(field("Start page", [start]))
        .child(field("Windows startup", windows))
        .child(field("System tray", tray))
        .child(field("Administrator", admin))
}

fn monitoring(app: &mut OmniApp, cx: &mut Context<OmniApp>) -> Div {
    let s = &app.settings;
    let interval = ui::dropdown(
        "sample-interval",
        format!("{} ms", s.sample_interval_ms),
        INTERVALS_MS
            .iter()
            .map(|ms| SharedString::from(format!("{ms} ms")))
            .collect(),
        INTERVALS_MS
            .iter()
            .position(|&ms| ms == s.sample_interval_ms),
        cx,
        |this, i, window, cx| {
            this.settings.sample_interval_ms = INTERVALS_MS[i];
            changed(this, window, cx);
        },
    );
    let alerts = setting_checkbox(
        "alert-notifications",
        "Show Windows notifications for alerts",
        s.alert_notifications,
        cx,
        |s, v| s.alert_notifications = v,
    );
    group("Monitoring", cx)
        .child(field(
            "Sampling interval",
            [
                interval,
                ui::weak("How often sensors are read. Shorter is more detailed but uses a little more CPU.", cx).into_any_element(),
            ],
        ))
        .child(field(
            "Alert notifications",
            [
                alerts,
                ui::weak("Fired alerts always appear in the app; this controls the Windows notification and taskbar flash.", cx).into_any_element(),
            ],
        ))
}

/// Use `dir` as the data folder (from the dialog or a typed path).
fn set_data_dir(this: &mut OmniApp, dir: PathBuf, window: &mut Window, cx: &mut Context<OmniApp>) {
    this.settings.data_dir = Some(dir);
    if let Some(edit) = &this.views.settings.data_dir_edit {
        edit.update(cx, |s, cx| s.set_value("", window, cx));
    }
    this.views.settings.data_dir_err.clear();
    changed(this, window, cx);
}

fn files(app: &mut OmniApp, window: &mut Window, cx: &mut Context<OmniApp>) -> Div {
    let current = report_dir(&mut app.views.settings, &app.settings.data_dir);
    let edit = app
        .views
        .settings
        .data_dir_edit
        .get_or_insert_with(|| {
            cx.new(|cx| InputState::new(window, cx).placeholder("Paste a folder path"))
        })
        .clone();
    let picking = app.views.settings.picker.is_open();
    let start = current.clone();
    let opened = current.clone();
    let mut out = group("Files", cx)
        .child(
            ui::wrap_row()
                .child(ui::label("Recordings and reports are saved in:"))
                .child(ui::strong(current.display().to_string())),
        )
        .child(
            ui::row()
                .child(Input::new(&edit).small().w(rems(22.5)))
                .child(
                    Button::new("data-dir-browse")
                        .small()
                        .label("Browse…")
                        .tooltip("Choose a folder")
                        .disabled(picking)
                        .on_click(cx.listener(move |this, _, _, _| {
                            this.views
                                .settings
                                .picker
                                .open(&this.repaint, start.clone());
                        })),
                )
                .child(
                    Button::new("data-dir-apply")
                        .small()
                        .label("Apply")
                        .on_click(cx.listener(|this, _, window, cx| {
                            let Some(edit) = this.views.settings.data_dir_edit.clone() else {
                                return;
                            };
                            let text = edit.read(cx).value().trim().to_string();
                            let p = PathBuf::from(&text);
                            if text.is_empty() {
                                this.views.settings.data_dir_err =
                                    "Enter a folder path first.".into();
                                cx.notify();
                            } else if !p.is_dir() {
                                this.views.settings.data_dir_err =
                                    format!("Folder not found: {}", p.display());
                                cx.notify();
                            } else {
                                set_data_dir(this, p, window, cx);
                            }
                        })),
                ),
        )
        .child(
            ui::row()
                .child(
                    Button::new("data-dir-reset")
                        .small()
                        .label("Reset to Documents")
                        .disabled(app.settings.data_dir.is_none())
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.settings.data_dir = None;
                            changed(this, window, cx);
                        })),
                )
                .child(
                    Button::new("data-dir-open")
                        .small()
                        .label("Open folder")
                        .on_click(cx.listener(move |_, _, _, _| {
                            crate::export::open_in_explorer(&opened, false);
                        })),
                ),
        );
    let err = &app.views.settings.data_dir_err;
    if !err.is_empty() {
        out = out.child(ui::colored(err.clone(), ORANGE, cx));
    }
    out
}

fn health_check(app: &mut OmniApp, cx: &mut Context<OmniApp>) -> Div {
    let s = &app.settings;
    let n = crate::dismissed::count();
    group("Health Check", cx)
        .child(setting_checkbox(
            "show-gaming",
            "Show gaming suggestions",
            s.show_gaming,
            cx,
            |s, v| s.show_gaming = v,
        ))
        .child(ui::weak("When off, the Gaming category and its suggestions are hidden in Health Check and on Overview.", cx))
        .child(ui::space(4.0))
        .child(setting_checkbox(
            "remember-dismissed",
            "Keep dismissed issues dismissed after restarting Omni-HW",
            s.remember_dismissed,
            cx,
            |s, v| s.remember_dismissed = v,
        ))
        .child(ui::weak(
            "When off, dismissed issues come back the next time Omni-HW starts.",
            cx,
        ))
        .child(
            div().child(
                Button::new("restore-dismissed")
                    .small()
                    .label(format!("Restore dismissed issues ({n})"))
                    .disabled(n == 0)
                    .on_click(cx.listener(|_, _, _, cx| {
                        crate::dismissed::restore_all();
                        cx.notify();
                    })),
            ),
        )
}

/// The optional extra sensors (PawnIO driver + Omni-HW sensor service): state, setup and removal
/// buttons, the on/off setting, and the driver's licences.
fn pawnio_group(app: &mut OmniApp, cx: &mut Context<OmniApp>) -> Div {
    use omnihw_core::sensor_service::ServiceState;
    let mut out = group("Extra sensors (PawnIO)", cx)
        .child(ui::label(
            "Per-core CPU temperatures, CPU voltage, and motherboard fan speeds, temperatures and voltages, read through PawnIO, a small signed open-source driver used by several hardware monitors. It's optional: everything else works without it.",
        ))
        .child(ui::weak(
            "Setup installs PawnIO and a small Omni-HW sensor service, which is the only part that talks to the driver. It only reads sensors (it never changes clocks, voltages or fan speeds) and only hands readings out, so Omni-HW itself doesn't need administrator rights.",
            cx,
        ))
        .child(ui::space(4.0));

    let job = app.pawnio.job();
    let status = app.sampler.snapshot().sensor_service.clone();
    let pawnio = status
        .as_ref()
        .and_then(|st| st.pawnio_version.clone())
        .map(|v| format!("PawnIO {v}"));
    // (offer setup / repair, offer removal)
    let (setup_label, can_remove) = match status.as_ref().map(|st| &st.state) {
        None => {
            out = out.child(ui::weak("Checking...", cx));
            (None, false)
        }
        Some(ServiceState::NotSetUp) => {
            out = out.child(ui::label(match &pawnio {
                Some(p) => format!(
                    "{p} is installed (by another app), but Omni-HW's sensor service isn't set up."
                ),
                None => "Not set up.".to_string(),
            }));
            (Some("Set up extra sensors..."), false)
        }
        Some(ServiceState::NotRunning(e)) => {
            out = out.child(ui::colored(
                format!("The sensor service isn't answering: {e}."),
                ORANGE,
                cx,
            ));
            (Some("Repair..."), true)
        }
        Some(ServiceState::Outdated { version }) => {
            out = out.child(ui::colored(
                format!("The sensor service is from Omni-HW {version} and needs updating."),
                ORANGE,
                cx,
            ));
            (Some("Update sensor service..."), true)
        }
        Some(ServiceState::Ready {
            sources, errors, ..
        }) => {
            let via = pawnio.unwrap_or_else(|| "PawnIO".into());
            out = out.child(if !app.settings.use_pawnio {
                ui::label(format!("Set up ({via}), but turned off below."))
            } else if sources.is_empty() {
                ui::colored(
                    format!(
                        "The sensor service found no sensors: {}.",
                        errors.join("; ")
                    ),
                    ORANGE,
                    cx,
                )
            } else {
                ui::label(format!("Working ({via}): {}.", sources.join(", ")))
            });
            (None, true)
        }
    };

    let busy = matches!(job, Job::Running(_));
    let mut actions = ui::wrap_row();
    if let Some(label) = setup_label {
        actions = actions.child(
            Button::new("pawnio-setup")
                .small()
                .label(label)
                .tooltip("Installs PawnIO (if needed) and the Omni-HW sensor service. Windows asks for administrator permission once.")
                .disabled(busy)
                .on_click(cx.listener(|this, _, _, _| this.pawnio.start(&this.repaint, true))),
        );
    }
    if can_remove {
        actions = actions.child(
            Button::new("pawnio-remove")
                .small()
                .label("Remove extra sensors...")
                .tooltip("Removes the sensor service, and PawnIO too if Omni-HW installed it. Asks for administrator permission.")
                .disabled(busy)
                .on_click(cx.listener(|this, _, _, _| this.pawnio.start(&this.repaint, false))),
        );
    }
    match &job {
        Job::Idle => {}
        Job::Running(what) => {
            actions = actions
                .child(Spinner::new().small())
                .child(ui::label(format!("{what}...")));
        }
        Job::Done(Ok(msg)) => actions = actions.child(ui::label(msg.clone())),
        Job::Done(Err(e)) => actions = actions.child(ui::colored(e.clone(), ORANGE, cx)),
    }
    out = out.child(actions).child(setting_checkbox(
        "use-pawnio",
        "Use extra sensors",
        app.settings.use_pawnio,
        cx,
        |s, v| s.use_pawnio = v,
    ));

    let key = "settings/pawnio-licences";
    let open = ui::is_open(app, key, false);
    out = out.child(ui::collapse_header(
        key,
        ui::label("Licences and source code"),
        open,
        cx,
    ));
    if open {
        let mut body = ui::column()
            .pl_4()
            .child(
                ui::wrap_row()
                    .gap_4()
                    .child(link("pawnio-site", "pawnio.eu", pawnio::SITE_URL))
                    .child(link("pawnio-driver-src", "Driver source", pawnio::DRIVER_SOURCE_URL))
                    .child(link("pawnio-setup-src", "Installer source", pawnio::SETUP_SOURCE_URL))
                    .child(link("pawnio-modules-src", "Modules source", pawnio::MODULES_SOURCE_URL)),
            )
            .child(ui::weak(
                "Omni-HW includes the official, unmodified PawnIO installer (GNU GPL 2 with an exception for programs that use its device interface, as Omni-HW does) and PawnIO modules (GNU LGPL 2.1).",
                cx,
            ));
        for (key, title, text) in [
            (
                "settings/pawnio-driver-licence",
                "PawnIO driver licence (GPL 2)",
                pawnio::DRIVER_LICENSE,
            ),
            (
                "settings/pawnio-modules-licence",
                "PawnIO modules licence (LGPL 2.1)",
                pawnio::MODULES_LICENSE,
            ),
        ] {
            let open = ui::is_open(app, key, false);
            body = body.child(ui::collapse_header(key, ui::label(title), open, cx));
            if open {
                body = body.child(
                    v_flex()
                        .id(key)
                        .ml_4()
                        .max_h(rems(16.25))
                        .overflow_y_scrollbar()
                        .child(ui::mono(text, cx)),
                );
            }
        }
        out = out.child(body);
    }
    out
}

/// A link that opens `url` in the user's browser.
fn link(id: &'static str, text: &'static str, url: &'static str) -> AnyElement {
    Link::new(id)
        .href(url)
        .child(ui::label(text))
        .into_any_element()
}

fn updates(app: &mut OmniApp, cx: &mut Context<OmniApp>) -> Div {
    let status = app.updates.status();
    let checking = status == Status::Checking;
    let version = env!("CARGO_PKG_VERSION");
    let mut row = ui::wrap_row()
        .child(
            Button::new("check-updates")
                .small()
                .label("Check for updates")
                .disabled(checking)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.updates.start(&this.repaint);
                    cx.notify();
                })),
        )
        // Opens the latest release notes in the browser; the app itself sends nothing.
        .child(
            Button::new("whats-new")
                .small()
                .label("What's new")
                .tooltip("Open the latest release notes on GitHub in your browser")
                .on_click(cx.listener(|_, _, _, cx| cx.open_url(RELEASES_URL))),
        );
    row = match status {
        Status::Idle => row.child(ui::weak(format!("You have version {version}."), cx)),
        Status::Checking => row
            .child(Spinner::new().small())
            .child(ui::label("Checking GitHub...")),
        Status::UpToDate => row.child(ui::label(format!(
            "Omni-HW {version} is the latest version."
        ))),
        Status::Available(latest) => row
            .child(ui::strong(format!(
                "Version {latest} is available (you have {version})."
            )))
            .child(link(
                "download-page",
                "Open the download page",
                RELEASES_URL,
            )),
        Status::AheadOfRelease(latest) => row.child(ui::label(format!(
            "You have {version}, newer than the latest release ({latest})."
        ))),
        Status::Failed(e) => row.child(ui::colored(e, ORANGE, cx)),
    };
    group("Updates", cx)
        .child(row)
        .child(setting_checkbox(
            "check-updates-at-start",
            "Check for updates when Omni-HW starts",
            app.settings.check_updates_at_start,
            cx,
            |s, v| s.check_updates_at_start = v,
        ))
        .child(ui::weak(
            "Update checks are Omni-HW's only network request and happen only when you click the button or turn this on. \
             They ask GitHub for the latest release's version number; nothing about your PC is sent.",
            cx,
        ))
}

pub fn show(app: &mut OmniApp, window: &mut Window, cx: &mut Context<OmniApp>) -> AnyElement {
    // Results of the folder dialog and the PawnIO setup, which finish on their own threads
    // (each wakes the UI). Applied after this redraw, as a settings change.
    if let Some(p) = app.views.settings.picker.take() {
        cx.defer_in(window, move |this, window, cx| {
            set_data_dir(this, p, window, cx)
        });
    }
    if let Some(set_up) = app.pawnio.take_finished() {
        // Turn the readings on after a successful setup, and re-check the service either way.
        app.sampler.refresh_sensor_service();
        if set_up {
            cx.defer_in(window, |this, window, cx| {
                this.settings.use_pawnio = true;
                changed(this, window, cx);
            });
        }
    }

    let mut page = ui::column()
        .child(ui::page_title("Settings"))
        .child(ui::weak(
            "Changes apply immediately and are saved automatically.",
            cx,
        ))
        .child(ui::space(8.0))
        .child(appearance(app, window, cx))
        .child(startup(app, cx))
        .child(monitoring(app, cx))
        .child(files(app, window, cx))
        .child(health_check(app, cx))
        .child(pawnio_group(app, cx))
        .child(updates(app, cx));
    if let Some(p) = Settings::path() {
        page = page.child(ui::weak(
            format!("Settings are stored in {}", p.display()),
            cx,
        ));
    }
    page.into_any_element()
}

#[cfg(test)]
mod tests {
    use crate::app::Tab;
    use crate::settings::Settings;
    use crate::tabs::page_tests::{assert_shows, go, machines, open, page, snapshot, texts};
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{AppContext, TestAppContext};

    #[test]
    fn percent_parsing() {
        assert_eq!(super::parse_percent("120"), Some(1.2));
        assert_eq!(super::parse_percent(" 85 % "), Some(0.85));
        assert_eq!(super::parse_percent("150%"), Some(1.5));
        assert_eq!(super::parse_percent("abc"), None);
    }

    #[test]
    fn scale_snaps_to_steps_and_range() {
        assert_eq!(super::snap_scale(1.2300001), 1.25);
        assert_eq!(super::snap_scale(5.0), 2.0);
        assert_eq!(super::snap_scale(0.1), 0.75);
    }

    #[gpui_kit::test]
    fn settings_page(cx: &mut TestAppContext) {
        for info in machines() {
            let shown = page(cx, &info, snapshot(&info), Tab::Settings);
            assert_shows(
                &shown,
                &[
                    "Settings",
                    "Appearance",
                    "Match Windows",
                    "100 %",
                    "Sidebar",
                    "Start page",
                    "Launch Omni-HW when I sign in to Windows",
                    "1000 ms",
                    "Recordings and reports are saved in:",
                    "Restore dismissed issues",
                    "Extra sensors (PawnIO)",
                    "Checking...",
                    "Check for updates",
                    "What's new",
                ],
            );
        }
        let info = machines()[0].clone();
        let shown = page(cx, &info, Default::default(), Tab::Settings);
        assert_shows(&shown, &["Settings", "Check for updates"]);
    }

    /// A harmless check box changes the in-memory setting. The click also schedules the
    /// delayed settings save, whose timer only fires when the test clock is advanced (it never
    /// is here: `run_until_parked` doesn't move it), so the user's settings file is not
    /// written.
    #[gpui_kit::test]
    fn show_gaming_checkbox(cx: &mut TestAppContext) {
        let info = machines()[0].clone();
        let (window, app) = open(cx, info.clone(), snapshot(&info), Settings::default());
        go(cx, &app, Tab::Settings);
        texts(cx, window);
        assert!(cx.read(|cx| app.read(cx).settings.show_gaming));
        cx.update_window(window, |_, window, cx| window.click("show-gaming", cx))
            .unwrap();
        assert!(!cx.read(|cx| app.read(cx).settings.show_gaming));
    }
}
