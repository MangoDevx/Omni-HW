//! Recording page. PENDING PORT: the egui version is
//! `git show main:crates/omnihw-app/src/tabs/recording.rs`.

use crate::app::OmniApp;
use gpui_kit::{AnyElement, Context, Window};

/// Selections, filters and edit buffers of this page.
#[derive(Default)]
pub struct ViewState {}

/// Start or stop recording; returns a status message for the status bar.
pub fn toggle(sampler: &crate::sampler::Sampler) -> String {
    if sampler.snapshot().recording.is_some() {
        match sampler.stop_recording() {
            Some(p) => format!("Recording saved to {}", p.display()),
            None => "Not recording".into(),
        }
    } else {
        match sampler.start_recording(&crate::export::report_dir(), &crate::export::timestamp()) {
            Ok(p) => format!("Recording to {}", p.display()),
            Err(e) => e,
        }
    }
}

pub fn show(app: &mut OmniApp, _window: &mut Window, cx: &mut Context<OmniApp>) -> AnyElement {
    super::pending(app.tab, cx)
}
