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
        finding_card(ui, f);
    }

    gaming_section(ui, &gaming);

    egui::CollapsingHeader::new(RichText::new(format!("Passed ({})", passed.len())).strong())
        .default_open(findings.is_empty())
        .show(ui, |ui| result_grid(ui, "passed", &passed));
    egui::CollapsingHeader::new(RichText::new(format!("Not checked ({})", skipped.len())).strong())
        .show(ui, |ui| result_grid(ui, "skipped", &skipped));
}

/// Gaming checks in their own collapsible section; the header shows the suggestion count in blue.
fn gaming_section(ui: &mut Ui, gaming: &[CheckResult]) {
    if gaming.is_empty() {
        return;
    }
    let findings = diagnostics::findings(gaming);
    // A plain expandable header like "Passed"; the finding cards appear inside when opened.
    CollapsingState::load_with_default_open(ui.ctx(), egui::Id::new("health_gaming"), false)
        .show_header(ui, |ui| {
            ui.label(RichText::new("Gaming").strong());
            if findings.is_empty() {
                ui.label(RichText::new("– all good").color(crate::heat::ink(ui.visuals(), GREEN)));
            } else {
                ui.label(
                    RichText::new(format!("– {}", plural(findings.len(), "suggestion")))
                        .color(crate::heat::ink(ui.visuals(), BLUE))
                        .strong(),
                );
            }
        })
        .body(|ui| {
            ui.weak("Settings that mainly affect games: frame rates, latency and GPU features.");
            ui.add_space(6.0);
            for f in &findings {
                finding_card(ui, f);
            }
            let rest: Vec<&CheckResult> = gaming
                .iter()
                .filter(|r| !matches!(r.outcome, Outcome::Findings(_)))
                .collect();
            result_grid(ui, "gaming_rest", &rest);
        });
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

fn finding_card(ui: &mut Ui, f: &Finding) {
    let color = level_color(f.level);
    // Warnings and critical issues start expanded; suggestions start collapsed.
    let default_open = f.level != Level::Info;
    egui::Frame::group(ui.style())
        .inner_margin(12)
        .corner_radius(8)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            CollapsingState::load_with_default_open(ui.ctx(), card_id(f), default_open)
                .show_header(ui, |ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(4.0, 20.0), Sense::hover());
                    ui.painter().rect_filled(r, 2, color);
                    ui.label(
                        RichText::new(f.level.name().to_uppercase())
                            .small()
                            .strong()
                            .color(crate::heat::ink(ui.visuals(), color)),
                    );
                    ui.weak(f.category.name());
                    ui.label(RichText::new(&f.title).size(16.0).strong());
                    if f.security {
                        security_icon(ui);
                        ui.label(
                            RichText::new("Security setting")
                                .small()
                                .strong()
                                .color(crate::heat::ink(ui.visuals(), ORANGE)),
                        );
                    }
                })
                .body(|ui| finding_body(ui, f));
        });
    ui.add_space(8.0);
}

fn finding_body(ui: &mut Ui, f: &Finding) {
    ui.add(egui::Label::new(&f.detail).wrap());
    if f.security {
        ui.add_space(4.0);
        egui::Frame::new()
            .fill(ORANGE.gamma_multiply(0.15))
            .stroke(egui::Stroke::new(1.0, ORANGE))
            .corner_radius(6)
            .inner_margin(8)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    security_icon(ui);
                    ui.label(
                        RichText::new(
                            "This is a security setting. Only change it if you know what you are doing \
                             and understand the protection you would be giving up.",
                        )
                        .color(crate::heat::ink(ui.visuals(), ORANGE)),
                    );
                });
            });
    }
    if !f.tips.is_empty() {
        ui.add_space(4.0);
        ui.label(RichText::new("How to fix").strong());
        for t in &f.tips {
            ui.horizontal_wrapped(|ui| {
                ui.label("•");
                ui.add(egui::Label::new(t).wrap());
            });
        }
    }
}
