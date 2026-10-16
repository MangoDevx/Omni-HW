//! Power changes requested by the page (or a Health Check "Fix"): kept in egui memory
//! during the frame, then confirmed if disruptive and run on a worker thread by [`dispatch`],
//! outside the snapshot lock.

use crate::elevate::elevated;
use crate::sampler::Sampler;
use crate::tabs::temp;
use egui::{Id, RichText, Ui};
use omnihw_core::power::Action;

const PENDING: &str = "power_pending";
const CONFIRM: &str = "power_confirm";
pub(super) const REFRESH: &str = "power_refresh";
pub(super) const QUERY: &str = "power_query";

/// Ask for `action` to be applied (after a confirmation if it's disruptive).
pub fn request(ctx: &egui::Context, action: Action) {
    temp::set(ctx, Id::new(PENDING), Some(action));
}

pub(super) fn flag(ctx: &egui::Context, key: &str) {
    temp::set(ctx, Id::new(key), true);
}

fn take_flag(ctx: &egui::Context, key: &str) -> bool {
    temp::take::<bool>(ctx, Id::new(key)).unwrap_or(false)
}

fn take_action(ctx: &egui::Context, key: &str) -> Option<Action> {
    temp::take::<Option<Action>>(ctx, Id::new(key)).flatten()
}

pub(super) fn admin_hint(a: &Action) -> &'static str {
    if a.needs_admin() && !elevated() {
        "Asks for administrator permission"
    } else {
        "Applies the change now"
    }
}

/// A "Fix" button for a Health Check finding.
pub fn fix_button(ui: &mut Ui, action: &Action) {
    if ui
        .button(RichText::new(format!("Fix: {}", action.label())).strong())
        .on_hover_text(admin_hint(action))
        .clicked()
    {
        request(ui.ctx(), action.clone());
    }
}

/// Run requested changes and show the confirmation dialog. Call once per frame, outside the
/// snapshot lock. Returns a status message when a change finished.
pub fn dispatch(ctx: &egui::Context, sampler: &Sampler) -> Option<String> {
    if take_flag(ctx, REFRESH) {
        sampler.refresh_power();
    }
    if take_flag(ctx, QUERY) {
        sampler.power_query_admin();
    }
    if let Some(a) = take_action(ctx, PENDING) {
        if a.confirm_text().is_some() {
            temp::set(ctx, Id::new(CONFIRM), Some(a));
        } else {
            sampler.power_action(a);
        }
    }
    if let Some(a) = temp::get::<Option<Action>>(ctx, Id::new(CONFIRM)).flatten() {
        let mut close = None;
        egui::Modal::new(Id::new("power_confirm_modal")).show(ctx, |ui| {
            ui.set_max_width(420.0);
            ui.label(RichText::new(a.label()).size(16.0).strong());
            ui.add_space(6.0);
            ui.add(egui::Label::new(a.confirm_text().unwrap_or_default()).wrap());
            if a.needs_admin() && !elevated() {
                ui.add_space(4.0);
                ui.weak("Windows will ask for administrator permission.");
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button(RichText::new("Apply").strong()).clicked() {
                    close = Some(true);
                }
                if ui.button("Cancel").clicked() {
                    close = Some(false);
                }
            });
        });
        if let Some(apply) = close {
            take_action(ctx, CONFIRM);
            if apply {
                sampler.power_action(a);
            }
        }
    }
    sampler.snapshot().power_msg.take()
}
