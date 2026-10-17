//! The details pane (a finding with its fix steps checklist, Fix and firmware buttons; a
//! category's passed checks; the dismissed list) and the "Restart into UEFI" confirmation.

use super::{GREEN, Item, Passed, Sel, View, level_color, select};
use crate::heat::{ORANGE, RED};
use crate::tabs::{pill, temp};
use egui::{Color32, RichText, Sense, Ui, vec2};
use omnihw_core::diagnostics::{Category, CheckResult, Finding, Outcome};

/// Text in the details pane stops at this width, so lines stay easy to read.
const MAX_WIDTH: f32 = 680.0;

/// The details of the selected list entry.
pub(super) fn detail(ui: &mut Ui, view: &View, sel: Option<&Sel>, uefi: bool) {
    ui.set_max_width(ui.available_width().min(MAX_WIDTH));
    match sel {
        Some(Sel::Finding(k)) => {
            if let Some(i) = view.finding(k) {
                finding_detail(ui, i, uefi, false);
            }
        }
        Some(Sel::DismissedFinding(k)) => {
            if ui
                .small_button("‹ All dismissed")
                .on_hover_text("Back to the dismissed list")
                .clicked()
            {
                select(ui.ctx(), Sel::Dismissed);
            }
            ui.add_space(6.0);
            if let Some(i) = view.dismissed_finding(k) {
                finding_detail(ui, i, uefi, true);
            }
        }
        Some(Sel::Passed(c)) => {
            if let Some(p) = view.passed.iter().find(|p| p.cat == *c) {
                passed_detail(ui, p);
            }
        }
        Some(Sel::Dismissed) => dismissed_list(ui, &view.dismissed),
        None => {
            ui.weak("No checks have run yet.");
        }
    }
}

/// A finding: title, level and category, what was measured, how to fix it, then Dismiss
/// (or Restore for a dismissed one).
fn finding_detail(ui: &mut Ui, item: &Item, uefi: bool, dismissed: bool) {
    let f = item.f;
    // Scope widget ids to the finding: otherwise, when one is dismissed, the next one's
    // buttons inherit its position-based ids and the same click.
    ui.push_id(("health_finding", item.check, &f.title), |ui| {
        ui.add(egui::Label::new(RichText::new(&f.title).size(18.0).strong()).wrap());
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            pill(ui, f.level.name(), level_color(f.level), true);
            pill(ui, f.category.name(), Color32::GRAY, true);
            if f.security {
                pill(ui, "Security setting", ORANGE, true);
            }
            if dismissed {
                pill(ui, "Dismissed", Color32::GRAY, true);
            }
        });
        ui.add_space(10.0);
        finding_body(ui, f, uefi);
        ui.add_space(16.0);
        ui.separator();
        ui.add_space(4.0);
        if dismissed {
            if ui
                .button("Restore")
                .on_hover_text("Show this in the list and the summary again")
                .clicked()
            {
                crate::dismissed::restore(&item.key);
                select(ui.ctx(), Sel::Finding(item.key.clone()));
            }
        } else {
            ui.horizontal_wrapped(|ui| {
                if ui
                    .button("Dismiss")
                    .on_hover_text(
                        "Hide this from the Health Check summary and the Overview. \
                         See or restore it under Dismissed at the bottom of the list.",
                    )
                    .clicked()
                {
                    crate::dismissed::dismiss(item.key.clone());
                }
                ui.weak("Not relevant to you? Hide it.");
            });
        }
    });
}

/// A category's passed (green OK + detail) and not-applicable (reason) checks.
fn passed_detail(ui: &mut Ui, p: &Passed) {
    ui.label(
        RichText::new(format!("{} · passed checks", p.cat.name()))
            .size(18.0)
            .strong(),
    );
    let mut counts = format!("{} passed", p.passed);
    if p.skipped > 0 {
        counts.push_str(&format!(", {} not applicable to this PC", p.skipped));
    }
    ui.weak(counts);
    if let Some(note) = category_note(p.cat) {
        ui.weak(note);
    }
    ui.add_space(8.0);
    result_grid(ui, ("health_grid", p.cat.name()), &p.rows);
}

/// Dismissed findings: click one to read it again, Restore brings it back.
fn dismissed_list(ui: &mut Ui, items: &[Item]) {
    ui.label(RichText::new("Dismissed").size(18.0).strong());
    ui.horizontal_wrapped(|ui| {
        ui.weak("Hidden from the summary and the Overview. Restore one to show it again.");
        if items.len() > 1 && ui.small_button("Restore all").clicked() {
            crate::dismissed::restore_all();
        }
    });
    ui.add_space(8.0);
    for i in items {
        ui.push_id(("health_dismissed", &i.key), |ui| {
            ui.horizontal(|ui| {
                if ui
                    .small_button("Restore")
                    .on_hover_text("Show this in the list and the summary again")
                    .clicked()
                {
                    crate::dismissed::restore(&i.key);
                }
                let r = super::list::row(
                    ui,
                    false,
                    level_color(i.f.level),
                    &i.f.title,
                    i.f.category.name(),
                );
                if r.on_hover_text("Read it again").clicked() {
                    select(ui.ctx(), Sel::DismissedFinding(i.key.clone()));
                }
            });
        });
    }
}

fn category_note(cat: Category) -> Option<&'static str> {
    match cat {
        Category::Gaming => {
            Some("Settings that mainly affect games: frame rates, latency and GPU features.")
        }
        Category::Crashes => Some(
            "Blue screens, unexpected restarts and hardware errors from the Windows event log (last 30 days).",
        ),
        Category::Power => Some(
            "Power plan, sleep and device power-saving settings that can interrupt what you're doing.",
        ),
        _ => None,
    }
}

/// Passed (green OK + detail) and skipped (reason) checks as a compact table.
fn result_grid(ui: &mut Ui, id: impl std::hash::Hash + std::fmt::Debug, rows: &[&CheckResult]) {
    if rows.is_empty() {
        return;
    }
    egui::Grid::new(id)
        .num_columns(3)
        .spacing([12.0, 4.0])
        .striped(true)
        .show(ui, |ui| {
            for r in rows {
                match &r.outcome {
                    Outcome::Pass(detail) => {
                        ui.colored_label(crate::heat::ink(ui.visuals(), GREEN), "OK");
                        ui.label(r.name);
                        ui.weak(detail);
                    }
                    Outcome::Skipped(why) => {
                        ui.weak("–");
                        ui.label(r.name);
                        ui.weak(why);
                    }
                    Outcome::Findings(_) => continue,
                }
                ui.end_row();
            }
        });
}

/// State id for a finding's checklist (stable across frames by title).
fn card_id(f: &Finding) -> egui::Id {
    egui::Id::new(("health_card", &f.title))
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
        ui.add_space(10.0);
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
