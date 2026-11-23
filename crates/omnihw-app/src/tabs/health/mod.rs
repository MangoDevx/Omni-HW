//! Health Check tab: runs the diagnostics rules and explains what they found, as a list of
//! findings (left) and the selected one's details (right).

mod detail;
mod list;

use crate::app::OmniApp;
use crate::heat::{BLUE, GREEN, ORANGE, RED, rgb8};
use crate::nav::Arrow;
use crate::sampler::Snapshot;
use crate::ui::{self, plural};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::separator::Separator;
use gpui_kit::component::{ActiveTheme, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, ClickEvent, Context as ViewContext, Div, ElementId, InteractiveElement,
    IntoElement, ParentElement, Rgba, ScrollHandle, StatefulInteractiveElement, Styled, Window,
    rems,
};
use omnihw_core::SystemInfo;
use omnihw_core::diagnostics::{self, Category, CheckResult, Context, Finding, Level, Outcome};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::time::{SystemTime, UNIX_EPOCH};

/// Colour of "not checked" markers and neutral pills.
const GRAY: Rgba = rgb8(160, 160, 160);

/// What the page remembers between redraws.
#[derive(Default)]
pub struct ViewState {
    /// The list entry the user picked; when it is no longer listed (or nothing was picked)
    /// the most severe finding is shown instead (see [`View::resolve`]).
    sel: Option<Sel>,
    /// Category filter from the tiles; None shows every category.
    filter: Option<Category>,
    /// Ticked "how to fix" steps by finding title. They only live for this session.
    done: HashMap<String, Vec<bool>>,
    /// Scroll position of the list pane (of the whole page when narrow), so an arrow key can
    /// bring the selected entry into view.
    list_scroll: ScrollHandle,
    /// Scroll the selected entry into view on the next draw (an arrow key moved it).
    reveal: bool,
    /// "Restart into UEFI" was confirmed and is waiting for its administrator prompt.
    restarting: bool,
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
/// run then rather than on every redraw.
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

/// What the details pane shows. Findings are kept by their dismiss key, so the selection
/// survives new samples and numbers changing in a title.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
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

/// What the list and details panes show, narrowed to the category filter. Findings in each
/// group are most severe first.
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
    dumps: &'a DumpStatus,
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
            dumps: &report.dumps,
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

/// The category filter, dropped when its category is no longer present (e.g. Gaming turned
/// off).
fn live_filter(filter: Option<Category>, report: &Report) -> Option<Category> {
    filter.filter(|&c| report.results.iter().any(|r| r.category == c))
}

/// An arrow key the navigation did not use while this page is open: Up/Down move the
/// selection through the findings list. (Keys only get here when no focused widget, menu
/// or dialog uses them.)
pub fn arrow_key(app: &mut OmniApp, arrow: Arrow, cx: &mut ViewContext<OmniApp>) {
    let delta = match arrow {
        Arrow::Up => -1,
        Arrow::Down => 1,
        Arrow::Left | Arrow::Right => return,
    };
    let Some(info) = app.sampler.info() else {
        return;
    };
    let snap = app.sampler.snapshot();
    let report = app.health.get(info, &snap, app.settings.show_gaming);
    let state = &mut app.views.health;
    let view = View::new(report, live_filter(state.filter, report));
    let sel = view.resolve(state.sel.clone());
    if let Some(next) = view
        .step(sel.as_ref(), delta)
        .filter(|s| Some(s) != sel.as_ref())
    {
        state.sel = Some(next);
        state.reveal = true;
        cx.notify();
    }
}

/// A click handler that shows `sel` in the details pane.
fn select(
    sel: Sel,
    cx: &mut ViewContext<OmniApp>,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static + use<> {
    let app = cx.weak_entity();
    move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
        let _ = app.update(cx, |this, cx| {
            this.views.health.sel = Some(sel.clone());
            cx.notify();
        });
    }
}

/// Below this width (in rems; 760 px at the default scale) the list and the details stack
/// instead of sitting side by side.
const NARROW: f32 = 54.0;
/// The list pane takes 38% of the width, but at least this many rems (280 px at the default
/// scale).
const LIST_MIN: f32 = 20.0;

pub fn show(app: &mut OmniApp, window: &mut Window, cx: &mut ViewContext<OmniApp>) -> AnyElement {
    let Some(info) = app.sampler.info() else {
        return ui::column().into_any_element();
    };
    let uefi = info.checkup.firmware.uefi == Some(true);
    let width = crate::app::page_width_rems(app.settings.nav_style, window);
    let snap = app.sampler.snapshot();
    let report = app.health.get(info, &snap, app.settings.show_gaming);
    let state = &mut app.views.health;
    let filter = live_filter(state.filter, report);
    let view = View::new(report, filter);
    let sel = view.resolve(state.sel.clone());
    let reveal = std::mem::take(&mut state.reveal);
    let handle = state.list_scroll.clone();
    let details = detail::Details {
        uefi,
        restarting: state.restarting,
        done: &state.done,
    };

    let header = header(report, filter, cx);
    let (mut rows, selected_row) = list::list(&view, sel.as_ref(), cx);
    let detail = detail::detail(&view, sel.as_ref(), &details, cx);

    if width < NARROW {
        // One scroll area: the header, the list, then the details.
        let mut children = vec![header.into_any_element()];
        let offset = children.len();
        children.append(&mut rows);
        children.extend([
            ui::space(8.0).into_any_element(),
            Separator::horizontal().into_any_element(),
            ui::space(8.0).into_any_element(),
            detail.into_any_element(),
        ]);
        if reveal && let Some(i) = selected_row {
            handle.scroll_to_item(offset + i);
        }
        return scroll_pane("health-page", &handle, children).into_any_element();
    }

    if reveal && let Some(i) = selected_row {
        handle.scroll_to_item(i);
    }
    let list_w = (width * 0.38).max(LIST_MIN);
    let theme = cx.theme();
    let list_pane = v_flex()
        .flex_none()
        .w(rems(list_w))
        .h_full()
        .p_1p5()
        .rounded(theme.radius)
        .bg(theme.muted)
        .child(scroll_pane("health-list", &handle, rows));
    // A new selection starts at the top of its details: each selection gets its own scroll
    // area.
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    sel.hash(&mut hasher);
    let detail_pane = v_flex()
        .flex_1()
        .min_w_0()
        .h_full()
        .min_h_0()
        .overflow_y_scrollbar()
        .id(ElementId::NamedInteger(
            "health-detail".into(),
            hasher.finish(),
        ))
        .child(ui::space(2.0))
        .child(detail);
    v_flex()
        .size_full()
        .min_h_0()
        .child(header)
        .child(
            h_flex()
                .flex_1()
                .min_h_0()
                .items_stretch()
                .gap_2()
                .child(list_pane)
                .child(detail_pane),
        )
        .into_any_element()
}

/// A vertical scroll area over `handle`, with a scrollbar. Its direct children are the items
/// `handle.scroll_to_item` counts.
fn scroll_pane(id: &'static str, handle: &ScrollHandle, children: Vec<AnyElement>) -> Div {
    v_flex()
        .relative()
        .flex_1()
        .min_h_0()
        .min_w_0()
        .child(
            v_flex()
                .id(id)
                .size_full()
                .overflow_y_scroll()
                .track_scroll(handle)
                .children(children),
        )
        .vertical_scrollbar(handle)
}

/// Page title, the category tiles and the totals line.
fn header(report: &Report, filter: Option<Category>, cx: &mut ViewContext<OmniApp>) -> Div {
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
    v_flex()
        .flex_none()
        .child(ui::page_title("Health Check"))
        .child(ui::space(4.0))
        .child(list::tiles(report, filter, cx))
        .child(ui::space(2.0))
        .child(h_flex().child(ui::with_tip(
            "health-stats",
            ui::weak(stats, cx),
            "Live checks (memory pressure, temperatures, throttling) use the last ~2 minutes of \
             sensor data and update as you use the PC.",
        )))
        .child(ui::space(8.0))
}

/// A category's tile status: "OK", "not checked", or its finding count in the colour of the
/// worst one. None when no check of the category ran.
fn category_status(results: &[CheckResult], cat: Category) -> Option<(String, Rgba)> {
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
            ("not checked".to_string(), GRAY)
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
    use crate::app::Tab;
    use crate::settings::Settings;
    use crate::tabs::page_tests::{assert_shows, go, machines, open, page, snapshot, texts};
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{AnyWindowHandle, AppContext, Entity, TestAppContext};
    use omnihw_core::testkit::{desktop, laptop};

    #[test]
    fn count_labels() {
        use Level::*;
        assert_eq!(count_label(&[Info, Info]), "2 suggestions");
        assert_eq!(count_label(&[Warning]), "1 warning");
        assert_eq!(count_label(&[Critical, Critical]), "2 critical");
        assert_eq!(count_label(&[Critical, Warning, Info]), "3 issues");
    }

    /// The app's report as list entries, the default selection and the finding titles and
    /// details by key, read the way the page reads them.
    struct Listed {
        entries: Vec<Sel>,
        default: Option<Sel>,
        findings: HashMap<String, (String, String)>,
        dismissed: Vec<String>,
    }

    fn listed(cx: &mut TestAppContext, app: &Entity<OmniApp>) -> Listed {
        cx.update(|cx| {
            app.update(cx, |app, _| {
                let info = app.sampler.info().expect("fake info");
                let snap = app.sampler.snapshot();
                let report = app.health.get(info, &snap, app.settings.show_gaming);
                let view = View::new(report, None);
                Listed {
                    entries: view.entries(),
                    default: view.resolve(None),
                    findings: view
                        .findings()
                        .map(|i| (i.key.clone(), (i.f.title.clone(), i.f.detail.clone())))
                        .collect(),
                    dismissed: view.dismissed.iter().map(|i| i.key.clone()).collect(),
                }
            })
        })
    }

    fn selection(cx: &mut TestAppContext, app: &Entity<OmniApp>) -> Option<Sel> {
        cx.read(|cx| app.read(cx).views.health.sel.clone())
    }

    /// A snapshot of the fake desktop 40 days after the last restart, so the uptime check
    /// finds something (a suggestion with steps).
    fn long_uptime(info: &SystemInfo) -> Snapshot {
        Snapshot {
            uptime_secs: Some(40 * 86_400),
            ..snapshot(info)
        }
    }

    fn open_health(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<OmniApp>) {
        let info = desktop();
        let (window, app) = open(cx, info.clone(), long_uptime(&info), Settings::default());
        go(cx, &app, Tab::Health);
        texts(cx, window);
        (window, app)
    }

    fn click(cx: &mut TestAppContext, window: AnyWindowHandle, id: impl Into<ElementId>) {
        let id = id.into();
        cx.update_window(window, |_, window, cx| window.click(id, cx))
            .unwrap();
    }

    #[gpui_kit::test]
    fn health_page_draws(cx: &mut TestAppContext) {
        for info in machines() {
            let shown = page(cx, &info, snapshot(&info), Tab::Health);
            assert_shows(
                &shown,
                &["Health Check", "All", "checks ·", "PASSED CHECKS"],
            );
        }
        let info = desktop();
        let shown = page(cx, &info, long_uptime(&info), Tab::Health);
        assert_shows(
            &shown,
            &[
                "SUGGESTIONS · 1",
                "Windows has been running for 40 days",
                "HOW TO FIX",
                "· 0 of 2 done",
                "1. Restart to clear them out",
                "Dismiss",
                "Not relevant to you? Hide it.",
            ],
        );
        // Before the first sample.
        let shown = page(cx, &info, Snapshot::default(), Tab::Health);
        assert_shows(&shown, &["Health Check", "All"]);
    }

    #[gpui_kit::test]
    fn narrow_window_draws(cx: &mut TestAppContext) {
        // At twice the UI scale the 1200 px test window is narrower than NARROW rems.
        let info = desktop();
        let settings = Settings {
            ui_scale: 2.0,
            ..Default::default()
        };
        let (window, app) = open(cx, info.clone(), long_uptime(&info), settings);
        go(cx, &app, Tab::Health);
        let shown = texts(cx, window);
        let narrow = cx
            .update_window(window, |_, window, cx| {
                crate::app::page_width_rems(app.read(cx).settings.nav_style, window) < NARROW
            })
            .unwrap();
        assert!(
            narrow,
            "the test window should get the single-column layout"
        );
        assert_shows(
            &shown,
            &["Health Check", "PASSED CHECKS", "HOW TO FIX", "Dismiss"],
        );
    }

    #[gpui_kit::test]
    fn clicking_a_row_shows_its_details(cx: &mut TestAppContext) {
        let (window, app) = open_health(cx);
        let l = listed(cx, &app);
        let passed = l
            .entries
            .iter()
            .find(|e| matches!(e, Sel::Passed(_)))
            .expect("a passed-checks entry")
            .clone();
        let Some(Sel::Finding(key)) = l.default.clone() else {
            panic!("the uptime finding is shown by default");
        };
        // A category's passed checks.
        click(cx, window, list::row_id(&passed));
        assert_eq!(selection(cx, &app), Some(passed.clone()));
        let Sel::Passed(c) = &passed else {
            unreachable!()
        };
        assert_shows(
            &texts(cx, window),
            &[&format!("{} · passed checks", c.name())],
        );
        // Back to the finding.
        let finding = Sel::Finding(key.clone());
        click(cx, window, list::row_id(&finding));
        assert_eq!(selection(cx, &app), Some(finding));
        let (title, detail) = &l.findings[&key];
        assert_shows(
            &texts(cx, window),
            &[title, detail, "HOW TO FIX", "Dismiss"],
        );
    }

    #[gpui_kit::test]
    fn down_arrow_moves_the_selection(cx: &mut TestAppContext) {
        let (window, app) = open_health(cx);
        let l = listed(cx, &app);
        let i = l
            .entries
            .iter()
            .position(|e| Some(e) == l.default.as_ref())
            .expect("the default selection is listed");
        let next = l.entries.get(i + 1).expect("an entry below the default");
        cx.update_window(window, |_, window, cx| window.press("down", cx))
            .unwrap();
        assert_eq!(selection(cx, &app).as_ref(), Some(next));
        assert_eq!(cx.read(|cx| app.read(cx).tab), Tab::Health);
        texts(cx, window);
        cx.update_window(window, |_, window, cx| window.press("up", cx))
            .unwrap();
        assert_eq!(selection(cx, &app), l.default);
    }

    #[gpui_kit::test]
    fn dismissing_moves_a_finding_to_the_dismissed_list(cx: &mut TestAppContext) {
        // `crate::dismissed` is shared by every test in the process, so this dismisses a
        // laptop finding, which the other tests here don't look at (they use the uptime one).
        let _alone = crate::dismissed::test_lock();
        let info = laptop();
        let (window, app) = open(cx, info.clone(), snapshot(&info), Settings::default());
        go(cx, &app, Tab::Health);
        texts(cx, window);
        let l = listed(cx, &app);
        let Some(Sel::Finding(key)) = l.default.clone() else {
            panic!("the fake laptop has findings");
        };
        let (title, _) = l.findings[&key].clone();
        click(cx, window, "health-dismiss");
        let shown = texts(cx, window);
        let after = listed(cx, &app);
        // Restore before asserting, so a failure leaves nothing dismissed behind.
        crate::dismissed::restore(&key);
        assert!(after.dismissed.contains(&key), "{title:?} wasn't dismissed");
        assert!(!after.findings.contains_key(&key));
        assert_shows(&shown, &["Dismissed (1)", "1 dismissed"]);

        // Restoring it from the dismissed list brings it back.
        crate::dismissed::dismiss(key.clone());
        texts(cx, window);
        click(cx, window, list::row_id(&Sel::Dismissed));
        assert_eq!(selection(cx, &app), Some(Sel::Dismissed));
        assert_shows(&texts(cx, window), &["Restore", &title]);
        click(cx, window, ("restore", 0usize));
        texts(cx, window);
        let after = listed(cx, &app);
        crate::dismissed::restore(&key);
        assert!(!after.dismissed.contains(&key));
        assert!(after.findings.contains_key(&key));
    }
}
