//! Health Check tab: runs the diagnostics rules and explains what they found.

mod cards;

use super::{page_title, pill, plural};
use crate::heat::{BLUE, GREEN, ORANGE, RED};
use crate::sampler::Snapshot;
use cards::{finding_card, restart_dialog};
use egui::collapsing_header::CollapsingState;
use egui::{Color32, RichText, Ui};
use omnihw_core::SystemInfo;
use omnihw_core::diagnostics::{self, Category, CheckResult, Context, Finding, Level, Outcome};
use std::time::{SystemTime, UNIX_EPOCH};

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
        power: snap.power.as_ref(),
    };
    diagnostics::run(&ctx)
}

/// Health Check results for the Health tab and the Overview banner. The checks only see data
/// that changes with a new sample, new power info or a (un)dismissal, so they run then rather
/// than on every frame (hovering alone repaints at the display rate).
#[derive(Default)]
pub struct HealthCache {
    /// (sample count, power revision, dismissed revision, show gaming) the report was built for.
    key: Option<(u64, u64, u64, bool)>,
    report: Report,
}

impl HealthCache {
    /// With `show_gaming` off, the Gaming category is left out entirely (findings, passed
    /// checks and dismissed gaming findings).
    pub fn get(&mut self, info: &SystemInfo, snap: &Snapshot, show_gaming: bool) -> &Report {
        let key = (
            snap.samples,
            snap.power_rev,
            crate::dismissed::revision(),
            show_gaming,
        );
        if self.key != Some(key) {
            let mut all = run(info, snap);
            if !show_gaming {
                all.retain(|r| !is_gaming(r));
            }
            self.report = Report::new(all);
            self.key = Some(key);
        }
        &self.report
    }
}

/// Check results with dismissed findings set aside, plus their summary lines.
#[derive(Default)]
pub struct Report {
    /// Every check, without dismissed findings.
    pub results: Vec<CheckResult>,
    /// Dismissed findings with the name of their check.
    pub dismissed: Vec<(&'static str, Finding)>,
    /// "1 critical issue, 2 warnings" and its colour; None when nothing general was found.
    pub summary: Option<(String, Color32)>,
    /// "2 gaming suggestions", if there are any.
    pub gaming_summary: Option<String>,
}

impl Report {
    fn new(all: Vec<CheckResult>) -> Report {
        let (results, dismissed) = crate::dismissed::split(all);
        let gaming = gaming_count(&results);
        Report {
            summary: summary(&results),
            gaming_summary: (gaming > 0).then(|| plural(gaming, "gaming suggestion")),
            results,
            dismissed,
        }
    }
}

fn is_gaming(r: &CheckResult) -> bool {
    r.category == Category::Gaming
}

fn findings_of(r: &CheckResult) -> &[Finding] {
    match &r.outcome {
        Outcome::Findings(f) => f,
        _ => &[],
    }
}

/// Counts of general (non-gaming) findings by level: (critical, warning, info).
fn counts(results: &[CheckResult]) -> (usize, usize, usize) {
    let mut n = (0, 0, 0);
    for f in results
        .iter()
        .filter(|r| !is_gaming(r))
        .flat_map(findings_of)
    {
        match f.level {
            Level::Critical => n.0 += 1,
            Level::Warning => n.1 += 1,
            Level::Info => n.2 += 1,
        }
    }
    n
}

/// Number of findings in the Gaming category.
fn gaming_count(results: &[CheckResult]) -> usize {
    results
        .iter()
        .filter(|r| is_gaming(r))
        .map(|r| findings_of(r).len())
        .sum()
}

/// "1 critical issue, 2 warnings" style summary of general findings, or None when nothing was found.
fn summary(results: &[CheckResult]) -> Option<(String, Color32)> {
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

/// Findings of these results with the name of the check that produced them, most severe first.
fn named_findings(results: &[CheckResult]) -> Vec<(&'static str, &Finding)> {
    let mut out: Vec<(&'static str, &Finding)> = results
        .iter()
        .flat_map(|r| findings_of(r).iter().map(move |x| (r.name, x)))
        .collect();
    out.sort_by_key(|(_, f)| std::cmp::Reverse(f.level));
    out
}

pub fn show(ui: &mut Ui, info: &SystemInfo, report: &Report) {
    let uefi = info.checkup.firmware.uefi == Some(true);
    restart_dialog(ui);
    let Report {
        results,
        dismissed,
        summary,
        gaming_summary,
    } = report;
    let findings = named_findings(results);
    let (mut general, mut passed, mut skipped, mut found) = (0, 0, 0, 0);
    for r in results.iter().filter(|r| !is_gaming(r)) {
        general += 1;
        match &r.outcome {
            Outcome::Pass(_) => passed += 1,
            Outcome::Skipped(_) => skipped += 1,
            Outcome::Findings(f) if !f.is_empty() => found += 1,
            Outcome::Findings(_) => {}
        }
    }

    page_title(ui, "Health Check");
    ui.horizontal_wrapped(|ui| {
        let (text, color) = match summary {
            Some((text, color)) => (text.as_str(), *color),
            None => ("No problems found", GREEN),
        };
        ui.label(
            RichText::new(text)
                .size(15.0)
                .color(crate::heat::ink(ui.visuals(), color))
                .strong(),
        );
        if let Some(g) = gaming_summary {
            ui.label(
                RichText::new(format!("·  {g} below"))
                    .size(15.0)
                    .color(crate::heat::ink(ui.visuals(), BLUE)),
            );
        }
    });
    let mut stats = format!(
        "{general} general checks: {found} found something, {passed} passed, {skipped} not applicable"
    );
    if !dismissed.is_empty() {
        stats.push_str(&format!(", {} dismissed", dismissed.len()));
    }
    ui.weak(format!(
        "{stats}. Live checks use the last ~2 minutes of sensor data and update as you use the PC."
    ));
    ui.add_space(10.0);

    let jump_to_dismissed = toolbar(ui, &findings, dismissed);
    ui.add_space(4.0);

    let mut rows: Vec<&CheckResult> = Vec::new();
    for cat in Category::ALL {
        rows.clear();
        rows.extend(results.iter().filter(|r| r.category == cat));
        if !rows.is_empty() {
            category_section(ui, cat, &rows, &findings, uefi);
        }
    }
    dismissed_section(ui, dismissed, uefi, jump_to_dismissed);
}

/// Which sections the expand/collapse buttons open.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Fold {
    /// Every category, every card and the Dismissed section.
    All,
    /// Only categories with findings (and their cards); everything else closes.
    Findings,
    /// Everything closed.
    None,
}

/// Expand/collapse buttons and the "Dismissed (n)" shortcut. Returns true on the frame the
/// shortcut is clicked, so the Dismissed section can open and scroll into view.
fn toolbar(
    ui: &mut Ui,
    findings: &[(&'static str, &Finding)],
    dismissed: &[(&'static str, Finding)],
) -> bool {
    let mut jump = false;
    ui.horizontal_wrapped(|ui| {
        if ui
            .small_button("Expand all")
            .on_hover_text("Open every category and issue")
            .clicked()
        {
            fold(ui, findings, dismissed, Fold::All);
        }
        if ui
            .small_button("Expand issues only")
            .on_hover_text("Open only the categories with issues or suggestions to fix")
            .clicked()
        {
            fold(ui, findings, dismissed, Fold::Findings);
        }
        if ui.small_button("Collapse all").clicked() {
            fold(ui, findings, dismissed, Fold::None);
        }
        ui.separator();
        let n = dismissed.len();
        jump = ui
            .add_enabled(n > 0, egui::Button::new(format!("Dismissed ({n})")).small())
            .on_hover_text("Show the issues you dismissed so you can restore them")
            .on_disabled_hover_text("Nothing dismissed. Use Dismiss on an issue to hide it.")
            .clicked();
    });
    jump
}

/// A collapsible tree section with a larger bold title and a coloured pill
/// (a count or a short status). Returns the header's response.
fn section(
    ui: &mut Ui,
    id: egui::Id,
    title: &str,
    pill_text: String,
    color: Color32,
    default_open: bool,
    body: impl FnOnce(&mut Ui),
) -> egui::Response {
    let (_, header, _) = CollapsingState::load_with_default_open(ui.ctx(), id, default_open)
        .show_header(ui, |ui| {
            ui.label(RichText::new(title).size(16.0).strong());
            pill(ui, pill_text, color, false);
        })
        .body(|ui| {
            ui.add_space(4.0);
            body(ui)
        });
    ui.add_space(4.0);
    header.response
}

/// One category: its findings as cards, then its passed / not-checked checks. Opens by default
/// only when it has findings; the pill shows the count in the colour of the worst one.
fn category_section(
    ui: &mut Ui,
    cat: Category,
    rows: &[&CheckResult],
    all_findings: &[(&'static str, &Finding)],
    uefi: bool,
) {
    let findings: Vec<(&'static str, &Finding)> = all_findings
        .iter()
        .copied()
        .filter(|(_, f)| f.category == cat)
        .collect();
    let (label, color) = match findings.iter().map(|(_, f)| f.level).max() {
        Some(Level::Info) => (plural(findings.len(), "suggestion"), BLUE),
        Some(l) => (plural(findings.len(), "issue"), level_color(l)),
        None if rows
            .iter()
            .all(|r| matches!(r.outcome, Outcome::Skipped(_))) =>
        {
            ("not checked".to_string(), Color32::GRAY)
        }
        None => ("all good".to_string(), GREEN),
    };
    section(
        ui,
        cat_id(cat, !findings.is_empty()),
        cat.name(),
        label,
        color,
        !findings.is_empty(),
        |ui| {
            if let Some(note) = category_note(cat) {
                ui.weak(note);
                ui.add_space(6.0);
            }
            for (name, f) in &findings {
                finding_card(ui, name, f, uefi, false);
            }
            let rest: Vec<&CheckResult> = rows
                .iter()
                .copied()
                .filter(|r| !matches!(r.outcome, Outcome::Findings(_)))
                .collect();
            result_grid(ui, ("health_grid", cat.name()), &rest);
        },
    );
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

/// Dismissed findings as full cards (so they can be read again), each with a Restore button.
/// `jump` opens the section and scrolls it into view.
fn dismissed_section(ui: &mut Ui, dismissed: &[(&'static str, Finding)], uefi: bool, jump: bool) {
    if dismissed.is_empty() {
        return;
    }
    let id = egui::Id::new(DISMISSED_ID);
    if jump {
        set_open(ui, id, true);
    }
    let header = section(
        ui,
        id,
        "Dismissed",
        dismissed.len().to_string(),
        Color32::GRAY,
        false,
        |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.weak("Hidden from the summary and the Overview. Restore one to show it again.");
                if dismissed.len() > 1 && ui.small_button("Restore all").clicked() {
                    crate::dismissed::restore_all();
                }
            });
            ui.add_space(4.0);
            for (name, f) in dismissed {
                finding_card(ui, name, f, uefi, true);
            }
        },
    );
    if jump {
        header.scroll_to_me(Some(egui::Align::TOP));
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

/// Collapse state id for a finding card (stable across frames by title).
fn card_id(f: &Finding) -> egui::Id {
    egui::Id::new(("health_card", &f.title))
}

/// Collapse state id for a category. Includes whether it has findings, so a category whose
/// findings arrive after the first frame (e.g. power data) still opens by default.
fn cat_id(cat: Category, has_findings: bool) -> egui::Id {
    egui::Id::new(("health_cat", cat.name(), has_findings))
}

const DISMISSED_ID: &str = "health_dismissed";

fn set_open(ui: &Ui, id: egui::Id, open: bool) {
    let mut st = CollapsingState::load_with_default_open(ui.ctx(), id, open);
    st.set_open(open);
    st.store(ui.ctx());
}

/// Open or close the category sections, the Dismissed section and the finding cards.
fn fold(
    ui: &Ui,
    findings: &[(&'static str, &Finding)],
    dismissed: &[(&'static str, Finding)],
    mode: Fold,
) {
    let has_findings = |c: Category| findings.iter().any(|(_, f)| f.category == c);
    let open_cat = |c: Category| match mode {
        Fold::All => true,
        Fold::Findings => has_findings(c),
        Fold::None => false,
    };
    let open_cards = mode != Fold::None;
    let open_dismissed = mode == Fold::All;
    for cat in Category::ALL {
        set_open(ui, cat_id(cat, has_findings(cat)), open_cat(cat));
    }
    set_open(ui, egui::Id::new(DISMISSED_ID), open_dismissed);
    for (_, f) in findings {
        set_open(ui, card_id(f), open_cards);
    }
    for (_, f) in dismissed {
        set_open(ui, card_id(f), open_dismissed);
    }
}
