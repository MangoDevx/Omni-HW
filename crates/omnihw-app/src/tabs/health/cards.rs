//! Finding cards (detail, fix steps checklist, Fix and firmware buttons) and the
//! "Restart into UEFI" confirmation.

use super::{card_id, level_color};
use crate::heat::{ORANGE, RED};
use crate::tabs::{pill, temp};
use egui::collapsing_header::CollapsingState;
use egui::{Color32, RichText, Sense, Ui, vec2};
use omnihw_core::diagnostics::{Finding, Level};

/// A finding: a softly shaded panel with a thin accent edge in the level colour, no border.
/// Dismissed cards show their category (they're listed outside it) and a Restore button.
pub(super) fn finding_card(ui: &mut Ui, check: &str, f: &Finding, uefi: bool, dismissed: bool) {
    // Scope widget ids to the finding: otherwise, when a card is dismissed, the next card's
    // buttons inherit its position-based ids and the same click.
    ui.push_id(("health_finding", check, &f.title), |ui| {
        finding_card_contents(ui, check, f, uefi, dismissed)
    });
}

fn finding_card_contents(ui: &mut Ui, check: &str, f: &Finding, uefi: bool, dismissed: bool) {
    let color = level_color(f.level);
    // Warnings and critical issues start expanded; suggestions and dismissed ones collapsed.
    let default_open = f.level != Level::Info && !dismissed;
    let resp = egui::Frame::new()
        .fill(ui.visuals().faint_bg_color)
        .corner_radius(6)
        .inner_margin(egui::Margin {
            left: 14,
            right: 10,
            top: 8,
            bottom: 8,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            CollapsingState::load_with_default_open(ui.ctx(), card_id(f), default_open)
                .show_header(ui, |ui| {
                    ui.label(RichText::new(&f.title).size(15.0).strong());
                    pill(ui, f.level.name(), color, true);
                    if f.security {
                        pill(ui, "Security setting", ORANGE, true);
                    }
                    if dismissed {
                        ui.weak(f.category.name());
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let key = crate::dismissed::key(check, f);
                        if dismissed {
                            if ui
                                .small_button("Restore")
                                .on_hover_text("Show this in its category again")
                                .clicked()
                            {
                                crate::dismissed::restore(&key);
                            }
                        } else if ui
                            .small_button("Dismiss")
                            .on_hover_text(
                                "Hide this from the Health Check summary and the Overview. \
                                 See or restore it with the Dismissed button at the top.",
                            )
                            .clicked()
                        {
                            crate::dismissed::dismiss(key);
                        }
                    });
                })
                .body(|ui| finding_body(ui, f, uefi));
        });
    let r = resp.response.rect;
    ui.painter().rect_filled(
        egui::Rect::from_min_size(r.min, vec2(3.0, r.height())),
        egui::CornerRadius {
            nw: 6,
            sw: 6,
            ne: 0,
            se: 0,
        },
        color,
    );
    ui.add_space(6.0);
}

fn finding_body(ui: &mut Ui, f: &Finding, uefi: bool) {
    ui.add(egui::Label::new(&f.detail).wrap());
    if f.security {
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            security_icon(ui);
            ui.label(
                RichText::new(
                    "Security setting: only change it if you understand the protection you would be giving up.",
                )
                .color(crate::heat::ink(ui.visuals(), ORANGE)),
            );
        });
    }
    if !f.steps.is_empty() {
        steps_checklist(ui, f);
    }
    if let Some(a) = &f.fix {
        ui.add_space(4.0);
        crate::tabs::power::fix_button(ui, a);
    }
    if f.bios && uefi {
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            if ui
                .button("Restart into UEFI / BIOS…")
                .on_hover_text(
                    "Restarts the PC straight into the firmware setup, no key presses needed. Asks for administrator rights.",
                )
                .clicked()
            {
                set_confirm_restart(ui.ctx(), true);
            }
        });
    }
}

/// Small orange warning triangle with "!", drawn so it doesn't depend on font glyphs.
fn security_icon(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
    let c = rect.center();
    let r = 7.0;
    ui.painter().add(egui::Shape::convex_polygon(
        vec![
            c + vec2(0.0, -r),
            c + vec2(r, r * 0.8),
            c + vec2(-r, r * 0.8),
        ],
        ORANGE,
        egui::Stroke::NONE,
    ));
    ui.painter().text(
        c + vec2(0.0, 1.5),
        egui::Align2::CENTER_CENTER,
        "!",
        egui::FontId::proportional(10.0),
        Color32::BLACK,
    );
}

/// The steps as a checklist: tick steps off as you go ("2 of 4 done"). Ticks only live in
/// memory for this session.
fn steps_checklist(ui: &mut Ui, f: &Finding) {
    let id = card_id(f).with("steps");
    let mut done: Vec<bool> = temp::get(ui.ctx(), id).unwrap_or_else(|| vec![false; f.steps.len()]);
    done.resize(f.steps.len(), false);
    let n_done = done.iter().filter(|&&d| d).count();
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new("HOW TO FIX").small().strong().weak());
        if f.steps.len() > 1 {
            ui.weak(format!("· {n_done} of {} done", f.steps.len()));
        }
    });
    let total = f.steps.len();
    for (i, s) in f.steps.iter().enumerate() {
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut done[i], "")
                .on_hover_text(format!("Mark step {} of {total} as done", i + 1));
            let text = RichText::new(format!("{}. {}", i + 1, s.text));
            ui.add(egui::Label::new(if done[i] { text.weak() } else { text }).wrap());
            if let Some(link) = &s.link {
                let url = link.url();
                if ui.small_button(link.label()).on_hover_text(&url).clicked() {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(url));
                }
            }
        });
    }
    temp::set(ui.ctx(), id, done);
}

fn confirm_id() -> egui::Id {
    egui::Id::new("health_confirm_uefi_restart")
}

fn set_confirm_restart(ctx: &egui::Context, open: bool) {
    temp::set(ctx, confirm_id(), open);
}

/// "Restart into UEFI" confirmation, shown over the Health tab when requested.
pub(super) fn restart_dialog(ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    if !temp::get::<bool>(&ctx, confirm_id()).unwrap_or(false) {
        return;
    }
    let error_id = confirm_id().with("error");
    let busy_id = confirm_id().with("busy");
    let busy = temp::get::<bool>(&ctx, busy_id).unwrap_or(false);
    let modal = egui::Modal::new(confirm_id()).show(&ctx, |ui| {
        ui.set_max_width(420.0);
        ui.label(
            RichText::new("Restart into UEFI / BIOS?")
                .size(16.0)
                .strong(),
        );
        ui.add_space(4.0);
        ui.label(
            "The PC restarts immediately and opens the firmware setup. Save your work and close \
             other programs first: anything unsaved will be lost.",
        );
        ui.add_space(2.0);
        ui.weak(
            "Windows asks for administrator rights to do this. In the setup, save and exit \
             (usually F10) to boot back into Windows.",
        );
        if let Some(err) = temp::get::<String>(&ctx, error_id) {
            ui.add_space(4.0);
            ui.colored_label(crate::heat::ink(ui.visuals(), RED), err);
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!busy, egui::Button::new("Restart now"))
                .clicked()
            {
                // The UAC prompt blocks the calling thread until answered: keep it off the UI.
                temp::set(&ctx, busy_id, true);
                let ctx = ctx.clone();
                std::thread::spawn(move || {
                    let result = crate::elevate::restart_to_firmware();
                    temp::set(&ctx, busy_id, false);
                    match result {
                        Ok(()) => set_confirm_restart(&ctx, false),
                        Err(e) => temp::set(&ctx, error_id, e),
                    }
                    ctx.request_repaint();
                });
            }
            if ui.button("Cancel").clicked() {
                set_confirm_restart(&ctx, false);
            }
        });
    });
    if modal.should_close() {
        set_confirm_restart(&ctx, false);
    }
    if !temp::get::<bool>(&ctx, confirm_id()).unwrap_or(false) {
        temp::take::<String>(&ctx, error_id);
    }
}
