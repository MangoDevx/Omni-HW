use crate::nav::{self, GROUPS};
use crate::sampler::Sampler;
use crate::settings::{NavStyle, Settings};
use crate::{export, tabs};
use egui::{Button, RichText, TextStyle};
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

    /// Leading glyph in the sidebar (all present in egui's built-in fonts).
    pub fn icon(self) -> &'static str {
        match self {
            Tab::Overview => "🏠",
            Tab::Health => "❤",
            Tab::Cpu => "🔲",
            Tab::Caches => "⚡",
            Tab::Mainboard => "🔧",
            Tab::Memory => "💾",
            Tab::Gpu => "🖵",
            Tab::Storage => "🖴",
            Tab::Network => "🌐",
            Tab::LanSecurity => "🛡",
            Tab::Power => "🔌",
            Tab::Sensors => "📈",
            Tab::Recording => "⏺",
            Tab::Alerts => "🔔",
            Tab::EventLog => "📋",
            Tab::About => "ℹ",
            Tab::Settings => "⚙",
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
/// Hover text of the "running as administrator" indicator, in every navigation style.
const ADMIN_HOVER: &str = "Omni-HW has administrator rights: you approved its UAC prompt or \
                           started it as administrator.";
/// Width of the navigation sidebar.
const SIDEBAR_WIDTH: f32 = 190.0;
/// Height of one sidebar row.
const ROW_HEIGHT: f32 = 26.0;

/// Sidebar background: a shade darker than the page, so it reads as its own surface.
fn sidebar_fill(v: &egui::Visuals) -> egui::Color32 {
    let t = if v.dark_mode { 0.3 } else { 0.035 };
    v.panel_fill.lerp_to_gamma(egui::Color32::BLACK, t)
}

/// Small uppercase category heading in the sidebar.
fn nav_section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        ui.add_space(10.0);
        let color = ui.visuals().weak_text_color();
        ui.add(
            egui::Label::new(
                RichText::new(title.to_uppercase())
                    .small()
                    .strong()
                    .extra_letter_spacing(1.0)
                    .color(color),
            )
            .selectable(false),
        );
    });
    ui.add_space(2.0);
}

/// A full-width sidebar row: glyph and label; when selected, filled with an accent bar on the
/// left edge.
fn nav_row(ui: &mut egui::Ui, selected: bool, icon: &str, label: &str) -> egui::Response {
    let size = egui::vec2(ui.available_width(), ROW_HEIGHT);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, label)
    });
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let v = ui.visuals();
    let radius = v.widgets.hovered.corner_radius;
    let painter = ui.painter();
    let text_color = if selected {
        painter.rect_filled(rect, radius, v.selection.bg_fill.gamma_multiply(0.6));
        let bar = egui::Rect::from_min_size(rect.min, egui::vec2(3.0, rect.height()));
        painter.rect_filled(
            bar.shrink2(egui::vec2(0.0, 4.0)),
            2.0,
            v.selection.stroke.color,
        );
        v.selection.stroke.color
    } else if response.hovered() {
        painter.rect_filled(rect, radius, v.widgets.hovered.weak_bg_fill);
        v.widgets.hovered.text_color()
    } else {
        v.text_color()
    };
    let font = egui::TextStyle::Button.resolve(ui.style());
    let y = rect.center().y;
    let icon_color = if selected {
        text_color
    } else {
        text_color.gamma_multiply(0.7)
    };
    painter.text(
        egui::pos2(rect.left() + 20.0, y),
        egui::Align2::CENTER_CENTER,
        icon,
        font.clone(),
        icon_color,
    );
    painter.text(
        egui::pos2(rect.left() + 36.0, y),
        egui::Align2::LEFT_CENTER,
        label,
        font,
        text_color,
    );
    response
}

/// Gap between the Tabs bar's category tabs.
const TAB_GAP: f32 = 2.0;
/// Thickness of the active category tab's underline.
const TAB_UNDERLINE: f32 = 3.0;
const TAB_PADDING: egui::Vec2 = egui::vec2(12.0, 7.0);
const CHIP_PADDING: egui::Vec2 = egui::vec2(11.0, 3.0);

fn tab_galley(ui: &egui::Ui, label: &str, size_delta: f32) -> std::sync::Arc<egui::Galley> {
    let size = TextStyle::Button.resolve(ui.style()).size + size_delta;
    egui::WidgetText::from(RichText::new(label).size(size)).into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        TextStyle::Button,
    )
}

fn underline_tab_size(ui: &egui::Ui, label: &str) -> egui::Vec2 {
    tab_galley(ui, label, 1.0).size() + 2.0 * TAB_PADDING + egui::vec2(0.0, TAB_UNDERLINE)
}

/// A category tab: frameless text, tinted on hover, with an accent underline when selected.
fn underline_tab(ui: &mut egui::Ui, label: &str, selected: bool) -> egui::Response {
    let galley = tab_galley(ui, label, 1.0);
    let size = galley.size() + 2.0 * TAB_PADDING + egui::vec2(0.0, TAB_UNDERLINE);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, true, selected, label)
    });
    if ui.is_rect_visible(rect) {
        let v = ui.visuals();
        let hovered = response.hovered() || response.has_focus();
        if hovered && !selected {
            let top = egui::CornerRadius {
                nw: 4,
                ne: 4,
                sw: 0,
                se: 0,
            };
            ui.painter().rect_filled(
                rect,
                top,
                v.widgets.hovered.weak_bg_fill.gamma_multiply(0.6),
            );
        }
        let color = if selected {
            v.strong_text_color()
        } else if hovered {
            v.widgets.hovered.text_color()
        } else {
            v.widgets.inactive.text_color()
        };
        let line = egui::Rect::from_min_max(
            egui::pos2(rect.left() + 4.0, rect.bottom() - TAB_UNDERLINE),
            egui::pos2(rect.right() - 4.0, rect.bottom()),
        );
        if selected {
            ui.painter().rect_filled(line, 1.5, v.hyperlink_color);
        } else if hovered {
            let faint = v.hyperlink_color.gamma_multiply(0.35);
            ui.painter()
                .rect_filled(line.shrink2(egui::vec2(0.0, 0.5)), 1.0, faint);
        }
        let pos = rect.min + TAB_PADDING;
        ui.painter().galley(pos, galley, color);
    }
    response
}

fn page_chip_size(ui: &egui::Ui, label: &str) -> egui::Vec2 {
    tab_galley(ui, label, -0.5).size() + 2.0 * CHIP_PADDING
}

/// A page chip on the Tabs bar's second row: a pill, filled with the selection colour when
/// selected and outlined otherwise.
fn page_chip(ui: &mut egui::Ui, label: &str, selected: bool) -> egui::Response {
    let galley = tab_galley(ui, label, -0.5);
    let size = galley.size() + 2.0 * CHIP_PADDING;
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, true, selected, label)
    });
    if ui.is_rect_visible(rect) {
        let v = ui.visuals();
        let hovered = response.hovered() || response.has_focus();
        let (fill, stroke, text) = if selected {
            (
                v.selection.bg_fill,
                egui::Stroke::NONE,
                v.selection.stroke.color,
            )
        } else if hovered {
            (
                v.widgets.hovered.weak_bg_fill,
                v.widgets.hovered.bg_stroke,
                v.widgets.hovered.text_color(),
            )
        } else {
            (
                egui::Color32::TRANSPARENT,
                egui::Stroke::new(1.0, v.widgets.noninteractive.bg_stroke.color),
                v.widgets.inactive.text_color(),
            )
        };
        let radius = rect.height() / 2.0;
        ui.painter()
            .rect(rect, radius, fill, stroke, egui::StrokeKind::Inside);
        ui.painter().galley(rect.min + CHIP_PADDING, galley, text);
    }
    response
}

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
    /// Width of the Tabs bar's actions last frame, to right-align them.
    actions_width: f32,
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
            actions_width: 0.0,
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

    /// Classic bar: every page as its own tab in one row.
    fn page_nav(&mut self, ui: &mut egui::Ui) {
        for tab in Tab::ALL {
            ui.selectable_value(&mut self.tab, tab, tab.name());
        }
    }

    /// Firing alert count and recording time, read in one short lock.
    fn bar_state(&self) -> (usize, Option<f64>) {
        let snap = self.sampler.snapshot();
        let elapsed = snap.recording.as_ref().map(|r| r.rec.duration_secs());
        (tabs::alerts::firing_count(&snap), elapsed)
    }

    /// Red "N ALERTS" badge, shown only while alerts fire.
    fn alerts_badge(&mut self, ui: &mut egui::Ui, firing: usize) {
        if firing == 0 {
            return;
        }
        let text = format!("{firing} ALERT{}", if firing == 1 { "" } else { "S" });
        let b = Button::new(RichText::new(text).strong().color(egui::Color32::WHITE))
            .fill(crate::heat::RED);
        if ui.add(b).on_hover_text("Open the Alerts tab").clicked() {
            self.tab = Tab::Alerts;
        }
    }

    /// Record / Stop button.
    fn record_button(&mut self, ui: &mut egui::Ui, elapsed: Option<f64>) {
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
    }

    /// "Report" menu: save an HTML report or copy a text one.
    fn report_menu(&mut self, ui: &mut egui::Ui) {
        let Some(info) = self.sampler.info() else {
            return;
        };
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
        if let Some(m) = msg {
            self.set_status(m);
        }
    }

    /// "Update available" button, shown only when a newer version was found.
    fn update_button(&mut self, ui: &mut egui::Ui) {
        if let crate::update::Status::Available(v) = self.updates.status()
            && ui
                .button(format!("Update {v} available"))
                .on_hover_text("Open Settings to check or download the new version")
                .clicked()
        {
            self.tab = Tab::Settings;
        }
    }

    /// Right-hand actions of the Classic and Tabs bars: alerts indicator, record, report and info
    /// menus, Settings, then the administrator indicator.
    fn actions(&mut self, ui: &mut egui::Ui) {
        if self.sampler.info().is_none() {
            return;
        }
        let tabbed = self.settings.nav_style == NavStyle::Tabs;
        // One short lock for what the bar shows.
        let (firing, elapsed) = self.bar_state();
        self.alerts_badge(ui, firing);
        self.record_button(ui, elapsed);
        self.report_menu(ui);
        self.update_button(ui);

        let mut open_about = false;
        // In the Tabs bar, highlighted while About is open, like a selected category tab (the
        // classic bar has its own About tab).
        let info_button = Button::new("Info  ⏷").selected(tabbed && self.tab == Tab::About);
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
        if !crate::elevate::elevated() {
            return;
        }
        if tabbed {
            // A compact badge, so the actions still fit beside the category tabs.
            let ink = crate::heat::ink(ui.visuals(), crate::heat::GREEN);
            let badge = Button::new(RichText::new("ADMIN").small().strong().color(ink))
                .fill(ink.gamma_multiply(0.12))
                .stroke(egui::Stroke::new(1.0, ink.gamma_multiply(0.6)))
                .corner_radius(8.0)
                .frame_when_inactive(true)
                .sense(egui::Sense::hover());
            ui.add(badge).on_hover_text(ADMIN_HOVER);
        } else {
            let ink = crate::heat::ink(ui.visuals(), crate::heat::YELLOW);
            ui.add(
                egui::Label::new(
                    RichText::new("Running as administrator")
                        .strong()
                        .color(ink),
                )
                .selectable(false),
            )
            .on_hover_text(ADMIN_HOVER);
        }
    }

    /// Tabs bar, top row: category tabs on the left, actions right-aligned. The actions are
    /// placed using their width from the last frame; when they don't fit beside the tabs (narrow
    /// window) they get their own line above the tabs, so the tabs stay on the page strip.
    fn primary_row(&mut self, ui: &mut egui::Ui) {
        let tabs_width: f32 = (0..nav::TAB_CATEGORIES)
            .map(|c| underline_tab_size(ui, nav::category_label(c)).x + TAB_GAP)
            .sum();
        let fits = tabs_width + 24.0 + self.actions_width <= ui.available_width();
        if !fits {
            ui.horizontal_wrapped(|ui| self.actions(ui));
        }
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = TAB_GAP;
            let current = nav::category_of(self.tab);
            for c in 0..nav::TAB_CATEGORIES {
                let label = nav::category_label(c);
                if underline_tab(ui, label, current == Some(c)).clicked()
                    && current != Some(c)
                    && let Some(page) = self.nav_memory.page_in(c)
                {
                    self.tab = page;
                }
            }
            if fits {
                let row = ui.min_rect();
                let right = ui.max_rect().right();
                let rect = egui::Rect::from_min_max(
                    egui::pos2(right - self.actions_width, row.top()),
                    egui::pos2(right, row.bottom() - TAB_UNDERLINE),
                );
                let builder = egui::UiBuilder::new()
                    .max_rect(rect)
                    .layout(egui::Layout::left_to_right(egui::Align::Center));
                let width = ui
                    .scope_builder(builder, |ui| self.actions(ui))
                    .response
                    .rect
                    .width();
                if (width - self.actions_width).abs() > 0.5 {
                    self.actions_width = width;
                    ui.ctx().request_discard("navigation actions resized");
                }
            }
        });
    }

    /// Tabs bar, second row: the open category's pages as chips, or the page name for a
    /// single-page category. Always shown (same height either way) so switching categories
    /// doesn't make the page below jump.
    fn page_strip(&mut self, ui: &mut egui::Ui) {
        let chip_height = page_chip_size(ui, "Ag").y;
        let pages = nav::category_of(self.tab).map_or(&[][..], |c| nav::CATEGORIES[c]);
        ui.horizontal_wrapped(|ui| {
            ui.set_min_height(chip_height);
            ui.spacing_mut().item_spacing.x = 6.0;
            if pages.len() > 1 {
                for &t in pages {
                    if page_chip(ui, t.name(), self.tab == t).clicked() {
                        self.tab = t;
                    }
                }
            } else {
                ui.add_space(4.0);
                ui.label(RichText::new(self.tab.name()).color(ui.visuals().weak_text_color()));
            }
            let hint = if pages.len() > 1 {
                "\u{2190} \u{2192} sections    \u{2191} \u{2193} pages"
            } else {
                "\u{2190} \u{2192} sections"
            };
            let hint = RichText::new(hint)
                .small()
                .color(ui.visuals().weak_text_color());
            let hint_width = egui::WidgetText::from(hint.clone())
                .into_galley(
                    ui,
                    Some(egui::TextWrapMode::Extend),
                    f32::INFINITY,
                    TextStyle::Small,
                )
                .size()
                .x;
            // Only when there's room, so it never pushes the chips onto another line.
            if ui.available_width() > hint_width + 24.0 {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(4.0);
                    ui.label(hint)
                        .on_hover_text("Arrow keys switch pages when no text field is focused");
                });
            }
        });
    }

    /// Sidebar navigation: every page listed under its category, with Feature guide, About and
    /// Settings pinned to the bottom.
    fn sidebar(&mut self, ui: &mut egui::Ui) {
        let fill = sidebar_fill(ui.visuals());
        let frame = egui::Frame::side_top_panel(ui.style())
            .fill(fill)
            .inner_margin(egui::Margin::symmetric(8, 10));
        egui::Panel::left("nav")
            .resizable(false)
            .exact_size(SIDEBAR_WIDTH)
            .frame(frame)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.add_space(6.0);
                    ui.add(
                        egui::Label::new(RichText::new("Omni-HW").size(20.0).strong())
                            .selectable(false),
                    );
                    ui.add(
                        egui::Label::new(
                            RichText::new(concat!("v", env!("CARGO_PKG_VERSION")))
                                .small()
                                .weak(),
                        )
                        .selectable(false),
                    );
                });
                ui.add_space(8.0);

                egui::Panel::bottom("nav_footer")
                    .frame(egui::Frame::NONE)
                    .resizable(false)
                    .show_separator_line(false)
                    .show(ui, |ui| {
                        ui.add_space(4.0);
                        ui.separator();
                        ui.add_space(2.0);
                        if nav_row(ui, self.help.open, "📖", "Feature guide")
                            .on_hover_text("What every page shows and how to use it")
                            .clicked()
                        {
                            self.help.open = !self.help.open;
                        }
                        for tab in [Tab::About, Tab::Settings] {
                            if nav_row(ui, self.tab == tab, tab.icon(), tab.name()).clicked() {
                                self.tab = tab;
                            }
                        }
                        if crate::elevate::elevated() {
                            ui.add_space(6.0);
                            ui.horizontal(|ui| {
                                ui.add_space(8.0);
                                let ink = crate::heat::ink(ui.visuals(), crate::heat::GREEN);
                                ui.add(
                                    egui::Label::new(
                                        RichText::new("● Running as administrator")
                                            .small()
                                            .color(ink),
                                    )
                                    .selectable(false),
                                )
                                .on_hover_text(ADMIN_HOVER);
                            });
                        }
                    });

                egui::CentralPanel::no_frame().show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .auto_shrink(false)
                        .show(ui, |ui| {
                            for tab in [Tab::Overview, Tab::Health] {
                                if nav_row(ui, self.tab == tab, tab.icon(), tab.name()).clicked() {
                                    self.tab = tab;
                                }
                            }
                            for (group, pages) in GROUPS {
                                nav_section(ui, group);
                                for &tab in pages {
                                    if nav_row(ui, self.tab == tab, tab.icon(), tab.name())
                                        .clicked()
                                    {
                                        self.tab = tab;
                                    }
                                }
                            }
                        });
                });
            });
    }

    /// Slim bar above the page (sidebar style): where you are on the left, actions on the right.
    fn top_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("actions").show(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add_space(4.0);
                if let Some((group, _)) = GROUPS.iter().find(|(_, p)| p.contains(&self.tab)) {
                    ui.weak(*group);
                    ui.weak("›");
                }
                ui.label(RichText::new(self.tab.name()).size(16.0).strong());
                if self.sampler.info().is_none() {
                    return;
                }
                let (firing, elapsed) = self.bar_state();
                // Right to left, so added in reverse of how they read.
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    self.report_menu(ui);
                    self.record_button(ui, elapsed);
                    self.update_button(ui);
                    self.alerts_badge(ui, firing);
                });
            });
            ui.add_space(2.0);
        });
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
        match self.settings.nav_style {
            NavStyle::Sidebar => {
                self.sidebar(ui);
                self.top_bar(ui);
            }
            NavStyle::Tabs => {
                // No bottom margin: the active tab's underline sits on the line between the rows.
                let frame = egui::Frame::side_top_panel(ui.style()).inner_margin(egui::Margin {
                    left: 8,
                    right: 8,
                    top: 6,
                    bottom: 0,
                });
                egui::Panel::top("tabs")
                    .frame(frame)
                    .show(ui, |ui| self.primary_row(ui));
                let v = ui.visuals();
                let band = crate::heat::lerp(v.panel_fill, v.extreme_bg_color, 0.45);
                let frame = egui::Frame::side_top_panel(ui.style())
                    .fill(band)
                    .inner_margin(egui::Margin::symmetric(8, 5));
                egui::Panel::top("pages")
                    .frame(frame)
                    .show(ui, |ui| self.page_strip(ui));
            }
            NavStyle::Classic => {
                egui::Panel::top("tabs").show(ui, |ui| {
                    ui.add_space(4.0);
                    ui.horizontal_wrapped(|ui| {
                        self.page_nav(ui);
                        ui.separator();
                        self.actions(ui);
                    });
                    ui.add_space(2.0);
                });
            }
        }
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
        tabs::health::dispatch(ui.ctx(), &self.sampler);

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
