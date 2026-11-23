//! The main window: navigation (sidebar, tabs or classic bar), the actions bar, the status
//! bar and the open page.
//!
//! [`OmniApp`] is the one gpui view. It redraws only when it is notified: by the sampler and
//! the worker threads through [`Repaint`] (new data), or by a click or key press handled here
//! or in a page. Pages are functions in [`crate::tabs`] that build their elements from the
//! latest snapshot.

use crate::nav::{self, Arrow, GROUPS};
use crate::repaint::Repaint;
use crate::sampler::Sampler;
use crate::settings::{NavStyle, Settings};
use crate::{export, heat, tabs, ui};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{
    ActiveTheme, Icon, IconName, Selectable, Sizable, StyledExt, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AnyElement, App, ClickEvent, ClipboardItem, Context, FocusHandle, FontWeight,
    InteractiveElement, IntoElement, KeyBinding, ParentElement, Render, StatefulInteractiveElement,
    Styled, WeakEntity, Window, div, px, rems,
};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    Overview,
    Health,
    Cpu,
    Caches,
    Mainboard,
    Memory,
    Gpu,
    Storage,
    Network,
    LanSecurity,
    Power,
    Sensors,
    Recording,
    Alerts,
    EventLog,
    About,
    Settings,
}

impl Tab {
    pub const ALL: [Tab; 16] = [
        Tab::Overview,
        Tab::Health,
        Tab::Cpu,
        Tab::Caches,
        Tab::Mainboard,
        Tab::Memory,
        Tab::Gpu,
        Tab::Storage,
        Tab::Network,
        Tab::LanSecurity,
        Tab::Power,
        Tab::Sensors,
        Tab::Recording,
        Tab::Alerts,
        Tab::EventLog,
        Tab::About,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Tab::Overview => "Overview",
            Tab::Health => "Health Check",
            Tab::Cpu => "CPU",
            Tab::Caches => "Caches",
            Tab::Mainboard => "Mainboard",
            Tab::Memory => "Memory",
            Tab::Gpu => "GPU",
            Tab::Storage => "Storage",
            Tab::Network => "Network",
            Tab::LanSecurity => "LAN Security Report",
            Tab::Power => "Power",
            Tab::Sensors => "Sensors",
            Tab::Recording => "Recording",
            Tab::Alerts => "Alerts",
            Tab::EventLog => "Event Log",
            Tab::About => "About",
            Tab::Settings => "Settings",
        }
    }

    /// Leading icon in the sidebar (all in the bundled default icon set).
    pub fn icon(self) -> IconName {
        match self {
            Tab::Overview => IconName::LayoutDashboard,
            Tab::Health => IconName::Heart,
            Tab::Cpu => IconName::Cpu,
            Tab::Caches => IconName::GalleryVerticalEnd,
            Tab::Mainboard => IconName::Frame,
            Tab::Memory => IconName::MemoryStick,
            Tab::Gpu => IconName::Palette,
            Tab::Storage => IconName::HardDrive,
            Tab::Network => IconName::Globe,
            Tab::LanSecurity => IconName::Network,
            Tab::Power => IconName::BatteryCharging,
            Tab::Sensors => IconName::ChartPie,
            Tab::Recording => IconName::Play,
            Tab::Alerts => IconName::Bell,
            Tab::EventLog => IconName::FileText,
            Tab::About => IconName::Info,
            Tab::Settings => IconName::Settings,
        }
    }

    /// Stable key used in settings (e.g. the start page).
    pub fn key(self) -> &'static str {
        match self {
            Tab::Overview => "overview",
            Tab::Health => "health",
            Tab::Cpu => "cpu",
            Tab::Caches => "caches",
            Tab::Mainboard => "mainboard",
            Tab::Memory => "memory",
            Tab::Gpu => "gpu",
            Tab::Storage => "storage",
            Tab::Network => "network",
            Tab::LanSecurity => "lansecurity",
            Tab::Power => "power",
            Tab::Sensors => "sensors",
            Tab::Recording => "recording",
            Tab::Alerts => "alerts",
            Tab::EventLog => "eventlog",
            Tab::About => "about",
            Tab::Settings => "settings",
        }
    }

    pub fn from_key(key: &str) -> Option<Tab> {
        Tab::ALL
            .into_iter()
            .chain([Tab::Settings])
            .find(|t| t.key() == key)
    }
}

gpui_kit::actions!(omnihw, [NavLeft, NavRight, NavUp, NavDown]);

/// Key context of the main window: the arrow keys below apply inside it, except where a
/// focused widget (text field, slider, menu) binds them itself.
const KEY_CONTEXT: &str = "OmniHW";

/// Bind the arrow keys to page navigation. Call once at startup.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("left", NavLeft, Some(KEY_CONTEXT)),
        KeyBinding::new("right", NavRight, Some(KEY_CONTEXT)),
        KeyBinding::new("up", NavUp, Some(KEY_CONTEXT)),
        KeyBinding::new("down", NavDown, Some(KEY_CONTEXT)),
    ]);
}

const STATUS_DURATION: Duration = Duration::from_secs(6);
/// How long after the last settings change the file is written, so dragging the UI scale
/// slider doesn't rewrite it on every step.
const SAVE_DELAY: Duration = Duration::from_millis(400);
/// Hover text of the "running as administrator" indicator, in every navigation style.
const ADMIN_HOVER: &str = "Omni-HW has administrator rights: you approved its UAC prompt or \
                           started it as administrator.";
/// Width of the navigation sidebar, in rems.
const SIDEBAR_WIDTH: f32 = 13.5;
/// Padding around a page, left and right together, in rems (the page frame's `p_3`).
const PAGE_PADDING: f32 = 1.5;

/// Width a page has to lay itself out in, in rems: the window without the sidebar and the
/// page padding. Pages that switch between a wide and a narrow layout use it (an element
/// can't ask for its own width while it is being built).
pub fn page_width_rems(nav_style: NavStyle, window: &Window) -> f32 {
    let beside = match nav_style {
        NavStyle::Sidebar => SIDEBAR_WIDTH + PAGE_PADDING,
        NavStyle::Tabs | NavStyle::Classic => PAGE_PADDING,
    };
    window.viewport_size().width / window.rem_size() - beside
}

pub struct OmniApp {
    /// The open page. Change it with [`OmniApp::open`], which also runs the page's on-open reads.
    pub tab: Tab,
    /// Last page used in each navigation category, for Left/Right arrows.
    nav_memory: nav::NavMemory,
    /// Persisted user settings (theme, navigation style, start page, ...). After changing
    /// them call [`OmniApp::settings_changed`].
    pub settings: Settings,
    pub sampler: Sampler,
    /// Wakes this view from worker threads (handed to the update check, folder picker, ...).
    pub repaint: Repaint,
    /// Transient message shown in the status bar (e.g. where a report was saved).
    status: Option<(String, Instant)>,
    /// Per-page view state (selections, filters, edit buffers).
    pub views: tabs::Views,
    /// In-app documentation window.
    pub help: crate::help::HelpState,
    pub updates: crate::update::UpdateChecker,
    pub pawnio: crate::pawnio::Installer,
    /// Health Check results, shared by the Health tab and the Overview banner.
    pub health: tabs::health::HealthCache,
    /// Bumped by every settings change; a delayed save only writes if no later change came.
    settings_rev: u64,
    settings_unsaved: bool,
    focus: FocusHandle,
}

impl OmniApp {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut settings = Settings::load();
        // The Run entry is the source of truth (it may have been removed outside Omni-HW).
        settings.launch_at_startup = crate::autostart::is_enabled();
        crate::export::set_data_dir(settings.data_dir.clone());
        crate::dismissed::load(&settings.dismissed);

        // Redraw when a worker thread has new data. One pending wake-up at most, so idle
        // costs nothing.
        let (repaint, wake) = Repaint::channel();
        cx.spawn(async move |this, cx| {
            while wake.recv().await.is_ok() {
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        })
        .detach();

        crate::tray::start(repaint.clone());
        crate::tray::set_icon_visible(settings.minimize_to_tray);
        let sampler = Sampler::start(repaint.clone(), settings.use_pawnio);
        sampler.set_interval_ms(settings.sample_interval_ms);
        sampler.set_alert_notifications(settings.alert_notifications);
        if let Some(hwnd) = window_hwnd(window) {
            sampler.set_window_handle(hwnd);
            crate::tray::set_main_window(hwnd);
        }
        let updates = crate::update::UpdateChecker::default();
        if settings.check_updates_at_start {
            updates.start(&repaint);
        }

        // Follow Windows' light/dark setting while the theme is "System".
        cx.observe_window_appearance(window, |this, window, cx| {
            crate::theme::apply(&this.settings, Some(window), cx);
        })
        .detach();
        // Minimizing deactivates the window: that is when it may go to the tray.
        cx.observe_window_activation(window, |this, _window, _cx| this.minimized_to_tray())
            .detach();
        cx.observe_window_bounds(window, |this, _window, _cx| this.minimized_to_tray())
            .detach();
        // Started with `--minimized` (launch at startup).
        if crate::autostart::launch_flags().1 {
            window.minimize_window();
        }

        Self::build(settings, sampler, repaint, updates, window, cx)
    }

    /// The app over a fake sampler, for page tests: no sampling thread, tray icon or settings
    /// file.
    #[cfg(test)]
    pub fn for_test(
        settings: Settings,
        sampler: Sampler,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let updates = Default::default();
        Self::build(settings, sampler, Repaint::detached(), updates, window, cx)
    }

    fn build(
        settings: Settings,
        sampler: Sampler,
        repaint: Repaint,
        updates: crate::update::UpdateChecker,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        crate::theme::apply(&settings, Some(window), cx);
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let mut app = OmniApp {
            tab: Tab::Overview,
            nav_memory: Default::default(),
            settings,
            sampler,
            repaint,
            status: None,
            views: Default::default(),
            help: Default::default(),
            updates,
            pawnio: Default::default(),
            health: Default::default(),
            settings_rev: 0,
            settings_unsaved: false,
            focus,
        };
        let start = Tab::from_key(&app.settings.start_page).unwrap_or(Tab::Overview);
        app.open(start, cx);
        app
    }

    /// Switch to `tab`, starting the reads a page wants when it is opened.
    pub fn open(&mut self, tab: Tab, cx: &mut Context<Self>) {
        let from = std::mem::replace(&mut self.tab, tab);
        self.nav_memory.visit(tab);
        // Re-read power devices and wake history whenever the Power page is opened.
        if tab == Tab::Power && from != Tab::Power {
            self.sampler.refresh_power();
        }
        // Read network adapters often (and in full right away) while the Network page is open.
        self.sampler.set_network_page(tab == Tab::Network);
        if tab == Tab::Network && from != Tab::Network {
            self.sampler.refresh_network();
        }
        // Read the event logs the first time the Event Log page is opened (Refresh re-reads).
        if tab == Tab::EventLog {
            let unread = {
                let snap = self.sampler.snapshot();
                snap.event_log.is_none() && !snap.event_log_busy
            };
            if unread {
                self.sampler.read_event_log();
            }
        }
        cx.notify();
    }

    /// Show `msg` in the status bar for a few seconds.
    pub fn set_status(&mut self, msg: impl Into<String>, cx: &mut Context<Self>) {
        self.status = Some((msg.into(), Instant::now()));
        cx.notify();
        // One redraw when the message expires; nothing runs in between.
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(STATUS_DURATION).await;
            let _ = this.update(cx, |this, cx| {
                if this
                    .status
                    .as_ref()
                    .is_some_and(|(_, at)| at.elapsed() >= STATUS_DURATION)
                {
                    this.status = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// The settings were edited: apply theme and scale now, and save them to disk shortly.
    pub fn settings_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        crate::theme::apply(&self.settings, Some(window), cx);
        crate::tray::set_icon_visible(self.settings.minimize_to_tray);
        self.save_settings_soon(cx);
        cx.notify();
    }

    /// Write the settings file once no further change arrives for [`SAVE_DELAY`].
    pub fn save_settings_soon(&mut self, cx: &mut Context<Self>) {
        // Page tests change settings in memory only: they must never write the user's file.
        self.settings_unsaved = !cfg!(test);
        self.settings_rev += 1;
        let rev = self.settings_rev;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SAVE_DELAY).await;
            let _ = this.update(cx, |this, cx| {
                if this.settings_rev == rev {
                    this.save_settings_now(cx);
                }
            });
        })
        .detach();
    }

    fn save_settings_now(&mut self, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.settings_unsaved)
            && let Err(e) = self.settings.save()
        {
            self.set_status(e, cx);
        }
    }

    /// Health Check dismissals changed: keep them in the settings (saved to disk only when
    /// "remember dismissed" is on).
    fn sync_dismissed(&mut self, cx: &mut Context<Self>) {
        // The dismissed set is process-wide: a page test drawing a frame must not take the
        // changes another test is checking for.
        if cfg!(test) {
            return;
        }
        if let Some(keys) = crate::dismissed::take_changes() {
            self.settings.dismissed = keys;
            if self.settings.remember_dismissed {
                self.save_settings_soon(cx);
            }
        }
    }

    /// Results of power and network changes, which finish on worker threads: show them in
    /// the status bar.
    fn worker_messages(&mut self, cx: &mut Context<Self>) {
        let msg = {
            let mut snap = self.sampler.snapshot();
            snap.power_msg.take().or_else(|| snap.net_msg.take())
        };
        if let Some(msg) = msg {
            self.set_status(msg, cx);
        }
    }

    /// With "minimize to tray" on, a minimized window leaves the taskbar for the tray.
    fn minimized_to_tray(&mut self) {
        if self.settings.minimize_to_tray && crate::tray::main_window_minimized() {
            crate::tray::hide_main_window();
        }
    }

    /// The window is closing: remove the tray icon and write unsaved settings.
    pub fn shutdown(&mut self) {
        crate::tray::shutdown();
        if std::mem::take(&mut self.settings_unsaved) {
            let _ = self.settings.save();
        }
    }

    /// Arrow keys switch pages (see [`nav`]). They only reach here when no focused widget
    /// (text field, slider, menu) uses them. Keys that don't move are left for the page.
    fn arrow(&mut self, arrow: Arrow, cx: &mut Context<Self>) {
        if let Some(next) = nav::target(self.settings.nav_style, self.tab, arrow, &self.nav_memory)
        {
            self.open(next, cx);
        } else if self.tab == Tab::Health {
            tabs::health::arrow_key(self, arrow, cx);
        }
    }

    /// Firing alert count and recording time, read in one short lock.
    fn bar_state(&self) -> (usize, Option<f64>) {
        let snap = self.sampler.snapshot();
        let elapsed = snap.recording.as_ref().map(|r| r.rec.duration_secs());
        (tabs::alerts::firing_count(&snap), elapsed)
    }

    fn save_html_report(&mut self, cx: &mut Context<Self>) {
        let Some(info) = self.sampler.info() else {
            return;
        };
        let msg = match export::save_html(info, &self.sampler.snapshot()) {
            Ok(p) => format!("Saved {}", p.display()),
            Err(e) => e,
        };
        self.set_status(msg, cx);
    }

    fn copy_text_report(&mut self, cx: &mut Context<Self>) {
        let Some(info) = self.sampler.info() else {
            return;
        };
        let text = export::text(info, &self.sampler.snapshot());
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.set_status("Report copied to clipboard", cx);
    }

    // ---- Actions (right-hand side of every bar) ---------------------------------------------

    /// Red "N ALERTS" badge, shown only while alerts fire.
    fn alerts_badge(&self, firing: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        if firing == 0 {
            return None;
        }
        let text = format!("{firing} ALERT{}", if firing == 1 { "" } else { "S" });
        Some(
            Button::new("alerts-badge")
                .danger()
                .small()
                .label(text)
                .tooltip("Open the Alerts tab")
                .on_click(cx.listener(|this, _, _, cx| this.open(Tab::Alerts, cx)))
                .into_any_element(),
        )
    }

    /// Record / Stop button.
    fn record_button(&self, elapsed: Option<f64>, cx: &mut Context<Self>) -> AnyElement {
        let button = match elapsed {
            Some(t) => Button::new("record")
                .danger()
                .label(format!(
                    "Stop  {}",
                    omnihw_core::recording::format_elapsed(t)
                ))
                .tooltip("Stop recording and save the CSV"),
            None => Button::new("record")
                .label("Record")
                .tooltip("Record all sensors over time (until stopped) to a CSV in Documents"),
        };
        button
            .small()
            .on_click(cx.listener(|this, _, _, cx| {
                let msg = tabs::recording::toggle(&this.sampler);
                this.set_status(msg, cx);
            }))
            .into_any_element()
    }

    /// "Report" menu: save an HTML report or copy a text one.
    fn report_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let app = cx.weak_entity();
        Button::new("report")
            .small()
            .label("Report")
            .dropdown_caret(true)
            .dropdown_menu(move |menu, _, _| {
                menu.item(menu_item("Save HTML report", &app, |this, _, cx| {
                    this.save_html_report(cx)
                }))
                .item(menu_item("Copy text report", &app, |this, _, cx| {
                    this.copy_text_report(cx)
                }))
            })
            .into_any_element()
    }

    /// "Update available" button, shown only when a newer version was found.
    fn update_button(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let crate::update::Status::Available(v) = self.updates.status() else {
            return None;
        };
        Some(
            Button::new("update")
                .small()
                .primary()
                .label(format!("Update {v} available"))
                .tooltip("Open Settings to check or download the new version")
                .on_click(cx.listener(|this, _, _, cx| this.open(Tab::Settings, cx)))
                .into_any_element(),
        )
    }

    /// "Info" menu of the Tabs and Classic bars: the feature guide and the About page.
    fn info_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let app = cx.weak_entity();
        Button::new("info")
            .small()
            .label("Info")
            .dropdown_caret(true)
            .selected(self.settings.nav_style == NavStyle::Tabs && self.tab == Tab::About)
            .dropdown_menu(move |menu, _, _| {
                menu.item(menu_item("Feature guide", &app, |this, _, cx| {
                    this.help.toggle(cx);
                    cx.notify();
                }))
                .item(menu_item("About Omni-HW", &app, |this, _, cx| {
                    this.open(Tab::About, cx)
                }))
            })
            .into_any_element()
    }

    /// Right-hand actions of the Classic and Tabs bars: alerts indicator, record, report and
    /// info menus, Settings, then the administrator indicator.
    fn actions(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut out = Vec::new();
        if self.sampler.info().is_none() {
            return out;
        }
        // One short lock for what the bar shows.
        let (firing, elapsed) = self.bar_state();
        out.extend(self.alerts_badge(firing, cx));
        out.push(self.record_button(elapsed, cx));
        out.push(self.report_menu(cx));
        out.extend(self.update_button(cx));
        out.push(self.info_menu(cx));
        out.push(
            Button::new("settings")
                .small()
                .label("Settings")
                .selected(self.tab == Tab::Settings)
                .tooltip("Theme, navigation bar, startup and more")
                .on_click(cx.listener(|this, _, _, cx| this.open(Tab::Settings, cx)))
                .into_any_element(),
        );
        if crate::elevate::elevated() {
            let (text, color) = if self.settings.nav_style == NavStyle::Tabs {
                // A compact badge, so the actions still fit beside the category tabs.
                ("ADMIN", heat::GREEN)
            } else {
                ("Running as administrator", heat::YELLOW)
            };
            out.push(
                ui::with_tip("admin", ui::pill(text, color, true, cx), ADMIN_HOVER)
                    .into_any_element(),
            );
        }
        out
    }

    // ---- Sidebar style ----------------------------------------------------------------------

    /// A full-width sidebar row: icon and label; when selected, filled with an accent bar on
    /// the left edge.
    fn nav_row(
        &self,
        id: impl Into<gpui_kit::ElementId>,
        selected: bool,
        icon: IconName,
        label: &'static str,
        cx: &App,
    ) -> ui::Clickable {
        let t = cx.theme();
        let (accent, hover, radius) = (t.primary, t.sidebar_accent, t.radius);
        let text = t.sidebar_foreground;
        ui::clickable(id)
            .h_flex()
            .w_full()
            .h(rems(1.85))
            .gap_2()
            .pl_2()
            .pr_1()
            .rounded(radius)
            .border_l_2()
            .cursor_pointer()
            .map(|row| {
                if selected {
                    row.border_color(accent)
                        .bg(accent.opacity(0.18))
                        .font_weight(FontWeight::MEDIUM)
                } else {
                    row.border_color(gpui_kit::transparent_black())
                        .hover(move |s| s.bg(hover))
                }
            })
            .child(
                Icon::new(icon)
                    .small()
                    .text_color(text.opacity(if selected { 1.0 } else { 0.7 })),
            )
            .child(ui::label(label).text_color(text))
    }

    /// A sidebar row that opens `tab`.
    fn nav_page(&self, tab: Tab, cx: &mut Context<Self>) -> ui::Clickable {
        self.nav_row(
            ("nav", tab as usize),
            self.tab == tab,
            tab.icon(),
            tab.name(),
            cx,
        )
        .on_click(cx.listener(move |this, _, _, cx| this.open(tab, cx)))
    }

    /// Sidebar navigation: every page listed under its category, with Feature guide, About and
    /// Settings pinned to the bottom.
    fn sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = cx.theme();
        let (fill, text, border, weak) = (
            t.sidebar,
            t.sidebar_foreground,
            t.sidebar_border,
            t.muted_foreground,
        );
        let mut pages = v_flex()
            .id("nav-pages")
            .flex_1()
            .min_h_0()
            .gap_0p5()
            .overflow_y_scroll();
        for tab in [Tab::Overview, Tab::Health] {
            pages = pages.child(self.nav_page(tab, cx));
        }
        for (group, tabs) in GROUPS {
            // Small uppercase category heading.
            pages = pages.child(
                ui::label(group.to_uppercase())
                    .pt_3()
                    .pb_0p5()
                    .pl_2()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(weak),
            );
            for &tab in tabs {
                pages = pages.child(self.nav_page(tab, cx));
            }
        }

        let mut footer = v_flex()
            .flex_none()
            .gap_0p5()
            .pt_1()
            .border_t_1()
            .border_color(border)
            .child(
                self.nav_row(
                    "nav-help",
                    self.help.is_open(),
                    IconName::BookOpen,
                    "Feature guide",
                    cx,
                )
                .tooltip(ui::tip("What every page shows and how to use it"))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.help.toggle(cx);
                    cx.notify();
                })),
            )
            .child(self.nav_page(Tab::About, cx))
            .child(self.nav_page(Tab::Settings, cx));
        if crate::elevate::elevated() {
            footer = footer.child(ui::with_tip(
                "admin",
                ui::colored("● Running as administrator", heat::GREEN, cx)
                    .text_xs()
                    .pl_2()
                    .pt_1(),
                ADMIN_HOVER,
            ));
        }

        v_flex()
            .flex_none()
            .w(rems(SIDEBAR_WIDTH))
            .h_full()
            .px_2()
            .py_2p5()
            .gap_2()
            .bg(fill)
            .text_color(text)
            .border_r_1()
            .border_color(border)
            .child(
                h_flex()
                    .flex_none()
                    .items_baseline()
                    .gap_2()
                    .pl_1p5()
                    .child(
                        ui::label("Omni-HW")
                            .text_xl()
                            .font_weight(FontWeight::BOLD)
                            .text_color(text),
                    )
                    .child(
                        ui::label(concat!("v", env!("CARGO_PKG_VERSION")))
                            .text_xs()
                            .text_color(weak),
                    ),
            )
            .child(pages)
            .child(footer)
    }

    /// Slim bar above the page (sidebar style): where you are on the left, actions on the right.
    fn top_bar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let mut crumbs = h_flex().gap_1p5().min_w_0();
        if let Some((group, _)) = GROUPS.iter().find(|(_, p)| p.contains(&self.tab)) {
            crumbs = crumbs.child(ui::weak(*group, cx)).child(ui::weak("›", cx));
        }
        crumbs = crumbs.child(
            ui::label(self.tab.name())
                .text_lg()
                .font_weight(FontWeight::SEMIBOLD),
        );
        let mut actions = h_flex().flex_none().gap_1p5();
        if self.sampler.info().is_some() {
            let (firing, elapsed) = self.bar_state();
            actions = actions
                .children(self.alerts_badge(firing, cx))
                .children(self.update_button(cx))
                .child(self.record_button(elapsed, cx))
                .child(self.report_menu(cx));
        }
        h_flex()
            .flex_none()
            .justify_between()
            .gap_3()
            .px_3()
            .py_1p5()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(crumbs)
            .child(actions)
    }

    // ---- Tabs style -------------------------------------------------------------------------

    /// A category tab: frameless text, tinted on hover, with an accent underline when selected.
    fn underline_tab(&self, category: usize, cx: &mut Context<Self>) -> ui::Clickable {
        let t = cx.theme();
        let (accent, hover, weak, strong) = (t.primary, t.accent, t.muted_foreground, t.foreground);
        let selected = nav::category_of(self.tab) == Some(category);
        ui::clickable(("category", category))
            .px_3()
            .pt_1p5()
            .pb_1()
            .border_b_3()
            .rounded_t(px(4.0))
            .cursor_pointer()
            .map(|tab| {
                if selected {
                    tab.border_color(accent).font_weight(FontWeight::MEDIUM)
                } else {
                    tab.border_color(gpui_kit::transparent_black())
                        .hover(move |s| s.bg(hover.opacity(0.6)).border_color(accent.opacity(0.35)))
                }
            })
            .child(
                ui::label(nav::category_label(category)).text_color(if selected {
                    strong
                } else {
                    weak
                }),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                if nav::category_of(this.tab) != Some(category)
                    && let Some(page) = this.nav_memory.page_in(category)
                {
                    this.open(page, cx);
                }
            }))
    }

    /// Tabs bar, top row: category tabs on the left, actions on the right. When they don't
    /// fit side by side (narrow window) the actions wrap onto their own line.
    fn primary_row(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let tabs = h_flex()
            .flex_none()
            .items_end()
            .gap_0p5()
            .children((0..nav::TAB_CATEGORIES).map(|c| self.underline_tab(c, cx)));
        h_flex()
            .flex_none()
            .flex_wrap_reverse()
            .items_end()
            .justify_between()
            .gap_x_4()
            .px_2()
            .pt_1p5()
            .child(tabs)
            .child(h_flex().gap_1p5().pb_1().children(self.actions(cx)))
    }

    /// A page chip on the Tabs bar's second row: a pill, filled with the selection colour when
    /// selected and outlined otherwise.
    fn page_chip(&self, tab: Tab, cx: &mut Context<Self>) -> ui::Clickable {
        let t = cx.theme();
        let (accent, on_accent, hover, border, text) = (
            t.primary,
            t.primary_foreground,
            t.accent,
            t.border,
            t.foreground,
        );
        let selected = self.tab == tab;
        ui::clickable(("chip", tab as usize))
            .px(px(11.0))
            .py(px(2.0))
            .text_sm()
            .rounded_full()
            .border_1()
            .cursor_pointer()
            .map(|chip| {
                if selected {
                    chip.bg(accent).border_color(accent)
                } else {
                    chip.border_color(border).hover(move |s| s.bg(hover))
                }
            })
            .child(ui::label(tab.name()).text_color(if selected { on_accent } else { text }))
            .on_click(cx.listener(move |this, _, _, cx| this.open(tab, cx)))
    }

    /// Tabs bar, second row: the open category's pages as chips, or the page name for a
    /// single-page category. Always shown (same height either way) so switching categories
    /// doesn't make the page below jump.
    fn page_strip(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let pages = nav::category_of(self.tab).map_or(&[][..], |c| nav::CATEGORIES[c]);
        let mut left = h_flex().flex_wrap().gap_1p5().min_h(rems(1.6));
        let hint = if pages.len() > 1 {
            left = left.children(pages.iter().map(|&t| self.page_chip(t, cx)));
            "← → sections    ↑ ↓ pages"
        } else {
            left = left.child(ui::weak(self.tab.name(), cx).pl_1());
            "← → sections"
        };
        let t = cx.theme();
        h_flex()
            .flex_none()
            .justify_between()
            .gap_3()
            .px_2()
            .py_1()
            .bg(t.muted)
            .border_y_1()
            .border_color(t.border)
            .child(left)
            .child(
                ui::with_tip(
                    "arrows-hint",
                    ui::small_weak(hint, cx).whitespace_nowrap(),
                    "Arrow keys switch pages when no text field is focused",
                )
                .flex_none(),
            )
    }

    // ---- Classic style ----------------------------------------------------------------------

    /// Classic bar: every page as its own tab in one row, then the actions.
    fn classic_bar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let pages = Tab::ALL.into_iter().map(|tab| {
            Button::new(("page", tab as usize))
                .small()
                .ghost()
                .label(tab.name())
                .selected(self.tab == tab)
                .on_click(cx.listener(move |this, _, _, cx| this.open(tab, cx)))
        });
        let t = cx.theme();
        h_flex()
            .flex_none()
            .flex_wrap()
            .gap_1()
            .px_2()
            .py_1()
            .border_b_1()
            .border_color(t.border)
            .children(pages.collect::<Vec<_>>())
            .child(div().w(px(1.0)).h(rems(1.2)).mx_1().bg(t.border))
            .children(self.actions(cx))
    }

    // ---- Page area --------------------------------------------------------------------------

    /// Thin bottom bar for transient messages, so they don't push the navigation onto two rows.
    fn status_bar(&self, cx: &App) -> Option<impl IntoElement + use<>> {
        let (msg, at) = self.status.as_ref()?;
        if at.elapsed() >= STATUS_DURATION {
            return None;
        }
        Some(
            div()
                .flex_none()
                .px_3()
                .py_1()
                .border_t_1()
                .border_color(cx.theme().border)
                .child(ui::weak(msg.clone(), cx)),
        )
    }

    /// The open page, in a scroll area unless the page scrolls its own panes.
    fn page(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if self.sampler.info().is_none() {
            // Static hardware info is still being read (off the UI thread).
            return div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .child(Spinner::new().large())
                .into_any_element();
        }
        let page = tabs::show(self, window, cx);
        self.sync_dismissed(cx);
        let frame = v_flex().flex_1().min_h_0().min_w_0().p_3();
        if matches!(self.tab, Tab::Recording | Tab::Health) {
            // A list and a details pane (or a chart), each with its own scroll area.
            frame.child(page).into_any_element()
        } else {
            frame
                .overflow_y_scrollbar()
                .id(("page", self.tab as usize))
                .child(page)
                .into_any_element()
        }
    }
}

/// A menu entry that runs `run` on the app when clicked.
fn menu_item(
    label: &'static str,
    app: &WeakEntity<OmniApp>,
    run: impl Fn(&mut OmniApp, &mut Window, &mut Context<OmniApp>) + 'static,
) -> PopupMenuItem {
    let app = app.clone();
    PopupMenuItem::new(label).on_click(move |_: &ClickEvent, window, cx| {
        let _ = app.update(cx, |this, cx| run(this, window, cx));
    })
}

/// The Win32 window handle, for notifications and the tray icon.
fn window_hwnd(window: &Window) -> Option<isize> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match HasWindowHandle::window_handle(window).ok()?.as_raw() {
        RawWindowHandle::Win32(w) => Some(w.hwnd.get()),
        _ => None,
    }
}

impl Render for OmniApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.worker_messages(cx);
        let page = self.page(window, cx);
        let status = self.status_bar(cx);
        let body = match self.settings.nav_style {
            NavStyle::Sidebar => h_flex()
                .size_full()
                .items_stretch()
                .child(self.sidebar(cx))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .child(self.top_bar(cx))
                        .child(page)
                        .children(status),
                ),
            NavStyle::Tabs => v_flex()
                .size_full()
                .child(self.primary_row(cx))
                .child(self.page_strip(cx))
                .child(page)
                .children(status),
            NavStyle::Classic => v_flex()
                .size_full()
                .child(self.classic_bar(cx))
                .child(page)
                .children(status),
        };
        let t = cx.theme();
        div()
            .id("omnihw")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &NavLeft, _, cx| this.arrow(Arrow::Left, cx)))
            .on_action(cx.listener(|this, _: &NavRight, _, cx| this.arrow(Arrow::Right, cx)))
            .on_action(cx.listener(|this, _: &NavUp, _, cx| this.arrow(Arrow::Up, cx)))
            .on_action(cx.listener(|this, _: &NavDown, _, cx| this.arrow(Arrow::Down, cx)))
            .size_full()
            .bg(t.background)
            .text_color(t.foreground)
            .font_family(t.font_family.clone())
            .text_size(t.font_size)
            .child(body)
    }
}
