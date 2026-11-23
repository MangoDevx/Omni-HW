//! In-app help: a window with one documentation tab per feature.
//!
//! Numbers that come from code (heatmap thresholds, bottleneck thresholds, the list of
//! Health Check rules) are read from the constants/code that use them, so the help stays in
//! step with the app.

mod hardware;
mod health;
mod monitoring;
mod start;

use crate::ui;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::separator::Separator;
use gpui_kit::component::{
    ActiveTheme, Icon, IconName, Selectable, Sizable, StyledExt, h_flex, v_flex,
};
use gpui_kit::{
    AnyElement, AnyWindowHandle, App, AppContext, Bounds, Context, ElementId, FontWeight, Hsla,
    InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, TitlebarOptions, WeakEntity, Window, WindowBounds,
    WindowOptions, div, px, rems, size,
};
use hardware::{hardware, lan_security, network, power, sensors};
use health::health_check;
use monitoring::{alerts, event_log, recording, reports};
use start::{getting_started, overview, privacy, settings, troubleshooting};
use std::cell::Cell;
use std::collections::HashSet;
use std::rc::Rc;

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum HelpTab {
    #[default]
    GettingStarted,
    Overview,
    HealthCheck,
    Hardware,
    Network,
    LanSecurity,
    Power,
    Sensors,
    Recording,
    Alerts,
    EventLog,
    Reports,
    Settings,
    Privacy,
    Troubleshooting,
}

const TABS: [(HelpTab, &str); 15] = [
    (HelpTab::GettingStarted, "Getting Started"),
    (HelpTab::Overview, "Overview"),
    (HelpTab::HealthCheck, "Health Check"),
    (HelpTab::Hardware, "Hardware Tabs"),
    (HelpTab::Network, "Network"),
    (HelpTab::LanSecurity, "LAN Security"),
    (HelpTab::Power, "Power"),
    (HelpTab::Sensors, "Sensors"),
    (HelpTab::Recording, "Recording"),
    (HelpTab::Alerts, "Alerts"),
    (HelpTab::EventLog, "Event Log"),
    (HelpTab::Reports, "Reports"),
    (HelpTab::Settings, "Settings"),
    (HelpTab::Privacy, "Privacy & Safety"),
    (HelpTab::Troubleshooting, "Troubleshooting"),
];

/// Help window state: the help window (its own OS window) while it is open.
#[derive(Default)]
pub struct HelpState {
    window: Option<AnyWindowHandle>,
    /// Shared with the window's close handler, which clears it when the user closes the
    /// window themselves.
    open: Rc<Cell<bool>>,
}

impl HelpState {
    /// Whether the help window is open (the sidebar highlights its row).
    pub fn is_open(&self) -> bool {
        self.open.get()
    }

    /// Open the help window, or close it if it is open.
    pub fn toggle(&mut self, cx: &mut App) {
        if self.open.get() {
            if let Some(handle) = self.window.take() {
                let _ = handle.update(cx, |_, window, _| window.remove_window());
            }
            self.open.set(false);
            return;
        }
        let bounds = Bounds::centered(None, size(px(760.), px(560.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("Omni-HW Help".into()),
                ..Default::default()
            }),
            window_min_size: Some(size(px(360.), px(240.))),
            is_resizable: true,
            ..Default::default()
        };
        let Ok((handle, _)) =
            gpui_kit::open_window(options, cx, |_, cx| cx.new(|_| HelpView::default()))
        else {
            return;
        };
        // The user closing the help window: un-highlight the main window's sidebar row.
        let open = self.open.clone();
        let _ = handle.update(cx, |_, window, cx| {
            window.on_window_should_close(cx, move |_, cx| {
                open.set(false);
                cx.refresh_windows();
                true
            });
        });
        self.open.set(true);
        self.window = Some(handle);
    }
}

/// The help window's view: the selected tab and the expanded "Details" sections (both kept
/// while the window is open).
#[derive(Default)]
pub struct HelpView {
    tab: HelpTab,
    expanded: HashSet<&'static str>,
}

impl Render for HelpView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let current = self.tab;
        let tabs = ui::wrap_row()
            .flex_none()
            .gap_1()
            .children(TABS.iter().enumerate().map(|(i, &(tab, name))| {
                Button::new(("help-tab", i))
                    .small()
                    .ghost()
                    .label(name)
                    .selected(tab == current)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.tab = tab;
                        cx.notify();
                    }))
            }));

        let mut doc = Doc {
            items: Vec::new(),
            expanded: &self.expanded,
            view: cx.weak_entity(),
            weak: ui::weak_color(cx),
        };
        match current {
            HelpTab::GettingStarted => getting_started(&mut doc),
            HelpTab::Overview => overview(&mut doc),
            HelpTab::HealthCheck => health_check(&mut doc),
            HelpTab::Hardware => hardware(&mut doc),
            HelpTab::Network => network(&mut doc),
            HelpTab::LanSecurity => lan_security(&mut doc),
            HelpTab::Power => power(&mut doc),
            HelpTab::Sensors => sensors(&mut doc),
            HelpTab::Recording => recording(&mut doc),
            HelpTab::Alerts => alerts(&mut doc),
            HelpTab::EventLog => event_log(&mut doc),
            HelpTab::Reports => reports(&mut doc),
            HelpTab::Settings => settings(&mut doc),
            HelpTab::Privacy => privacy(&mut doc),
            HelpTab::Troubleshooting => troubleshooting(&mut doc),
        }
        // Each tab keeps its own scroll position.
        let content = v_flex()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .pr_3()
            .children(doc.items)
            .overflow_y_scrollbar()
            .id(("help-scroll", current as usize));

        let t = cx.theme();
        v_flex()
            .id("help")
            .size_full()
            .p_3()
            .gap_2()
            .bg(t.background)
            .text_color(t.foreground)
            .font_family(t.font_family.clone())
            .text_size(t.font_size)
            .child(tabs)
            .child(Separator::horizontal())
            .child(content)
    }
}

// ---- Formatting helpers ---------------------------------------------------------------------

/// The content of one help tab, built top to bottom by the helpers below.
struct Doc<'a> {
    items: Vec<AnyElement>,
    /// The "Details" sections that are open.
    expanded: &'a HashSet<&'static str>,
    view: WeakEntity<HelpView>,
    /// The theme's secondary text colour (for "Details").
    weak: Hsla,
}

impl Doc<'_> {
    /// Append one block. Each block gets its own id, so labels with the same text in
    /// different blocks (every bullet's "•") have unique ids.
    fn push(&mut self, block: impl IntoElement) {
        let n = self.items.len();
        let block = div().id(("help-item", n)).w_full().min_w_0().child(block);
        self.items.push(block.into_any_element());
    }
}

fn h(doc: &mut Doc, text: &str) {
    doc.push(
        v_flex()
            .child(ui::space(8.0))
            .child(
                ui::label(text.to_string())
                    .text_size(rems(16.0 / 14.0))
                    .font_weight(FontWeight::BOLD),
            )
            .child(ui::space(2.0)),
    );
}

fn p(doc: &mut Doc, text: &str) {
    doc.push(
        v_flex()
            .child(ui::label(text.to_string()).min_w_0())
            .child(ui::space(4.0)),
    );
}

/// A bold line (the category names above the Health Check rules).
fn strong(doc: &mut Doc, text: &str) {
    doc.push(ui::strong(text.to_string()));
}

fn bullets<S: AsRef<str>>(doc: &mut Doc, items: &[S]) {
    for t in items {
        doc.push(
            h_flex()
                .items_start()
                .gap_1p5()
                .child(ui::label("•").flex_none())
                .child(ui::label(t.as_ref().to_string()).flex_1().min_w_0()),
        );
    }
    doc.push(ui::space(4.0));
}

/// Smaller, collapsed technical notes for curious readers.
fn details<S: AsRef<str>>(doc: &mut Doc, id: &'static str, items: &[S]) {
    let open = doc.expanded.contains(id);
    let view = doc.view.clone();
    let weak = doc.weak;
    let header = ui::clickable(details_id(id))
        .h_flex()
        .gap_1()
        .py_0p5()
        .cursor_pointer()
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .child(
            Icon::new(if open {
                IconName::ChevronDown
            } else {
                IconName::ChevronRight
            })
            .small()
            .text_color(weak),
        )
        .child(ui::label("Details"))
        .on_click(move |_, _, cx| {
            let _ = view.update(cx, |view, cx| {
                if !view.expanded.remove(id) {
                    view.expanded.insert(id);
                }
                cx.notify();
            });
        });
    doc.push(div().flex().child(header));
    if open {
        for t in items {
            doc.push(
                h_flex()
                    .items_start()
                    .gap_1p5()
                    .text_xs()
                    .child(ui::label("•").flex_none().text_color(weak))
                    .child(
                        ui::label(t.as_ref().to_string())
                            .text_color(weak)
                            .flex_1()
                            .min_w_0(),
                    ),
            );
        }
    }
    doc.push(ui::space(4.0));
}

/// The id of the "Details" header `id` (tests click it).
fn details_id(id: &str) -> ElementId {
    ElementId::Name(SharedString::from(format!("help-details-{id}")))
}

fn pct(f: f64) -> String {
    format!("{:.0} %", f * 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tabs::page_tests::{assert_shows, texts};
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{Entity, Point, TestAppContext};

    /// Open the help view in a window without a screen, `height` logical pixels tall.
    fn open_help(cx: &mut TestAppContext, height: f32) -> (AnyWindowHandle, Entity<HelpView>) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            let bounds = Bounds {
                origin: Point::default(),
                size: size(px(760.0), px(height)),
            };
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            };
            gpui_kit::open_window(options, cx, |_, cx| cx.new(|_| HelpView::default()))
                .expect("open the help window")
        })
    }

    fn click(cx: &mut TestAppContext, window: AnyWindowHandle, id: impl Into<ElementId>) {
        let id = id.into();
        cx.update_window(window, |_, window, cx| window.click(id, cx))
            .expect("the help window is open");
    }

    /// First heading of each tab (after the intro paragraph on tabs that start with one).
    const FIRST_HEADINGS: [&str; 15] = [
        "What is Omni-HW?",
        "What it shows",
        "What it does",
        "CPU",
        "What it shows",
        "What it does",
        "What it shows",
        "What it shows",
        "Recording a session",
        "What it does",
        "What it shows",
        "Save report",
        "Appearance",
        "Everything stays on your PC",
        "A sensor or value is missing",
    ];

    #[gpui_kit::test]
    fn every_tab_draws(cx: &mut TestAppContext) {
        let (window, view) = open_help(cx, 560.0);
        assert_shows(&texts(cx, window), &["What is Omni-HW?", "Troubleshooting"]);
        for (i, (tab, _)) in TABS.iter().enumerate() {
            click(cx, window, ("help-tab", i));
            let shown = texts(cx, window);
            assert_eq!(cx.read(|cx| view.read(cx).tab), *tab);
            assert!(
                shown.iter().any(|t| t == FIRST_HEADINGS[i]),
                "{tab:?} doesn't show {:?}; it shows {shown:?}",
                FIRST_HEADINGS[i]
            );
        }
    }

    #[gpui_kit::test]
    fn details_expand(cx: &mut TestAppContext) {
        let (window, view) = open_help(cx, 1200.0);
        click(cx, window, ("help-tab", HelpTab::Hardware as usize));
        let line = "Read with the CPUID instruction";
        let shown = texts(cx, window);
        assert!(!shown.iter().any(|t| t.contains(line)));
        click(cx, window, details_id("cpu"));
        assert!(cx.read(|cx| view.read(cx).expanded.contains("cpu")));
        assert_shows(&texts(cx, window), &[line, "VT-x may be missing"]);
        // A second click collapses it again.
        click(cx, window, details_id("cpu"));
        assert!(!texts(cx, window).iter().any(|t| t.contains(line)));
    }

    #[gpui_kit::test]
    fn toggle_opens_and_closes_the_window(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let mut help = HelpState::default();
        let before = cx.update(|cx| cx.windows().len());
        cx.update(|cx| help.toggle(cx));
        assert!(help.is_open());
        cx.run_until_parked();
        assert_eq!(cx.update(|cx| cx.windows().len()), before + 1);
        cx.update(|cx| help.toggle(cx));
        assert!(!help.is_open());
        cx.run_until_parked();
        assert_eq!(cx.update(|cx| cx.windows().len()), before);

        // The user closing the help window themselves clears the flag too.
        cx.update(|cx| help.toggle(cx));
        assert!(help.is_open());
        let handle = help.window.expect("the help window is open");
        assert!(gpui_kit::VisualTestContext::from_window(handle, cx).simulate_close());
        assert!(!help.is_open());
    }

    #[gpui_kit::test]
    fn paragraphs_wrap_inside_the_window(cx: &mut TestAppContext) {
        let (window, _) = open_help(cx, 560.0);
        texts(cx, window);
        let bounds = cx
            .update_window(window, |_, window, _| {
                gpui_kit::base::test_support::texts(window)
                    .into_iter()
                    .find(|t| t.text().starts_with("Omni-HW combines what CPU-Z"))
                    .map(|t| t.bounds())
            })
            .unwrap()
            .expect("the first paragraph is drawn");
        assert!(bounds.right() <= px(760.0), "{bounds:?}");
        // More than one line high: the paragraph wrapped.
        assert!(bounds.size.height > px(30.0), "{bounds:?}");
    }

    #[gpui_kit::test]
    fn numbers_come_from_the_code(cx: &mut TestAppContext) {
        // Tall enough to show every Health Check rule.
        let (window, _) = open_help(cx, 12_000.0);
        click(cx, window, ("help-tab", HelpTab::HealthCheck as usize));
        let shown = texts(cx, window);
        assert_shows(&shown, &["All checks", "CPU", "Uptime: Windows running"]);
        for (name, _) in health::all_checks() {
            assert_shows(&shown, &[name]);
        }

        click(cx, window, ("help-tab", HelpTab::Overview as usize));
        let core = format!(
            "orange with a warning triangle from {:.0} %, red with an exclamation mark from {:.0} %",
            crate::heat::CORE_LOAD.warn,
            crate::heat::CORE_LOAD.crit
        );
        let memory = format!(
            "Memory bar: turns orange from {} used",
            pct(crate::heat::MEMORY_USED.warn)
        );
        assert_shows(&texts(cx, window), &[&core, &memory]);

        click(cx, window, ("help-tab", HelpTab::Recording as usize));
        let gpu_bound = format!(
            "GPU-bound: GPU load at {:.0} % or more",
            omnihw_core::bottleneck::GPU_BOUND
        );
        assert_shows(&texts(cx, window), &[&gpu_bound]);
    }

    #[test]
    fn percentages() {
        assert_eq!(pct(0.8), "80 %");
        assert_eq!(pct(0.85), "85 %");
    }
}
