//! Alerts page. PENDING PORT: the egui version is
//! `git show main:crates/omnihw-app/src/tabs/alerts.rs`.

use crate::app::OmniApp;
use gpui_kit::{AnyElement, Context, Window};

/// Selections, filters and edit buffers of this page.
#[derive(Default)]
pub struct ViewState {}

/// Number of alerts currently firing (for the actions bar indicator).
pub fn firing_count(snap: &crate::sampler::Snapshot) -> usize {
    (0..snap.alerts.rules().len())
        .filter(|&i| snap.alerts.is_active(i))
        .count()
}

pub fn show(app: &mut OmniApp, _window: &mut Window, cx: &mut Context<OmniApp>) -> AnyElement {
    super::pending(app.tab, cx)
}
