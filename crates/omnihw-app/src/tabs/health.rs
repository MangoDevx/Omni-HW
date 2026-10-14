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
        power: snap.power.as_ref(),
    };
    diagnostics::run(&ctx)
}

fn is_gaming(r: &CheckResult) -> bool {
    r.category == Category::Gaming
}

/// Counts of general (non-gaming), non-dismissed findings by level: (critical, warning, info).
pub fn counts(results: &[CheckResult]) -> (usize, usize, usize) {
    let (visible, _) = crate::dismissed::split(results);
    let general: Vec<CheckResult> = visible.into_iter().filter(|r| !is_gaming(r)).collect();
    let f = diagnostics::findings(&general);
    let n = |l| f.iter().filter(|x| x.level == l).count();
    (n(Level::Critical), n(Level::Warning), n(Level::Info))
}

/// Number of non-dismissed findings in the Gaming category.
pub fn gaming_count(results: &[CheckResult]) -> usize {
    let (visible, _) = crate::dismissed::split(results);
    visible
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

/// Findings of these results with the name of the check that produced them, most severe first.
fn named_findings(results: &[CheckResult]) -> Vec<(&'static str, &Finding)> {
    let mut out: Vec<(&'static str, &Finding)> = results
        .iter()
        .filter_map(|r| match &r.outcome {
            Outcome::Findings(f) => Some(f.iter().map(move |x| (r.name, x))),
            _ => None,
        })
        .flatten()
        .collect();
    out.sort_by_key(|(_, f)| std::cmp::Reverse(f.level));
    out
}

pub fn show(ui: &mut Ui, info: &SystemInfo, snap: &Snapshot) {
    let uefi = info.checkup.firmware.uefi == Some(true);
    restart_dialog(ui);
    let all = run(info, snap);
    let (results, dismissed) = crate::dismissed::split(&all);
    let findings = named_findings(&results);
    let general: Vec<&CheckResult> = results.iter().filter(|r| !is_gaming(r)).collect();
    let passed = general
        .iter()
        .filter(|r| matches!(r.outcome, Outcome::Pass(_)))
        .count();
    let skipped = general
        .iter()
        .filter(|r| matches!(r.outcome, Outcome::Skipped(_)))
        .count();
    let found = general
        .iter()
        .filter(|r| matches!(&r.outcome, Outcome::Findings(f) if !f.is_empty()))
        .count();

    ui.label(RichText::new("Health Check").size(20.0).strong());
    ui.horizontal_wrapped(|ui| {
        match summary(&all) {
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
        if let Some(g) = gaming_summary(&all) {
            ui.label(
                RichText::new(format!("·  {g} below"))
                    .size(15.0)
                    .color(crate::heat::ink(ui.visuals(), BLUE)),
            );
        }
    });
    let mut stats = format!(
        "{} general checks: {found} found something, {passed} passed, {skipped} not applicable",
        general.len()
    );
    if !dismissed.is_empty() {
        stats.push_str(&format!(", {} dismissed", dismissed.len()));
    }
    ui.weak(format!(
        "{stats}. Live checks use the last ~2 minutes of sensor data and update as you use the PC."
    ));
    ui.add_space(10.0);

    let jump_to_dismissed = toolbar(ui, &findings, &dismissed);
    ui.add_space(4.0);

    for cat in Category::ALL {
        let rows: Vec<&CheckResult> = results.iter().filter(|r| r.category == cat).collect();
        if !rows.is_empty() {
            category_section(ui, cat, &rows, &findings, uefi);
        }
    }
    dismissed_section(ui, &dismissed, uefi, jump_to_dismissed);
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
/// Dismissed cards show their category (they're listed outside it) and a Restore button.
fn finding_card(ui: &mut Ui, check: &str, f: &Finding, uefi: bool, dismissed: bool) {
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
        super::power::fix_button(ui, a);
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
