//! The details pane (a finding with its fix steps checklist, Fix and firmware buttons; a
//! category's passed checks; the dismissed list) and the "Restart into UEFI" confirmation.

use super::{DumpStatus, GRAY, GREEN, Item, Passed, Sel, View, level_color, select};
use crate::app::OmniApp;
use crate::heat::{ORANGE, RED};
use crate::ui;
use crate::ui::LineSized;
use gpui_kit::component::button::Button;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::separator::Separator;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme, Disableable, Icon, IconName, Sizable, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AnyElement, App, Context, Div, ElementId, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, Window, rems,
};
use omnihw_core::diagnostics::{Category, CheckResult, Finding, Outcome, is_blue_screen_finding};
use std::collections::HashMap;

/// Text in the details pane stops at this width (in rems; 680 px at the default scale), so
/// lines stay easy to read.
const MAX_WIDTH: f32 = 48.5;

/// What the details pane needs besides the list entries.
pub(super) struct Details<'a> {
    /// The firmware is UEFI, so "Restart into UEFI / BIOS" can be offered.
    pub uefi: bool,
    /// A firmware restart is waiting for its administrator prompt.
    pub restarting: bool,
    /// Ticked fix steps by finding title.
    pub done: &'a HashMap<String, Vec<bool>>,
}

/// The details of the selected list entry.
pub(super) fn detail(
    view: &View,
    sel: Option<&Sel>,
    d: &Details,
    cx: &mut Context<OmniApp>,
) -> Div {
    let pane = v_flex().w_full().max_w(rems(MAX_WIDTH));
    match sel {
        Some(Sel::Finding(k)) => match view.finding(k) {
            Some(i) => pane.child(finding_detail(i, d, false, view.dumps, cx)),
            None => pane,
        },
        Some(Sel::DismissedFinding(k)) => {
            let back = Button::new("health-back")
                .line_sized()
                .caption("‹ All dismissed")
                .tooltip("Back to the dismissed list")
                .on_click(select(Sel::Dismissed, cx));
            let pane = pane.child(h_flex().child(back)).child(ui::space(6.0));
            match view.dismissed_finding(k) {
                Some(i) => pane.child(finding_detail(i, d, true, view.dumps, cx)),
                None => pane,
            }
        }
        Some(Sel::Passed(c)) => match view.passed.iter().find(|p| p.cat == *c) {
            Some(p) => pane.child(passed_detail(p, cx)),
            None => pane,
        },
        Some(Sel::Dismissed) => pane.child(dismissed_list(&view.dismissed, cx)),
        None => pane.child(ui::weak("No checks have run yet.", cx)),
    }
}

/// A large bold heading at the top of the details.
#[track_caller]
fn title(text: impl Into<gpui_kit::SharedString>) -> gpui_kit::component::label::Label {
    ui::label(text).text_size(gpui_kit::rems(18.0 / 16.0))
}

/// A finding: title, level and category, what was measured, how to fix it, then Dismiss
/// (or Restore for a dismissed one).
fn finding_detail(
    item: &Item,
    d: &Details,
    dismissed: bool,
    dumps: &DumpStatus,
    cx: &mut Context<OmniApp>,
) -> Div {
    let f = item.f;
    let mut pills = ui::wrap_row()
        .gap_1p5()
        .child(ui::pill(f.level.name(), level_color(f.level), true, cx))
        .child(ui::pill(f.category.name(), GRAY, true, cx));
    if f.security {
        pills = pills.child(ui::pill("Security setting", ORANGE, true, cx));
    }
    if dismissed {
        pills = pills.child(ui::pill("Dismissed", GRAY, true, cx));
    }
    let mut col = v_flex()
        .child(title(f.title.clone()))
        .child(ui::space(4.0))
        .child(pills)
        .child(ui::space(10.0))
        .children(finding_body(f, d, cx));
    if is_blue_screen_finding(item.check, f) {
        col = col.child(dump_button(dumps, cx));
    }
    col = col
        .child(ui::space(16.0))
        .child(Separator::horizontal())
        .child(ui::space(4.0));
    let key = item.key.clone();
    if dismissed {
        col.child(
            h_flex().child(
                Button::new("health-restore")
                    .line_sized()
                    .caption("Restore")
                    .tooltip("Show this in the list and the summary again")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        crate::dismissed::restore(&key);
                        this.views.health.sel = Some(Sel::Finding(key.clone()));
                        cx.notify();
                    })),
            ),
        )
    } else {
        col.child(
            ui::wrap_row()
                .child(
                    Button::new("health-dismiss")
                        .line_sized()
                        .caption("Dismiss")
                        .tooltip(
                            "Hide this from the Health Check summary and the Overview. \
                             See or restore it under Dismissed at the bottom of the list.",
                        )
                        .on_click(cx.listener(move |_, _, _, cx| {
                            crate::dismissed::dismiss(key.clone());
                            cx.notify();
                        })),
                )
                .child(ui::weak("Not relevant to you? Hide it.", cx)),
        )
    }
}

/// "Analyze crash dumps" with its state: a spinner while reading, then the error or a note
/// when there were none (the results themselves show in the finding's details and steps).
fn dump_button(dumps: &DumpStatus, cx: &mut Context<OmniApp>) -> Div {
    let elevated = crate::elevate::elevated();
    let label = if elevated {
        "Analyze crash dumps"
    } else {
        "Analyze crash dumps (asks for administrator)"
    };
    let hover = if elevated {
        "Reads C:\\Windows\\Minidump."
    } else {
        "Reads C:\\Windows\\Minidump. Windows asks for administrator permission because only administrators can read crash dumps."
    };
    let mut row = ui::wrap_row().child(
        Button::new("health-dumps")
            .line_sized()
            .caption(label)
            .tooltip(hover)
            .disabled(dumps.busy)
            .on_click(cx.listener(|this, _, _, cx| {
                this.sampler.read_dumps();
                cx.notify();
            })),
    );
    if dumps.busy {
        row = row.child(Spinner::new().small());
    }
    let status = match &dumps.result {
        _ if dumps.busy => None,
        Some(Err(e)) => Some(ui::colored(
            format!("Couldn't read crash dumps: {e}"),
            RED,
            cx,
        )),
        Some(Ok(0)) => Some(ui::weak("No crash dumps found", cx)),
        Some(Ok(n)) => Some(ui::weak(
            format!(
                "{n} crash {} analysed (listed above)",
                if *n == 1 { "dump" } else { "dumps" }
            ),
            cx,
        )),
        None => None,
    };
    v_flex().child(ui::space(10.0)).child(row.children(status))
}

/// A category's passed (green OK + detail) and not-applicable (reason) checks.
fn passed_detail(p: &Passed, cx: &App) -> Div {
    let mut counts = format!("{} passed", p.passed);
    if p.skipped > 0 {
        counts.push_str(&format!(", {} not applicable to this PC", p.skipped));
    }
    v_flex()
        .child(title(format!("{} · passed checks", p.cat.name())))
        .child(ui::weak(counts, cx))
        .children(category_note(p.cat).map(|note| ui::weak(note, cx)))
        .child(ui::space(8.0))
        .child(result_grid(&p.rows, cx))
}

/// Dismissed findings: click one to read it again, Restore brings it back.
fn dismissed_list(items: &[Item], cx: &mut Context<OmniApp>) -> Div {
    let mut intro = ui::wrap_row().child(ui::weak(
        "Hidden from the summary and the Overview. Restore one to show it again.",
        cx,
    ));
    if items.len() > 1 {
        intro = intro.child(
            Button::new("health-restore-all")
                .line_sized()
                .caption("Restore all")
                .on_click(cx.listener(|_, _, _, cx| {
                    crate::dismissed::restore_all();
                    cx.notify();
                })),
        );
    }
    let mut list = v_flex().gap_0p5();
    for (n, i) in items.iter().enumerate() {
        let key = i.key.clone();
        let restore = Button::new(("restore", n))
            .line_sized()
            .caption("Restore")
            .tooltip("Show this in the list and the summary again")
            .on_click(cx.listener(move |_, _, _, cx| {
                crate::dismissed::restore(&key);
                cx.notify();
            }));
        let row = super::list::row(
            ElementId::Name(format!("health-dismissed/{}", i.key).into()),
            false,
            level_color(i.f.level),
            &i.f.title,
            i.f.category.name(),
            Some("Read it again"),
            cx,
        )
        .on_click(select(Sel::DismissedFinding(i.key.clone()), cx))
        .flex_1()
        .min_w_0();
        list = list.child(
            h_flex()
                .id(("health-dismissed-row", n))
                .gap_2()
                .child(restore)
                .child(row),
        );
    }
    v_flex()
        .child(title("Dismissed"))
        .child(intro)
        .child(ui::space(8.0))
        .child(list)
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

/// Passed (green OK + detail) and skipped (reason) checks as a compact striped table: the
/// status, the check's name, then the detail, which wraps.
fn result_grid(rows: &[&CheckResult], cx: &App) -> Div {
    let stripe = cx.theme().muted;
    let mut table = v_flex();
    let mut n = 0;
    for r in rows {
        let (status, detail) = match &r.outcome {
            Outcome::Pass(detail) => (ui::colored("OK", GREEN, cx), detail),
            Outcome::Skipped(why) => (ui::weak("–", cx), why),
            Outcome::Findings(_) => continue,
        };
        table = table.child(
            h_flex()
                .id(n)
                .items_start()
                .gap_3()
                .px_2()
                .py_0p5()
                .when(n % 2 == 1, |row| row.bg(stripe))
                .child(status.flex_none().w(rems(1.75)))
                .child(ui::label(r.name).flex_none().w(rems(14.0)))
                .child(ui::weak(detail.clone(), cx).flex_1().min_w_0()),
        );
        n += 1;
    }
    table
}

/// What was measured, the security warning, the steps checklist, then the Fix and firmware
/// buttons.
fn finding_body(f: &Finding, d: &Details, cx: &mut Context<OmniApp>) -> Vec<AnyElement> {
    let mut out = vec![ui::label(f.detail.clone()).into_any_element()];
    if f.security {
        out.push(ui::space(6.0).into_any_element());
        out.push(
            h_flex()
                .items_start()
                .gap_2()
                .child(security_icon())
                .child(
                    ui::colored(
                        "Security setting: only change it if you understand the protection you would be giving up.",
                        ORANGE,
                        cx,
                    )
                    .flex_1()
                    .min_w_0(),
                )
                .into_any_element(),
        );
    }
    if !f.steps.is_empty() {
        out.extend(steps_checklist(f, d.done, cx));
    }
    if let Some(a) = &f.fix {
        out.push(ui::space(10.0).into_any_element());
        out.push(
            h_flex()
                .child(crate::tabs::power::fix_button("health-fix", a, cx))
                .into_any_element(),
        );
    }
    if f.bios && d.uefi {
        out.push(ui::space(6.0).into_any_element());
        out.push(
            h_flex()
                .child(
                    Button::new("health-uefi").line_sized()
                        .caption("Restart into UEFI / BIOS…")
                        .tooltip(
                            "Restarts the PC straight into the firmware setup, no key presses needed. Asks for administrator rights.",
                        )
                        .disabled(d.restarting)
                        .on_click(cx.listener(|_, _, window, cx| restart_dialog(window, cx))),
                )
                .into_any_element(),
        );
    }
    out
}

/// Small orange warning triangle with "!" (a bundled icon, so it doesn't depend on font
/// glyphs).
fn security_icon() -> Icon {
    Icon::new(IconName::TriangleAlert)
        .small()
        .flex_none()
        .text_color(ORANGE)
}

/// The steps as a checklist: tick steps off as you go ("2 of 4 done"). Ticks only live in
/// memory for this session.
fn steps_checklist(
    f: &Finding,
    done: &HashMap<String, Vec<bool>>,
    cx: &mut Context<OmniApp>,
) -> Vec<AnyElement> {
    let total = f.steps.len();
    let mut ticks = done.get(&f.title).cloned().unwrap_or_default();
    ticks.resize(total, false);
    let n_done = ticks.iter().filter(|&&d| d).count();
    let weak = ui::weak_color(cx);
    let mut head = ui::row().child(
        ui::label("HOW TO FIX")
            .text_size(crate::ui::small_size())
            .text_color(weak),
    );
    if total > 1 {
        head = head.child(ui::weak(format!("· {n_done} of {total} done"), cx));
    }
    let mut out = vec![ui::space(8.0).into_any_element(), head.into_any_element()];
    for (i, s) in f.steps.iter().enumerate() {
        let finding = f.title.clone();
        let check = Checkbox::new("done")
            .checked(ticks[i])
            .tooltip(format!("Mark step {} of {total} as done", i + 1))
            .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                let ticks = this.views.health.done.entry(finding.clone()).or_default();
                ticks.resize(total, false);
                ticks[i] = *checked;
                cx.notify();
            }));
        let text = ui::label(format!("{}. {}", i + 1, s.text))
            .flex_1()
            .min_w_0()
            .when(ticks[i], |t| t.text_color(weak));
        let mut row = h_flex()
            .id(("health-step", i))
            .items_start()
            .gap_2()
            .py_0p5()
            .child(check)
            .child(text);
        if let Some(link) = &s.link {
            let url = link.url();
            row = row.child(
                Button::new("link")
                    .line_sized()
                    .caption(link.label())
                    .tooltip(url.clone())
                    .on_click(move |_, _, cx| cx.open_url(&url)),
            );
        }
        out.push(row.into_any_element());
    }
    out
}

/// "Restart into UEFI" confirmation.
fn restart_dialog(window: &mut Window, cx: &mut Context<OmniApp>) {
    ui::confirm(
        "Restart into UEFI / BIOS?",
        "The PC restarts immediately and opens the firmware setup. Save your work and close \
         other programs first: anything unsaved will be lost.\n\n\
         Windows asks for administrator rights to do this. In the setup, save and exit \
         (usually F10) to boot back into Windows.",
        "Restart now",
        window,
        cx,
        |this, _, cx| start_restart(this, cx),
    );
}

/// Restart into the firmware setup. The UAC prompt blocks the calling thread until answered,
/// so it runs off the UI thread; an error shows in the status bar.
fn start_restart(app: &mut OmniApp, cx: &mut Context<OmniApp>) {
    if app.views.health.restarting {
        return;
    }
    app.views.health.restarting = true;
    let restart = cx
        .background_executor()
        .spawn(async { crate::elevate::restart_to_firmware() });
    cx.spawn(async move |this, cx| {
        let result = restart.await;
        let _ = this.update(cx, |this, cx| {
            this.views.health.restarting = false;
            if let Err(e) = result {
                this.set_status(e, cx);
            }
            cx.notify();
        });
    })
    .detach();
}
