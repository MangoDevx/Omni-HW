//! Page tests: every page drawn from the fake machines in [`omnihw_core::testkit`] (an AMD
//! desktop with an NVIDIA GPU and an Intel laptop) in a window without a screen, and checked
//! through the text it painted. They catch pages that panic on real-looking data and labels
//! that go missing.
//!
//! The helpers here are shared: a page's own tests (in its module) open the app with
//! [`open`] or [`page`], drive it with `gpui_kit::test::TestWindowExt` (click, press, ...)
//! and read the result with [`texts`].

use crate::app::{OmniApp, Tab};
use crate::sampler::{Sampler, Snapshot};
use crate::settings::{NavStyle, Settings, Theme};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext, Bounds, Entity, Point, TestAppContext, WindowBounds,
    WindowOptions, px, size,
};
use omnihw_core::SystemInfo;
use omnihw_core::testkit::{desktop, laptop, sensor_table};

/// A snapshot with five samples of every sensor of `info`.
pub fn snapshot(info: &SystemInfo) -> Snapshot {
    Snapshot {
        table: sensor_table(info, 5),
        samples: 5,
        uptime_secs: Some(3 * 86_400 + 4 * 3600 + 12 * 60),
        ..Default::default()
    }
}

/// Both fake machines.
pub fn machines() -> [SystemInfo; 2] {
    [desktop(), laptop()]
}

/// Open the app on a fake sampler serving `info` and `snap`, in a window tall enough that
/// nothing is cut off. No sampling thread, tray icon or settings file is involved.
pub fn open(
    cx: &mut TestAppContext,
    info: SystemInfo,
    snap: Snapshot,
    settings: Settings,
) -> (AnyWindowHandle, Entity<OmniApp>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::app::bind_keys(cx);
        let bounds = Bounds {
            origin: Point::default(),
            size: size(px(1200.0), px(3000.0)),
        };
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| OmniApp::for_test(settings, Sampler::fake(info, snap), window, cx))
        })
        .expect("open the test window")
    })
}

/// Show `tab` without starting the reads a real page switch would.
pub fn go(cx: &mut TestAppContext, app: &Entity<OmniApp>, tab: Tab) {
    cx.update(|cx| {
        app.update(cx, |app, cx| {
            app.tab = tab;
            cx.notify();
        })
    });
}

/// Draw a frame and return every visible piece of text on it: the labels, then the names of
/// the controls (buttons, check boxes, ...), which is how assistive technology sees them.
pub fn texts(cx: &mut TestAppContext, window: AnyWindowHandle) -> Vec<String> {
    use gpui_kit::base::test_support;
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.draw(cx).clear(cx);
        let labels = test_support::texts(window);
        let controls = test_support::snapshots(window);
        labels
            .iter()
            .filter(|t| t.visible())
            .map(|t| t.text().to_string())
            .chain(
                controls
                    .iter()
                    .filter(|c| c.visible())
                    .filter_map(|c| c.label().map(str::to_string)),
            )
            .collect()
    })
    .expect("the test window is open")
}

/// Open the app with default settings on `tab` and return the text it shows.
pub fn page(cx: &mut TestAppContext, info: &SystemInfo, snap: Snapshot, tab: Tab) -> Vec<String> {
    let (window, app) = open(cx, info.clone(), snap, Settings::default());
    go(cx, &app, tab);
    texts(cx, window)
}

/// Every string in `wanted` appears somewhere in `texts`.
#[track_caller]
pub fn assert_shows(texts: &[String], wanted: &[&str]) {
    for w in wanted {
        assert!(
            texts.iter().any(|t| t.contains(w)),
            "page doesn't show {w:?}; it shows {texts:?}"
        );
    }
}

#[gpui_kit::test]
fn cpu_page(cx: &mut TestAppContext) {
    for info in machines() {
        let model = if info.cpu.vendor == omnihw_core::cpuid::Vendor::Amd {
            "Ryzen 7 7800X3D"
        } else {
            "i7-1360P"
        };
        let shown = page(cx, &info, snapshot(&info), Tab::Cpu);
        assert_shows(&shown, &[model, "Processor", "Clocks", "Topology"]);
    }
}

#[gpui_kit::test]
fn caches_page(cx: &mut TestAppContext) {
    for info in machines() {
        let shown = page(cx, &info, snapshot(&info), Tab::Caches);
        assert_shows(&shown, &["L1", "L3", "Associativity"]);
    }
}

#[gpui_kit::test]
fn mainboard_page(cx: &mut TestAppContext) {
    let info = desktop();
    let shown = page(cx, &info, snapshot(&info), Tab::Mainboard);
    assert_shows(&shown, &["ROG STRIX B650E-F GAMING WIFI", "2613"]);
    let info = laptop();
    let shown = page(cx, &info, snapshot(&info), Tab::Mainboard);
    assert_shows(&shown, &["LENOVO"]);
}

#[gpui_kit::test]
fn gpu_page(cx: &mut TestAppContext) {
    for info in machines() {
        let shown = page(cx, &info, snapshot(&info), Tab::Gpu);
        assert_shows(&shown, &[&info.gpus[0].name, "Graphics Card"]);
    }
    // NVML details on the NVIDIA card.
    let info = desktop();
    let shown = page(cx, &info, snapshot(&info), Tab::Gpu);
    assert_shows(&shown, &["5888", "581.57"]);
}

#[gpui_kit::test]
fn about_page(cx: &mut TestAppContext) {
    for info in machines() {
        let shown = page(cx, &info, snapshot(&info), Tab::About);
        assert_shows(&shown, &["Samples taken", "MIT"]);
    }
}

#[gpui_kit::test]
fn sensors_page(cx: &mut TestAppContext) {
    let info = desktop();
    let (window, app) = open(cx, info.clone(), snapshot(&info), Settings::default());
    go(cx, &app, Tab::Sensors);
    let shown = texts(cx, window);
    assert_shows(
        &shown,
        &[
            "GPU Temperature",
            "Package Power",
            "Reset min/max",
            "Average",
        ],
    );
    // A page with no samples yet shows its empty state.
    let shown = page(cx, &info, Snapshot::default(), Tab::Sensors);
    assert_shows(&shown, &["Waiting for first sample"]);
}

#[gpui_kit::test]
fn missing_text_fails(cx: &mut TestAppContext) {
    let info = desktop();
    let shown = page(cx, &info, snapshot(&info), Tab::About);
    assert_shows(&shown, &["Omni-HW"]);
    let missing = std::panic::catch_unwind(|| assert_shows(&shown, &["absent text"]));
    assert!(missing.is_err());
}

#[gpui_kit::test]
fn every_navigation_style_and_theme_draws(cx: &mut TestAppContext) {
    let info = desktop();
    for nav_style in [NavStyle::Sidebar, NavStyle::Tabs, NavStyle::Classic] {
        for theme in [Theme::Dark, Theme::Light, Theme::Retro] {
            let settings = Settings {
                nav_style,
                theme,
                ..Default::default()
            };
            let (window, app) = open(cx, info.clone(), snapshot(&info), settings);
            go(cx, &app, Tab::Cpu);
            let shown = texts(cx, window);
            // The page, and the navigation around it.
            assert_shows(&shown, &["Processor", "Record", "Report"]);
            let nav = match nav_style {
                NavStyle::Sidebar => ["Feature guide", "MONITORING"],
                NavStyle::Tabs => ["Hardware", "Mainboard"],
                NavStyle::Classic => ["LAN Security Report", "Event Log"],
            };
            assert_shows(&shown, &nav);
        }
    }
}

#[gpui_kit::test]
fn clicking_the_sidebar_opens_the_page(cx: &mut TestAppContext) {
    let info = desktop();
    let (window, app) = open(cx, info.clone(), snapshot(&info), Settings::default());
    texts(cx, window);
    cx.update_window(window, |_, window, cx| {
        window.click(("nav", Tab::Mainboard as usize), cx);
    })
    .unwrap();
    assert_eq!(cx.read(|cx| app.read(cx).tab), Tab::Mainboard);
    assert_shows(&texts(cx, window), &["ROG STRIX B650E-F GAMING WIFI"]);
}

#[gpui_kit::test]
fn arrow_keys_switch_pages(cx: &mut TestAppContext) {
    let info = desktop();
    let settings = Settings {
        nav_style: NavStyle::Classic,
        ..Default::default()
    };
    let (window, app) = open(cx, info.clone(), snapshot(&info), settings);
    go(cx, &app, Tab::Cpu);
    texts(cx, window);
    cx.update_window(window, |_, window, cx| window.press("right", cx))
        .unwrap();
    assert_eq!(cx.read(|cx| app.read(cx).tab), Tab::Caches);
    cx.update_window(window, |_, window, cx| window.press("left", cx))
        .unwrap();
    assert_eq!(cx.read(|cx| app.read(cx).tab), Tab::Cpu);
}
