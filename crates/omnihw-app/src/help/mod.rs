//! In-app help: a window with one documentation tab per feature.
//!
//! Numbers that come from code (heatmap thresholds, bottleneck thresholds, the list of
//! Health Check rules) are read from the constants/code that use them, so the help stays in
//! step with the app.

mod hardware;
mod health;
mod monitoring;
mod start;

use egui::{Context, RichText, Ui};
use hardware::{hardware, lan_security, network, power, sensors};
use health::health_check;
use monitoring::{alerts, recording, reports};
use start::{getting_started, overview, privacy, settings, troubleshooting};

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum HelpTab {
    #[default]
    GettingStarted,
    Overview,
    HealthCheck,
    Hardware,
    Network,
    LanSecurity,
    Power,
    Sensors,
    Recording,
    Alerts,
    Reports,
    Settings,
    Privacy,
    Troubleshooting,
}

const TABS: [(HelpTab, &str); 14] = [
    (HelpTab::GettingStarted, "Getting Started"),
    (HelpTab::Overview, "Overview"),
    (HelpTab::HealthCheck, "Health Check"),
    (HelpTab::Hardware, "Hardware Tabs"),
    (HelpTab::Network, "Network"),
    (HelpTab::LanSecurity, "LAN Security"),
    (HelpTab::Power, "Power"),
    (HelpTab::Sensors, "Sensors"),
    (HelpTab::Recording, "Recording"),
    (HelpTab::Alerts, "Alerts"),
    (HelpTab::Reports, "Reports"),
    (HelpTab::Settings, "Settings"),
    (HelpTab::Privacy, "Privacy & Safety"),
    (HelpTab::Troubleshooting, "Troubleshooting"),
];

/// Help window state: whether it's open and which tab is selected (kept while the app runs).
#[derive(Default)]
pub struct HelpState {
    pub open: bool,
    tab: HelpTab,
}

/// Show the help window if it's open.
pub fn show(ctx: &Context, state: &mut HelpState) {
    let mut open = state.open;
    egui::Window::new("Omni-HW Help")
        .open(&mut open)
        .resizable(true)
        .collapsible(false)
        .default_size([760.0, 560.0])
        .show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                for (tab, name) in TABS {
                    ui.selectable_value(&mut state.tab, tab, name);
                }
            });
            ui.separator();
            egui::ScrollArea::vertical()
                .id_salt(("help_scroll", state.tab as u8))
                .auto_shrink(false)
                .show(ui, |ui| match state.tab {
                    HelpTab::GettingStarted => getting_started(ui),
                    HelpTab::Overview => overview(ui),
                    HelpTab::HealthCheck => health_check(ui),
                    HelpTab::Hardware => hardware(ui),
                    HelpTab::Network => network(ui),
                    HelpTab::LanSecurity => lan_security(ui),
                    HelpTab::Power => power(ui),
                    HelpTab::Sensors => sensors(ui),
                    HelpTab::Recording => recording(ui),
                    HelpTab::Alerts => alerts(ui),
                    HelpTab::Reports => reports(ui),
                    HelpTab::Settings => settings(ui),
                    HelpTab::Privacy => privacy(ui),
                    HelpTab::Troubleshooting => troubleshooting(ui),
                });
        });
    state.open = open;
}

// ---- Formatting helpers ---------------------------------------------------------------------

fn h(ui: &mut Ui, text: &str) {
    ui.add_space(8.0);
    ui.label(RichText::new(text).size(16.0).strong());
    ui.add_space(2.0);
}

fn p(ui: &mut Ui, text: &str) {
    ui.add(egui::Label::new(text).wrap());
    ui.add_space(4.0);
}

fn bullets<S: AsRef<str>>(ui: &mut Ui, items: &[S]) {
    for t in items {
        ui.horizontal_wrapped(|ui| {
            ui.label("•");
            ui.add(egui::Label::new(t.as_ref()).wrap());
        });
    }
    ui.add_space(4.0);
}

/// Smaller, collapsed technical notes for curious readers.
fn details<S: AsRef<str>>(ui: &mut Ui, id: &str, items: &[S]) {
    egui::CollapsingHeader::new(RichText::new("Details").small().strong())
        .id_salt(("help_details", id))
        .show(ui, |ui| {
            for t in items {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("•").small().weak());
                    ui.add(egui::Label::new(RichText::new(t.as_ref()).small().weak()).wrap());
                });
            }
        });
    ui.add_space(4.0);
}

fn pct(f: f64) -> String {
    format!("{:.0} %", f * 100.0)
}
