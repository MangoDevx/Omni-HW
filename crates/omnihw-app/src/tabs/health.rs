//! Health Check tab: runs the diagnostics rules and explains what they found.

use crate::heat::{GREEN, ORANGE, RED};
use crate::sampler::Snapshot;
use egui::collapsing_header::CollapsingState;
use egui::{Color32, RichText, Sense, Ui, vec2};
use omnihw_core::SystemInfo;
use omnihw_core::diagnostics::{self, Category, CheckResult, Context, Finding, Level, Outcome};
use std::time::{SystemTime, UNIX_EPOCH};

const BLUE: Color32 = Color32::from_rgb(66, 133, 244);

pub fn level_color(l: Level) -> Color32 {
    match l {
        Level::Info => BLUE,
        Level::Warning => ORANGE,
        Level::Critical => RED,
    }
}

/// Run all checks against the current data.
pub fn run(info: &SystemInfo, snap: &Snapshot) -> Vec<CheckResult> {
    let days = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() / 86400)
        .unwrap_or(0) as i64;
    let ctx = Context {
        info,
        sensors: &snap.table,
        today: omnihw_core::civil_from_days(days),
        uptime_secs: omnihw_core::win::uptime_secs(),
    };
    diagnostics::run(&ctx)
}

fn is_gaming(r: &CheckResult) -> bool {
    r.category == Category::Gaming
}

/// Counts of general (non-gaming) findings by level: (critical, warning, info).
pub fn counts(results: &[CheckResult]) -> (usize, usize, usize) {
    let general: Vec<CheckResult> = results.iter().filter(|r| !is_gaming(r)).cloned().collect();
    let f = diagnostics::findings(&general);
    let n = |l| f.iter().filter(|x| x.level == l).count();
    (n(Level::Critical), n(Level::Warning), n(Level::Info))
}

/// Number of findings in the Gaming category.
pub fn gaming_count(results: &[CheckResult]) -> usize {
    results
        .iter()
        .filter(|r| is_gaming(r))
        .map(|r| match &r.outcome {
            Outcome::Findings(f) => f.len(),
            _ => 0,
        })
        .sum()
}

fn plural(n: usize, s: &str) -> String {
    format!("{n} {s}{}", if n == 1 { "" } else { "s" })
}

/// "1 critical issue, 2 warnings" style summary of general findings, or None when nothing was found.
pub fn summary(results: &[CheckResult]) -> Option<(String, Color32)> {
    let (c, w, i) = counts(results);
    let mut parts = Vec::new();
    if c > 0 {
        parts.push(plural(c, "critical issue"));
    }
    if w > 0 {
        parts.push(plural(w, "warning"));
    }
    if i > 0 {
        parts.push(plural(i, "suggestion"));
    }
    let color = if c > 0 {
        RED
    } else if w > 0 {
        ORANGE
    } else {
        BLUE
    };
    (!parts.is_empty()).then(|| (parts.join(", "), color))
}

/// "2 gaming suggestions", if there are any.
pub fn gaming_summary(results: &[CheckResult]) -> Option<String> {
    let n = gaming_count(results);
    (n > 0).then(|| plural(n, "gaming suggestion"))
}

pub fn show(ui: &mut Ui, info: &SystemInfo, snap: &Snapshot) {
    let uefi = info.checkup.firmware.uefi == Some(true);
    restart_dialog(ui);
    let results = run(info, snap);
    let (gaming, general): (Vec<CheckResult>, Vec<CheckResult>) =
        results.iter().cloned().partition(is_gaming);
    let findings = diagnostics::findings(&general);
    let passed: Vec<_> = general
        .iter()
        .filter(|r| matches!(r.outcome, Outcome::Pass(_)))
        .collect();
    let skipped: Vec<_> = general
        .iter()
        .filter(|r| matches!(r.outcome, Outcome::Skipped(_)))
        .collect();

    ui.label(RichText::new("Health Check").size(20.0).strong());
    ui.horizontal_wrapped(|ui| {
        match summary(&results) {
            Some((text, color)) => ui.label(
                RichText::new(text)
                    .size(15.0)
                    .color(crate::heat::ink(ui.visuals(), color))
                    .strong(),
            ),
            None => ui.label(
                RichText::new("No problems found")
                    .size(15.0)
                    .color(crate::heat::ink(ui.visuals(), GREEN))
                    .strong(),
            ),
        };
        if let Some(g) = gaming_summary(&results) {
            ui.label(
                RichText::new(format!("·  {g} below"))
                    .size(15.0)
                    .color(crate::heat::ink(ui.visuals(), BLUE)),
            );
        }
    });
    let general_found = general.len() - passed.len() - skipped.len();
    ui.weak(format!(
        "{} general checks: {general_found} found something, {} passed, {} not applicable. Live checks use the last ~2 minutes of sensor data and update as you use the PC.",
        general.len(),
        passed.len(),
        skipped.len()
    ));
    ui.add_space(10.0);

    if !findings.is_empty() {
        ui.horizontal(|ui| {
            if ui.small_button("Expand all").clicked() {
                set_all_open(ui, &findings, true);
            }
            if ui.small_button("Collapse all").clicked() {
                set_all_open(ui, &findings, false);
            }
        });
        ui.add_space(4.0);
    }
    for f in &findings {
        finding_card(ui, f, uefi);
    }

    gaming_section(ui, &gaming, uefi);

    // Sections hold no suggestions, so they start collapsed.
    section(
        ui,
        "health_passed",
        "Passed",
        passed.len().to_string(),
        GREEN,
        false,
        |ui| result_grid(ui, "passed", &passed),
    );
    section(
        ui,
        "health_skipped",
        "Not checked",
        skipped.len().to_string(),
        Color32::GRAY,
        false,
        |ui| result_grid(ui, "skipped", &skipped),
    );
}

/// A collapsible tree section with a larger bold title and a coloured pill
/// (a count or a short status).
fn section(
    ui: &mut Ui,
    id: &str,
    title: &str,
    pill_text: String,
    color: Color32,
    default_open: bool,
    body: impl FnOnce(&mut Ui),
) {
    CollapsingState::load_with_default_open(ui.ctx(), egui::Id::new(id), default_open)
        .show_header(ui, |ui| {
            ui.label(RichText::new(title).size(16.0).strong());
            pill(ui, pill_text, color, false);
        })
        .body(|ui| {
            ui.add_space(4.0);
            body(ui)
        });
    ui.add_space(4.0);
}

/// Gaming checks in their own collapsible section; the header shows the suggestion count in blue.
fn gaming_section(ui: &mut Ui, gaming: &[CheckResult], uefi: bool) {
    if gaming.is_empty() {
        return;
    }
    let findings = diagnostics::findings(gaming);
    let (label, color) = if findings.is_empty() {
        ("all good".to_string(), GREEN)
    } else {
        (plural(findings.len(), "suggestion"), BLUE)
    };
    // Open only when there is something to act on.
    section(
        ui,
        "health_gaming",
        "Gaming",
        label,
        color,
        !findings.is_empty(),
        |ui| {
            ui.weak("Settings that mainly affect games: frame rates, latency and GPU features.");
            ui.add_space(6.0);
            for f in &findings {
                finding_card(ui, f, uefi);
            }
            let rest: Vec<&CheckResult> = gaming
                .iter()
                .filter(|r| !matches!(r.outcome, Outcome::Findings(_)))
                .collect();
            result_grid(ui, "gaming_rest", &rest);
        },
    );
}

/// Passed (green OK + detail) and skipped (reason) checks as a compact table.
fn result_grid(ui: &mut Ui, id: &str, rows: &[&CheckResult]) {
    egui::Grid::new(id)
        .num_columns(3)
        .spacing([12.0, 4.0])
        .striped(true)
        .show(ui, |ui| {
            for r in rows {
                match &r.outcome {
                    Outcome::Pass(detail) => {
                        ui.colored_label(crate::heat::ink(ui.visuals(), GREEN), "OK");
                        ui.label(format!("{} · {}", r.category.name(), r.name));
                        ui.weak(detail);
                    }
                    Outcome::Skipped(why) => {
                        ui.weak("–");
                        ui.label(format!("{} · {}", r.category.name(), r.name));
                        ui.weak(why);
                    }
                    Outcome::Findings(_) => continue,
                }
                ui.end_row();
            }
        });
}

/// Collapse state id for a finding card (stable across frames by title).
fn card_id(f: &Finding) -> egui::Id {
    egui::Id::new(("health_card", &f.title))
}

/// Open or close every finding card.
fn set_all_open(ui: &Ui, findings: &[&Finding], open: bool) {
    for f in findings {
        let mut st = CollapsingState::load_with_default_open(ui.ctx(), card_id(f), open);
        st.set_open(open);
        st.store(ui.ctx());
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

/// Small rounded tag with tinted fill, used for counts, levels and the security marker.
fn pill(ui: &mut Ui, text: impl Into<String>, color: Color32, small: bool) {
    let ink = crate::heat::ink(ui.visuals(), color);
    let mut t = RichText::new(text.into()).strong().color(ink);
    if small {
        t = t.small();
    }
    egui::Frame::new()
        .fill(color.gamma_multiply(0.16))
        .corner_radius(10)
        .inner_margin(egui::Margin::symmetric(7, 1))
        .show(ui, |ui| ui.label(t));
}

/// A finding: a softly shaded panel with a thin accent edge in the level colour, no border.
fn finding_card(ui: &mut Ui, f: &Finding, uefi: bool) {
    let color = level_color(f.level);
    // Warnings and critical issues start expanded; suggestions start collapsed.
    let default_open = f.level != Level::Info;
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
                    // Inside the Gaming section the category is implied.
                    if f.category != Category::Gaming {
                        ui.weak(f.category.name());
                    }
                    if f.security {
                        pill(ui, "Security setting", ORANGE, true);
                    }
                })
                .body(|ui| finding_body(ui, f, uefi));
        });
    let r = resp.response.rect;
    ui.painter().rect_filled(
        egui::Rect::from_min_size(r.min, vec2(3.0, r.height())),
        egui::CornerRadius { nw: 6, sw: 6, ne: 0, se: 0 },
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

/// The steps as a checklist: tick steps off as you go ("2 of 4 done"). Ticks only live in
/// memory for this session.
fn steps_checklist(ui: &mut Ui, f: &Finding) {
    let id = card_id(f).with("steps");
    let mut done: Vec<bool> = ui
        .ctx()
        .data(|d| d.get_temp(id))
        .unwrap_or_else(|| vec![false; f.steps.len()]);
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
    ui.ctx().data_mut(|d| d.insert_temp(id, done));
}

fn confirm_id() -> egui::Id {
    egui::Id::new("health_confirm_uefi_restart")
}

fn set_confirm_restart(ctx: &egui::Context, open: bool) {
    ctx.data_mut(|d| d.insert_temp(confirm_id(), open));
}

/// "Restart into UEFI" confirmation, shown over the Health tab when requested.
fn restart_dialog(ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    if !ctx.data(|d| d.get_temp::<bool>(confirm_id()).unwrap_or(false)) {
        return;
    }
    let error_id = confirm_id().with("error");
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
        if let Some(err) = ctx.data(|d| d.get_temp::<String>(error_id)) {
            ui.add_space(4.0);
            ui.colored_label(crate::heat::ink(ui.visuals(), RED), err);
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button("Restart now").clicked() {
                match crate::elevate::restart_to_firmware() {
                    Ok(()) => set_confirm_restart(&ctx, false),
                    Err(e) => ctx.data_mut(|d| {
                        d.insert_temp(error_id, e);
                    }),
                }
            }
            if ui.button("Cancel").clicked() {
                set_confirm_restart(&ctx, false);
            }
        });
    });
    if modal.should_close() {
        set_confirm_restart(&ctx, false);
    }
    if !ctx.data(|d| d.get_temp::<bool>(confirm_id()).unwrap_or(false)) {
        ctx.data_mut(|d| d.remove::<String>(error_id));
    }
}
