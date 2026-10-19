use crate::nav::{self, GROUPS};
use crate::sampler::Sampler;
use crate::settings::{NavStyle, Settings};
use crate::{export, tabs};
use egui::{Button, RichText};
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

const STATUS_DURATION: Duration = Duration::from_secs(6);

pub struct OmniApp {
    tab: Tab,
    /// Page shown last frame, to notice when the Power page is opened.
    last_tab: Option<Tab>,
    /// Last page used in each navigation category, for Left/Right arrows.
    nav_memory: nav::NavMemory,
    /// Persisted user settings (theme, navigation style, start page, ...).
    settings: Settings,
    /// Selected adapter on the GPU tab.
    gpu_index: usize,
    sampler: Sampler,
    /// Transient message shown in the status bar (e.g. where a report was saved).
    status: Option<(String, Instant)>,
    rec_view: tabs::recording::ViewState,
    alerts_view: tabs::alerts::ViewState,
    event_log_view: tabs::eventlog::ViewState,
    /// Window handle passed to the sampler for notifications.
    hwnd_sent: bool,
    /// Started with `--minimized` (launch at startup); minimize on the first frame.
    minimize_on_start: bool,
    /// In-app documentation window.
    help: crate::help::HelpState,
    updates: crate::update::UpdateChecker,
    pawnio: crate::pawnio::Installer,
    /// Health Check results, shared by the Health tab and the Overview banner.
    health: tabs::health::HealthCache,
    /// Settings changed but not written to disk yet: they're saved once no mouse button is
    /// held, so dragging the UI scale slider doesn't rewrite the file on every frame.
    settings_unsaved: bool,
}

impl OmniApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut settings = Settings::load();
        // The Run entry is the source of truth (it may have been removed outside Omni-HW).
        settings.launch_at_startup = crate::autostart::is_enabled();
        crate::retro::install(&cc.egui_ctx);
        crate::settings::apply_visuals(&cc.egui_ctx, &settings);
        crate::export::set_data_dir(settings.data_dir.clone());
        crate::dismissed::load(&settings.dismissed);
        crate::tray::start(cc.egui_ctx.clone());
        crate::tray::set_icon_visible(settings.minimize_to_tray);
        let sampler = Sampler::start(cc.egui_ctx.clone(), settings.use_pawnio);
        sampler.set_interval_ms(settings.sample_interval_ms);
        sampler.set_alert_notifications(settings.alert_notifications);
        let updates = crate::update::UpdateChecker::default();
        if settings.check_updates_at_start {
            updates.start(&cc.egui_ctx);
        }
        OmniApp {
            tab: Tab::from_key(&settings.start_page).unwrap_or(Tab::Overview),
            last_tab: None,
            nav_memory: Default::default(),
            settings,
            gpu_index: 0,
            sampler,
            status: None,
            rec_view: Default::default(),
            alerts_view: Default::default(),
            event_log_view: Default::default(),
            hwnd_sent: false,
            minimize_on_start: crate::autostart::launch_flags().1,
            help: Default::default(),
            updates,
            pawnio: Default::default(),
            health: Default::default(),
            settings_unsaved: false,
        }
    }

    /// Arrow keys switch pages (see [`nav`]) unless a widget has keyboard focus (text field,
    /// slider, Tab-key focus) or a menu is open. Keys that don't move are left for the page.
    fn arrow_navigation(&mut self, ctx: &egui::Context) {
        self.nav_memory.visit(self.tab);
        if ctx.egui_wants_keyboard_input()
            || egui::Popup::is_any_open(ctx)
            || ctx.memory(|m| m.focused().is_some())
        {
            return;
        }
        let pressed = ctx.input(|i| {
            if !i.modifiers.is_none() {
                return None;
            }
            nav::Arrow::ALL
                .into_iter()
                .find(|&(key, _)| i.key_pressed(key))
        });
        let Some((key, arrow)) = pressed else {
            return;
        };
        if let Some(next) = nav::target(self.settings.nav_style, self.tab, arrow, &self.nav_memory)
        {
            ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, key));
            self.tab = next;
            self.nav_memory.visit(next);
        }
    }

    fn set_status(&mut self, msg: String) {
        self.status = Some((msg, Instant::now()));
    }

    /// Page buttons: grouped drop-downs (one compact row) or every page in a row.
    fn page_nav(&mut self, ui: &mut egui::Ui) {
        if self.settings.nav_style == NavStyle::Classic {
            for tab in Tab::ALL {
                ui.selectable_value(&mut self.tab, tab, tab.name());
            }
            return;
        }
        for tab in [Tab::Overview, Tab::Health] {
            ui.selectable_value(&mut self.tab, tab, tab.name());
        }
        for (group, pages) in GROUPS {
            let current = pages.iter().find(|&&t| t == self.tab);
            // The button names the open page, e.g. "Hardware: GPU".
            let label = match current {
                Some(t) => format!("{group}: {}  ⏷", t.name()),
                None => format!("{group}  ⏷"),
            };
            let button = Button::new(label).selected(current.is_some());
            egui::containers::menu::MenuButton::from_button(button).ui(ui, |ui| {
                for &t in pages {
                    if ui.selectable_label(self.tab == t, t.name()).clicked() {
                        self.tab = t;
                        ui.close();
                    }
                }
            });
        }
    }

    /// Right-hand actions: alerts indicator, record, report and info menus.
    fn actions(&mut self, ui: &mut egui::Ui) {
        let Some(info) = self.sampler.info() else {
            return;
        };
        // One short lock for what the bar shows.
        let (firing, elapsed) = {
            let snap = self.sampler.snapshot();
            let elapsed = snap.recording.as_ref().map(|r| r.rec.duration_secs());
            (tabs::alerts::firing_count(&snap), elapsed)
        };
        if firing > 0 {
            let text = format!("{firing} ALERT{}", if firing == 1 { "" } else { "S" });
            let b = Button::new(RichText::new(text).strong().color(egui::Color32::WHITE))
                .fill(crate::heat::RED);
            if ui.add(b).on_hover_text("Open the Alerts tab").clicked() {
                self.tab = Tab::Alerts;
            }
        }
        let rec_button = match elapsed {
            Some(t) => Button::new(
                RichText::new(format!(
                    "Stop  {}",
                    omnihw_core::recording::format_elapsed(t)
                ))
                .strong(),
            )
            .fill(crate::heat::RED.gamma_multiply(0.6)),
            None => Button::new("Record"),
        };
        let hover = if elapsed.is_some() {
            "Stop recording and save the CSV"
        } else {
            "Record all sensors over time (until stopped) to a CSV in Documents"
        };
        if ui.add(rec_button).on_hover_text(hover).clicked() {
            self.status = Some((tabs::recording::toggle(&self.sampler), Instant::now()));
        }

        let mut msg = None;
        egui::containers::menu::MenuButton::new("Report  ⏷").ui(ui, |ui| {
            if ui
                .button("Save HTML report")
                .on_hover_text("Save a report to Documents and open it in your browser")
                .clicked()
            {
                msg = Some(match export::save_html(info, &self.sampler.snapshot()) {
                    Ok(p) => format!("Saved {}", p.display()),
                    Err(e) => e,
                });
                ui.close();
            }
            if ui
                .button("Copy text report")
                .on_hover_text("Copy a plain-text report to the clipboard")
                .clicked()
            {
                ui.ctx()
                    .copy_text(export::text(info, &self.sampler.snapshot()));
                msg = Some("Report copied to clipboard".into());
                ui.close();
            }
        });

        if let crate::update::Status::Available(v) = self.updates.status()
            && ui
                .button(format!("Update {v} available"))
                .on_hover_text("Open Settings to check or download the new version")
                .clicked()
        {
            self.tab = Tab::Settings;
        }

        let mut open_about = false;
        // In the grouped bar, highlighted while About is open, like the Hardware and Monitoring
        // menus (the classic bar has its own About tab).
        let info_button = Button::new("Info  ⏷")
            .selected(self.tab == Tab::About && self.settings.nav_style == NavStyle::Grouped);
        egui::containers::menu::MenuButton::from_button(info_button).ui(ui, |ui| {
            if ui.button("Feature guide").clicked() {
                self.help.open = true;
                ui.close();
            }
            if ui.button("About Omni-HW").clicked() {
                open_about = true;
                ui.close();
            }
        });
        if open_about {
            self.tab = Tab::About;
        }
        if ui
            .add(Button::new("Settings").selected(self.tab == Tab::Settings))
            .on_hover_text("Theme, navigation bar, startup and more")
            .clicked()
        {
            self.tab = Tab::Settings;
        }
        if let Some(m) = msg {
            self.set_status(m);
        }
    }

    /// Thin bottom bar for transient messages, so they don't push the navigation onto two rows.
    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let Some((msg, at)) = &self.status else {
            return;
        };
        let left = STATUS_DURATION.saturating_sub(at.elapsed());
        if left.is_zero() {
            self.status = None;
            return;
        }
        let msg = msg.clone();
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.weak(msg);
        });
        ui.ctx().request_repaint_after(left);
    }
}

impl Drop for OmniApp {
    fn drop(&mut self) {
        // Remove the tray icon before the process exits.
        crate::tray::shutdown();
        if self.settings_unsaved {
            let _ = self.settings.save();
        }
    }
}

impl eframe::App for OmniApp {
    /// Runs even while the window is minimized or hidden (unlike `ui`), so minimize-to-tray works.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        crate::tray::set_icon_visible(self.settings.minimize_to_tray);
        if self.settings.minimize_to_tray && ctx.input(|i| i.viewport().minimized) == Some(true) {
            crate::tray::hide_main_window();
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        if !self.hwnd_sent {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            if let Ok(h) = frame.window_handle()
                && let RawWindowHandle::Win32(w) = h.as_raw()
            {
                self.sampler.set_window_handle(w.hwnd.get());
                crate::tray::set_main_window(w.hwnd.get());
            }
            self.hwnd_sent = true;
        }
        if std::mem::take(&mut self.minimize_on_start) {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        }
        self.arrow_navigation(ui.ctx());
        egui::Panel::top("tabs").show(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                self.page_nav(ui);
                ui.separator();
                self.actions(ui);
            });
            ui.add_space(2.0);
        });
        self.status_bar(ui);

        crate::help::show(ui.ctx(), &mut self.help);

        // Re-read power devices and wake history whenever the Power page is opened.
        if self.tab == Tab::Power && self.last_tab != Some(Tab::Power) {
            self.sampler.refresh_power();
        }
        // Read network adapters often (and in full right away) while the Network page is open.
        let on_network = self.tab == Tab::Network;
        self.sampler.set_network_page(on_network);
        if on_network && self.last_tab != Some(Tab::Network) {
            self.sampler.refresh_network();
        }
        // Read the event logs the first time the Event Log page is opened (Refresh re-reads).
        if self.tab == Tab::EventLog && self.last_tab != Some(Tab::EventLog) {
            let unread = {
                let snap = self.sampler.snapshot();
                snap.event_log.is_none() && !snap.event_log_busy
            };
            if unread {
                self.sampler.read_event_log();
            }
        }
        self.last_tab = Some(self.tab);

        egui::CentralPanel::default_margins().show(ui, |ui| {
            let Some(info) = self.sampler.info() else {
                ui.centered_and_justified(|ui| {
                    ui.spinner();
                });
                return;
            };
            if self.tab == Tab::Recording {
                // Has its own panels and scroll areas.
                if tabs::recording::show(ui, &self.sampler, &mut self.rec_view) {
                    let msg = tabs::recording::toggle(&self.sampler);
                    self.set_status(msg);
                }
                return;
            }
            if self.tab == Tab::Health {
                // A list and a details pane, each with its own scroll area. The report
                // doesn't borrow the snapshot, so the lock is released before drawing.
                let snap = self.sampler.snapshot();
                let report = self.health.get(info, &snap, self.settings.show_gaming);
                drop(snap);
                tabs::health::show(ui, info, report);
                return;
            }
            egui::ScrollArea::vertical()
                .auto_shrink(false)
                .show(ui, |ui| {
                    if self.tab == Tab::Alerts {
                        // Takes the snapshot lock itself.
                        return tabs::alerts::show(ui, info, &self.sampler, &mut self.alerts_view);
                    }
                    if self.tab == Tab::EventLog {
                        // Takes the snapshot lock itself, briefly.
                        return tabs::eventlog::show(ui, &self.sampler, &mut self.event_log_view);
                    }
                    if self.tab == Tab::Sensors {
                        // Takes the snapshot lock itself.
                        return tabs::sensors::show(ui, &self.sampler);
                    }
                    if self.tab == Tab::Settings {
                        if tabs::settings::show(
                            ui,
                            &mut self.settings,
                            &self.sampler,
                            &self.updates,
                            &self.pawnio,
                        ) {
                            crate::settings::apply_visuals(ui.ctx(), &self.settings);
                            self.settings_unsaved = true;
                        }
                        return;
                    }
                    let snap = self.sampler.snapshot();
                    match self.tab {
                        Tab::Overview => {
                            let health = self.health.get(info, &snap, self.settings.show_gaming);
                            if tabs::overview::show(ui, info, &snap, health) {
                                self.tab = Tab::Health;
                            }
                        }
                        Tab::Cpu => tabs::cpu::show(ui, info, &snap),
                        Tab::Caches => tabs::caches::show(ui, info),
                        Tab::Mainboard => tabs::mainboard::show(ui, info),
                        Tab::Memory => tabs::memory::show(ui, info, &snap),
                        Tab::Gpu => tabs::gpu::show(ui, info, &snap, &mut self.gpu_index),
                        Tab::Storage => tabs::storage::show(ui, info, &snap),
                        Tab::Network => tabs::network::show(ui, &snap),
                        Tab::LanSecurity => tabs::lansec::show(ui, &snap),
                        Tab::Power => tabs::power::show(ui, &snap),
                        Tab::About => tabs::about::show(ui, info, &snap),
                        Tab::Sensors
                        | Tab::Recording
                        | Tab::Alerts
                        | Tab::EventLog
                        | Tab::Settings
                        | Tab::Health => {
                            unreachable!()
                        }
                    }
                });
        });

        // Power changes requested this frame (Power page or a Health Check "Fix"), applied
        // outside the snapshot lock.
        if let Some(msg) = tabs::power::dispatch(ui.ctx(), &self.sampler) {
            self.set_status(msg);
        }
        // Same for network changes and tests.
        if let Some(msg) = tabs::network::dispatch(ui.ctx(), &self.sampler, on_network) {
            self.set_status(msg);
        }
        // The LAN Security Report scan, when its button asks for one.
        tabs::lansec::dispatch(ui.ctx(), &self.sampler);
        tabs::storage::dispatch(ui.ctx(), &self.sampler);
        tabs::memory::dispatch(ui.ctx(), &self.sampler);

        // Health Check dismissals changed this frame: keep them in the settings (saved to disk
        // only when "remember dismissed" is on).
        if let Some(keys) = crate::dismissed::take_changes() {
            self.settings.dismissed = keys;
            self.settings_unsaved |= self.settings.remember_dismissed;
        }
        if self.settings_unsaved && !ui.ctx().input(|i| i.pointer.any_down()) {
            self.settings_unsaved = false;
            if let Err(e) = self.settings.save() {
                self.set_status(e);
            }
        }
    }
}
