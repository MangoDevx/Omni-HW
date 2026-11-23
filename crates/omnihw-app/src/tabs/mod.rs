//! Pages. Each is a module with a `show` function that builds the page's elements from the
//! latest snapshot, and (where the page has any) a `ViewState` for what it remembers between
//! redraws. Shared building blocks live in [`crate::ui`].

pub mod about;
pub mod alerts;
pub mod bottleneck;
pub mod caches;
pub mod compare;
pub mod cpu;
pub mod eventlog;
pub mod gpu;
pub mod health;
pub mod lansec;
pub mod mainboard;
pub mod memory;
pub mod network;
pub mod overview;
#[cfg(test)]
pub mod page_tests;
pub mod power;
pub mod recording;
pub mod sensors;
pub mod settings;
pub mod storage;

use crate::app::{OmniApp, Tab};
use gpui_kit::{AnyElement, Context, SharedString, Window};
use std::collections::HashSet;

/// What the pages remember between redraws (selections, filters, edit buffers).
#[derive(Default)]
pub struct Views {
    /// Collapsible sections that were toggled away from their default (see [`ui::is_open`]).
    pub toggled: HashSet<SharedString>,
    pub health: health::ViewState,
    pub gpu: gpu::ViewState,
    pub network: network::ViewState,
    pub recording: recording::ViewState,
    pub alerts: alerts::ViewState,
    pub eventlog: eventlog::ViewState,
    pub settings: settings::ViewState,
}

/// Build the open page. Only called once the static hardware info is ready, so pages may
/// rely on `app.sampler.info()` being `Some`.
pub fn show(app: &mut OmniApp, window: &mut Window, cx: &mut Context<OmniApp>) -> AnyElement {
    match app.tab {
        Tab::Overview => overview::show(app, window, cx),
        Tab::Health => health::show(app, window, cx),
        Tab::Cpu => cpu::show(app, window, cx),
        Tab::Caches => caches::show(app, window, cx),
        Tab::Mainboard => mainboard::show(app, window, cx),
        Tab::Memory => memory::show(app, window, cx),
        Tab::Gpu => gpu::show(app, window, cx),
        Tab::Storage => storage::show(app, window, cx),
        Tab::Network => network::show(app, window, cx),
        Tab::LanSecurity => lansec::show(app, window, cx),
        Tab::Power => power::show(app, window, cx),
        Tab::Sensors => sensors::show(app, window, cx),
        Tab::Recording => recording::show(app, window, cx),
        Tab::Alerts => alerts::show(app, window, cx),
        Tab::EventLog => eventlog::show(app, window, cx),
        Tab::About => about::show(app, window, cx),
        Tab::Settings => settings::show(app, window, cx),
    }
}
