//! Power page. PENDING PORT: the egui version is
//! `git show main:crates/omnihw-app/src/tabs/power/` (mod.rs, actions.rs).
//!
//! The helpers shared with the Health Check page ([`request`], [`fix_button`],
//! [`admin_hint`]) are ported; the page itself is not.

use crate::app::OmniApp;
use crate::elevate::elevated;
use crate::ui;
use gpui_kit::component::button::Button;
use gpui_kit::{AnyElement, Context, ElementId, IntoElement, Window};
use omnihw_core::power::Action;

/// Selections, filters and edit buffers of this page.
#[derive(Default)]
pub struct ViewState {}

pub fn show(app: &mut OmniApp, _window: &mut Window, cx: &mut Context<OmniApp>) -> AnyElement {
    super::pending(app.tab, cx)
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
