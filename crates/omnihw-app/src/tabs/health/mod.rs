//! Health Check tab: runs the diagnostics rules and explains what they found, as a list of
//! findings (left) and the selected one's details (right).
//!
//! PENDING PORT: the model below (running the checks, the cached report and its summaries) is
//! ported; the page itself is not. The egui version is
//! `git show main:crates/omnihw-app/src/tabs/health/` (mod.rs, list.rs, detail.rs).

use crate::app::OmniApp;
use crate::heat::{BLUE, ORANGE, RED};
use crate::sampler::Snapshot;
use crate::ui::plural;
use gpui_kit::{AnyElement, Context as ViewContext, Rgba, Window};
use omnihw_core::SystemInfo;
use omnihw_core::diagnostics::{self, Category, CheckResult, Context, Finding, Level, Outcome};
use std::time::{SystemTime, UNIX_EPOCH};

/// Selections, filters and edit buffers of this page.
#[derive(Default)]
pub struct ViewState {}

/// An arrow key the navigation did not use while this page is open: Up/Down move the
/// selection through the findings list.
pub fn arrow_key(_app: &mut OmniApp, _arrow: crate::nav::Arrow, _cx: &mut ViewContext<OmniApp>) {}

pub fn show(app: &mut OmniApp, _window: &mut Window, cx: &mut ViewContext<OmniApp>) -> AnyElement {
    super::pending(app.tab, cx)
}

pub fn level_color(l: Level) -> Rgba {
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
        uptime_secs: snap.uptime_secs,
        power: snap.power.as_ref(),
        network: snap.network.as_ref(),
        whea_live: &snap.whea_live,
        dumps: match &snap.dumps {
            Some(Ok(d)) => Some(d.as_slice()),
            _ => None,
        },
    };
    diagnostics::run(&ctx)
}

/// Crash dump analysis state, for the blue screen finding's details.
#[derive(Default, Clone)]
pub struct DumpStatus {
    pub busy: bool,
    /// Number of dumps analysed, or why they couldn't be read; None until asked.
    pub result: Option<Result<usize, String>>,
}

/// Health Check results for the Health tab and the Overview banner. The checks only see data
/// that changes with a new sample, new power info, new WHEA errors or a (un)dismissal, so they
/// run then rather than on every frame (hovering alone repaints at the display rate).
#[derive(Default)]
pub struct HealthCache {
    /// (sample count, power revision, live WHEA revision, dismissed revision, show gaming, dumps
    /// revision) the report was built for.
    key: Option<(u64, u64, u64, u64, bool, u64)>,
    report: Report,
}

impl HealthCache {
    /// With `show_gaming` off, the Gaming category is left out entirely (findings, passed
    /// checks and dismissed gaming findings).
    pub fn get(&mut self, info: &SystemInfo, snap: &Snapshot, show_gaming: bool) -> &Report {
        let key = (
            snap.samples,
            snap.power_rev,
            snap.whea_rev,
            crate::dismissed::revision(),
            show_gaming,
            snap.dumps_rev,
        );
        if self.key != Some(key) {
            let mut all = run(info, snap);
            if !show_gaming {
                all.retain(|r| !is_gaming(r));
            }
            self.report = Report::new(all);
            self.report.dumps = DumpStatus {
                busy: snap.dumps_busy,
                result: snap
                    .dumps
                    .as_ref()
                    .map(|r| r.as_ref().map(Vec::len).map_err(Clone::clone)),
            };
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
    pub summary: Option<(String, Rgba)>,
    /// "2 gaming suggestions", if there are any.
    pub gaming_summary: Option<String>,
    /// Crash dump analysis state.
    pub dumps: DumpStatus,
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
            dumps: DumpStatus::default(),
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
fn summary(results: &[CheckResult]) -> Option<(String, Rgba)> {
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
