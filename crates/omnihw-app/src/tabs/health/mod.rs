//! Health Check tab: runs the diagnostics rules and explains what they found, as a list of
//! findings (left) and the selected one's details (right).

mod detail;
mod list;

use super::{page_title, plural, temp};
use crate::heat::{BLUE, GREEN, ORANGE, RED};
use crate::sampler::Snapshot;
use detail::restart_dialog;
use egui::{Color32, Ui, vec2};
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
        network: snap.network.as_ref(),
        events: snap.event_log.as_ref().map(|(log, _)| log.as_ref()),
        power: snap.power.as_ref(),
    };
    diagnostics::run(&ctx)
}

/// Health Check results for the Health tab and the Overview banner. The checks only see data
/// that changes with a new sample or a (un)dismissal, so they run then rather than on every
/// frame (hovering alone repaints at the display rate).
#[derive(Default)]
pub struct HealthCache {
    /// (sample count, dismissed revision, show gaming) the report was built for.
    key: Option<(u64, u64, bool)>,
    report: Report,
}

impl HealthCache {
    /// With `show_gaming` off, the Gaming category is left out entirely (findings, passed
    /// checks and dismissed gaming findings).
    pub fn get(&mut self, info: &SystemInfo, snap: &Snapshot, show_gaming: bool) -> &Report {
        let key = (snap.samples, crate::dismissed::revision(), show_gaming);
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

/// What the details pane shows. Findings are kept by their dismiss key, so the selection
/// survives new samples and numbers changing in a title.
#[derive(Clone, PartialEq, Eq, Debug)]
enum Sel {
    /// A current finding.
    Finding(String),
    /// A category's passed and not-applicable checks.
    Passed(Category),
    /// The list of dismissed findings.
    Dismissed,
    /// One dismissed finding.
    DismissedFinding(String),
}

/// A finding with the check that produced it and its dismiss key.
struct Item<'a> {
    check: &'static str,
    f: &'a Finding,
    key: String,
}

impl<'a> Item<'a> {
    fn new(check: &'static str, f: &'a Finding) -> Item<'a> {
        Item {
            check,
            f,
            key: crate::dismissed::key(check, f),
        }
    }
}

/// The passed and not-applicable checks of one category.
struct Passed<'a> {
    cat: Category,
    rows: Vec<&'a CheckResult>,
    passed: usize,
    skipped: usize,
}

/// What the list and details panes show this frame, narrowed to the category filter.
/// Findings in each group are most severe first.
struct View<'a> {
    /// Critical issues and warnings outside Gaming.
    fix: Vec<Item<'a>>,
    /// Suggestions outside Gaming.
    suggestions: Vec<Item<'a>>,
    /// Gaming findings.
    gaming: Vec<Item<'a>>,
    passed: Vec<Passed<'a>>,
    /// Dismissed findings (not filtered).
    dismissed: Vec<Item<'a>>,
    filter: Option<Category>,
}

impl<'a> View<'a> {
    fn new(report: &'a Report, filter: Option<Category>) -> View<'a> {
        let wanted = |c: Category| filter.is_none_or(|f| f == c);
        let (mut fix, mut suggestions, mut gaming) = (Vec::new(), Vec::new(), Vec::new());
        for r in &report.results {
            for f in findings_of(r).iter().filter(|f| wanted(f.category)) {
                let group = if f.category == Category::Gaming {
                    &mut gaming
                } else if f.level == Level::Info {
                    &mut suggestions
                } else {
                    &mut fix
                };
                group.push(Item::new(r.name, f));
            }
        }
        for g in [&mut fix, &mut suggestions, &mut gaming] {
            g.sort_by_key(|i| std::cmp::Reverse(i.f.level));
        }
        let passed = Category::ALL
            .into_iter()
            .filter(|&c| wanted(c))
            .filter_map(|cat| {
                let rows: Vec<&CheckResult> = report
                    .results
                    .iter()
                    .filter(|r| r.category == cat && !matches!(r.outcome, Outcome::Findings(_)))
                    .collect();
                let passed = rows
                    .iter()
                    .filter(|r| matches!(r.outcome, Outcome::Pass(_)))
                    .count();
                (!rows.is_empty()).then(|| Passed {
                    cat,
                    skipped: rows.len() - passed,
                    passed,
                    rows,
                })
            })
            .collect();
        View {
            fix,
            suggestions,
            gaming,
            passed,
            dismissed: report
                .dismissed
                .iter()
                .map(|(check, f)| Item::new(check, f))
                .collect(),
            filter,
        }
    }

    /// Current findings in list order.
    fn findings(&self) -> impl Iterator<Item = &Item<'a>> {
        self.fix.iter().chain(&self.suggestions).chain(&self.gaming)
    }

    fn finding(&self, key: &str) -> Option<&Item<'a>> {
        self.findings().find(|i| i.key == key)
    }

    fn dismissed_finding(&self, key: &str) -> Option<&Item<'a>> {
        self.dismissed.iter().find(|i| i.key == key)
    }

    /// `sel` if it is still listed, otherwise the most severe finding, then the first
    /// passed-checks entry, then the dismissed list.
    fn resolve(&self, sel: Option<Sel>) -> Option<Sel> {
        let valid = |s: &Sel| match s {
            Sel::Finding(k) => self.finding(k).is_some(),
            Sel::Passed(c) => self.passed.iter().any(|p| p.cat == *c),
            Sel::Dismissed => !self.dismissed.is_empty(),
            Sel::DismissedFinding(k) => self.dismissed_finding(k).is_some(),
        };
        if let Some(s) = sel.filter(valid) {
            return Some(s);
        }
        // First of the most severe (max_by_key would pick the last).
        let worst = self.findings().fold(None::<&Item>, |best, i| match best {
            Some(b) if b.f.level >= i.f.level => Some(b),
            _ => Some(i),
        });
        if let Some(i) = worst {
            return Some(Sel::Finding(i.key.clone()));
        }
        if let Some(p) = self.passed.first() {
            return Some(Sel::Passed(p.cat));
        }
        (!self.dismissed.is_empty()).then_some(Sel::Dismissed)
    }

    /// Every list entry in list order: findings, passed-checks entries, then the dismissed list.
    fn entries(&self) -> Vec<Sel> {
        let mut entries: Vec<Sel> = self
            .findings()
            .map(|i| Sel::Finding(i.key.clone()))
            .chain(self.passed.iter().map(|p| Sel::Passed(p.cat)))
            .collect();
        if !self.dismissed.is_empty() {
            entries.push(Sel::Dismissed);
        }
        entries
    }

    /// The entry `delta` rows from `sel`, stopping at the ends. A dismissed finding counts as
    /// the dismissed list it is opened from.
    fn step(&self, sel: Option<&Sel>, delta: isize) -> Option<Sel> {
        let entries = self.entries();
        let sel = match sel {
            Some(Sel::DismissedFinding(_)) => Some(&Sel::Dismissed),
            s => s,
        };
        let i = match sel.and_then(|s| entries.iter().position(|e| e == s)) {
            Some(i) => i
                .saturating_add_signed(delta)
                .min(entries.len().saturating_sub(1)),
            None => 0,
        };
        entries.get(i).cloned()
    }
}

/// Up/Down move the selection through the list, unless a widget has keyboard focus, a menu is
/// open or a dialog is up. Returns the new selection when it moved.
fn arrow_step(ctx: &egui::Context, view: &View, sel: Option<&Sel>) -> Option<Sel> {
    if ctx.egui_wants_keyboard_input()
        || egui::Popup::is_any_open(ctx)
        || ctx.memory(|m| m.focused().is_some() || m.top_modal_layer().is_some())
    {
        return None;
    }
    let delta = ctx.input_mut(|i| {
        if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
            -1
        } else if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
            1
        } else {
            0
        }
    });
    if delta == 0 {
        return None;
    }
    view.step(sel, delta).filter(|s| Some(s) != sel)
}

fn sel_id() -> egui::Id {
    egui::Id::new("health_selection")
}

fn filter_id() -> egui::Id {
    egui::Id::new("health_filter")
}

/// Show `sel` in the details pane (on a click in either pane).
fn select(ctx: &egui::Context, sel: Sel) {
    temp::set(ctx, sel_id(), sel);
}

/// The stored selection, or the default when it is no longer listed.
fn selection(ctx: &egui::Context, view: &View) -> Option<Sel> {
    view.resolve(temp::get::<Sel>(ctx, sel_id()))
}

/// Below this width the list and the details stack instead of sitting side by side.
const NARROW: f32 = 760.0;

pub fn show(ui: &mut Ui, info: &SystemInfo, report: &Report) {
    let uefi = info.checkup.firmware.uefi == Some(true);
    restart_dialog(ui);
    let ctx = ui.ctx().clone();
    // A filter for a category that is no longer present (e.g. Gaming turned off) is dropped.
    let filter = temp::get::<Option<Category>>(&ctx, filter_id())
        .flatten()
        .filter(|&c| report.results.iter().any(|r| r.category == c));
    let view = View::new(report, filter);
    let mut before = selection(&ctx, &view);
    let moved = arrow_step(&ctx, &view, before.as_ref());
    if let Some(s) = &moved {
        select(&ctx, s.clone());
        before = moved.clone();
    }
    let moved = moved.is_some();

    if ui.available_width() < NARROW {
        egui::ScrollArea::vertical()
            .id_salt("health_page")
            .auto_shrink(false)
            .show(ui, |ui| {
                header(ui, report, filter);
                list::list(ui, &view, before.as_ref(), moved);
                ui.add_space(8.0);
                ui.separator();
                ui.add_space(8.0);
                detail::detail(ui, &view, selection(&ctx, &view).as_ref(), uefi);
            });
        return;
    }

    header(ui, report, filter);
    let h = ui.available_height();
    let list_w = (ui.available_width() * 0.38).max(280.0);
    ui.horizontal_top(|ui| {
        let down = egui::Layout::top_down(egui::Align::Min);
        ui.allocate_ui_with_layout(vec2(list_w, h), down, |ui| {
            ui.set_width(list_w);
            egui::Frame::new()
                .fill(ui.visuals().faint_bg_color)
                .corner_radius(6)
                .inner_margin(6)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("health_list")
                        .auto_shrink(false)
                        .show(ui, |ui| list::list(ui, &view, before.as_ref(), moved));
                });
        });
        ui.add_space(8.0);
        ui.allocate_ui_with_layout(vec2(ui.available_width(), h), down, |ui| {
            let sel = selection(&ctx, &view);
            let mut area = egui::ScrollArea::vertical()
                .id_salt("health_detail")
                .auto_shrink(false);
            if sel != before || moved {
                // A new selection starts at the top of its details.
                area = area.vertical_scroll_offset(0.0);
            }
            area.show(ui, |ui| {
                ui.add_space(2.0);
                detail::detail(ui, &view, sel.as_ref(), uefi);
            });
        });
    });
}

/// Page title, the category tiles and the totals line.
fn header(ui: &mut Ui, report: &Report, filter: Option<Category>) {
    page_title(ui, "Health Check");
    ui.add_space(4.0);
    if let Some(f) = list::tiles(ui, report, filter) {
        temp::set(ui.ctx(), filter_id(), f);
    }
    ui.add_space(2.0);
    let results = &report.results;
    let count = |p: fn(&Outcome) -> bool| results.iter().filter(|r| p(&r.outcome)).count();
    let found = count(|o| matches!(o, Outcome::Findings(f) if !f.is_empty()));
    let passed = count(|o| matches!(o, Outcome::Pass(_)));
    let skipped = count(|o| matches!(o, Outcome::Skipped(_)));
    let mut stats = format!(
        "{} · {found} found something · {passed} passed · {skipped} not applicable",
        plural(results.len(), "check")
    );
    if !report.dismissed.is_empty() {
        stats.push_str(&format!(" · {} dismissed", report.dismissed.len()));
    }
    ui.weak(stats).on_hover_text(
        "Live checks (memory pressure, temperatures, throttling) use the last ~2 minutes of \
         sensor data and update as you use the PC.",
    );
    ui.add_space(8.0);
}

/// A category's tile status: "OK", "not checked", or its finding count in the colour of the
/// worst one. None when no check of the category ran.
fn category_status(results: &[CheckResult], cat: Category) -> Option<(String, Color32)> {
    let rows: Vec<&CheckResult> = results.iter().filter(|r| r.category == cat).collect();
    if rows.is_empty() {
        return None;
    }
    let levels: Vec<Level> = results
        .iter()
        .flat_map(findings_of)
        .filter(|f| f.category == cat)
        .map(|f| f.level)
        .collect();
    Some(match levels.iter().max() {
        Some(&worst) => (count_label(&levels), level_color(worst)),
        None if rows
            .iter()
            .all(|r| matches!(r.outcome, Outcome::Skipped(_))) =>
        {
            ("not checked".to_string(), Color32::GRAY)
        }
        None => ("OK".to_string(), GREEN),
    })
}

/// "1 warning", "4 suggestions", "2 critical"; "3 issues" when the levels are mixed.
fn count_label(levels: &[Level]) -> String {
    let n = levels.len();
    let all = |l: Level| levels.iter().all(|&x| x == l);
    if all(Level::Info) {
        plural(n, "suggestion")
    } else if all(Level::Warning) {
        plural(n, "warning")
    } else if all(Level::Critical) {
        format!("{n} critical")
    } else {
        plural(n, "issue")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_labels() {
        use Level::*;
        assert_eq!(count_label(&[Info, Info]), "2 suggestions");
        assert_eq!(count_label(&[Warning]), "1 warning");
        assert_eq!(count_label(&[Critical, Critical]), "2 critical");
        assert_eq!(count_label(&[Critical, Warning, Info]), "3 issues");
    }
}
