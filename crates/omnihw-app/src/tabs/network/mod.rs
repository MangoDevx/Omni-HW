//! Network page. PENDING PORT: the egui version is
//! `git show main:crates/omnihw-app/src/tabs/network/mod.rs`.

use crate::app::OmniApp;
use gpui_kit::{AnyElement, Context, Window};

/// Selections, filters and edit buffers of this page.
#[derive(Default)]
pub struct ViewState {}

pub fn show(app: &mut OmniApp, _window: &mut Window, cx: &mut Context<OmniApp>) -> AnyElement {
    super::pending(app.tab, cx)
}
