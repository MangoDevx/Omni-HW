//! Page tests: every page drawn from the fake machines in [`omnihw_core::testkit`] (an AMD
//! desktop with an NVIDIA GPU and an Intel laptop) and checked through the accessibility tree.
//! They catch pages that panic on real-looking data and labels that go missing.

use crate::sampler::{Sampler, Snapshot};
use egui_kittest::kittest::Queryable;
use egui_kittest::{Harness, SnapshotResults};
use omnihw_core::SystemInfo;
use omnihw_core::testkit::{desktop, laptop, sensor_table};

/// Screenshot tests run locally only: GitHub's runners have no GPU to render them with.
fn screenshots_enabled() -> bool {
    std::env::var_os("GITHUB_ACTIONS").is_none()
}

/// Draw `page` at `size` and compare it with `tests/snapshots/<name>.png` (create or update the
/// file with `UPDATE_SNAPSHOTS=1`). Results are collected in `results`, which reports every
/// mismatch of a test at once when it drops.
fn screenshot<'a>(
    results: &mut SnapshotResults,
    name: &str,
    size: egui::Vec2,
    page: impl FnMut(&mut egui::Ui) + 'a,
) {
    if !screenshots_enabled() {
        return;
    }
    let mut h = Harness::builder().with_size(size).build_ui(page);
    h.run_steps(4);
    h.snapshot(name);
    results.extend(h.take_snapshot_results());
}

fn snapshot(info: &SystemInfo) -> Snapshot {
    Snapshot {
        table: sensor_table(info, 5),
        samples: 5,
        uptime_secs: Some(3 * 86_400 + 4 * 3600 + 12 * 60),
        ..Default::default()
    }
}

/// Draw `page` in a window tall enough that nothing is cut off. A few frames, not "until idle":
/// spinners and some layouts keep asking for repaints.
fn draw<'a>(page: impl FnMut(&mut egui::Ui) + 'a) -> Harness<'a> {
    let mut h = Harness::builder()
        .with_size(egui::vec2(1200.0, 3000.0))
        .build_ui(page);
    h.run_steps(4);
    h
}

/// Every text in `texts` appears somewhere on the page.
fn assert_shows(h: &Harness<'_>, texts: &[&str]) {
    for t in texts {
        assert!(
            h.query_all_by_label_contains(t).next().is_some(),
            "page doesn't show {t:?}"
        );
    }
}

/// (machine, texts its pages must show) for the hardware pages.
fn machines() -> [(SystemInfo, &'static str); 2] {
    [(desktop(), "desktop"), (laptop(), "laptop")]
}

#[test]
fn cpu_page() {
    for (info, _) in machines() {
        let snap = snapshot(&info);
        let h = draw(|ui| super::cpu::show(ui, &info, &snap));
        let model = if info.cpu.vendor == omnihw_core::cpuid::Vendor::Amd {
            "Ryzen 7 7800X3D"
        } else {
            "i7-1360P"
        };
        assert_shows(&h, &[model]);
    }
}

#[test]
fn caches_page() {
    for (info, _) in machines() {
        let h = draw(|ui| super::caches::show(ui, &info));
        assert_shows(&h, &["L1", "L3"]);
    }
}

#[test]
fn mainboard_page() {
    let info = desktop();
    let h = draw(|ui| super::mainboard::show(ui, &info));
    assert_shows(&h, &["ROG STRIX B650E-F GAMING WIFI", "2613"]);
    let info = laptop();
    let h = draw(|ui| super::mainboard::show(ui, &info));
    assert_shows(&h, &["LENOVO"]);
}

#[test]
fn memory_page() {
    for (info, _) in machines() {
        let snap = snapshot(&info);
        let h = draw(|ui| super::memory::show(ui, &info, &snap));
        let part = if info.battery.is_some() {
            "MT62F1G32D2DS-026"
        } else {
            "KF560C30-16"
        };
        assert_shows(&h, &[part]);
    }
}

#[test]
fn gpu_page() {
    for (info, _) in machines() {
        let snap = snapshot(&info);
        let mut selected = 0;
        let name = info.gpus[0].name.clone();
        let h = draw(|ui| super::gpu::show(ui, &info, &snap, &mut selected));
        assert_shows(&h, &[&name]);
    }
    // NVML details on the NVIDIA card.
    let info = desktop();
    let snap = snapshot(&info);
    let mut selected = 0;
    let h = draw(|ui| super::gpu::show(ui, &info, &snap, &mut selected));
    assert_shows(&h, &["5888", "581.57"]);
}

#[test]
fn storage_page() {
    for (info, _) in machines() {
        let snap = snapshot(&info);
        let model = info.drives[0].model.clone();
        let h = draw(|ui| super::storage::show(ui, &info, &snap));
        assert_shows(&h, &[&model]);
    }
    let info = desktop();
    let snap = snapshot(&info);
    let h = draw(|ui| super::storage::show(ui, &info, &snap));
    assert_shows(&h, &["ST4000DM004-2U9104"]);
}

#[test]
fn about_page() {
    for (info, _) in machines() {
        let snap = snapshot(&info);
        let h = draw(|ui| super::about::show(ui, &info, &snap));
        assert_shows(&h, &["Samples taken", "MIT"]);
    }
}

#[test]
fn overview_and_health_pages() {
    for (info, _) in machines() {
        let snap = snapshot(&info);
        let mut cache = super::health::HealthCache::default();
        let report = cache.get(&info, &snap, true);
        let h = draw(|ui| {
            super::overview::show(ui, &info, &snap, report);
        });
        assert_shows(&h, &[&info.gpus[0].name]);
        let h = draw(|ui| super::health::show(ui, &info, report));
        assert_shows(&h, &["Health Check"]);
    }
}

#[test]
fn snapshot_pages_without_data() {
    // Pages that wait for a background read show their empty state without panicking.
    let snap = Snapshot::default();
    draw(|ui| super::network::show(ui, &snap));
    draw(|ui| super::lansec::show(ui, &snap));
    draw(|ui| super::power::show(ui, &snap));
}

#[test]
fn sampler_pages() {
    let info = desktop();
    let sampler = Sampler::fake(info.clone(), snapshot(&info));
    let h = draw(|ui| super::sensors::show(ui, &sampler));
    assert_shows(&h, &["GPU Temperature", "Package Power"]);

    let mut state = super::eventlog::ViewState::default();
    let h = draw(|ui| super::eventlog::show(ui, &sampler, &mut state));
    assert_shows(&h, &["Event Log"]);

    let mut state = super::alerts::ViewState::default();
    let h = draw(|ui| super::alerts::show(ui, &info, &sampler, &mut state));
    assert_shows(&h, &["Alerts"]);

    let mut state = super::recording::ViewState::default();
    draw(|ui| {
        super::recording::show(ui, &sampler, &mut state);
    });
}

#[test]
fn missing_text_fails() {
    let h = draw(|ui| {
        ui.label("present");
    });
    assert_shows(&h, &["present"]);
    let missing = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        assert_shows(&h, &["absent"])
    }));
    assert!(missing.is_err());
}

fn cpu_section(ui: &mut egui::Ui) {
    super::section(
        ui,
        "Processor",
        &[
            ("Name", "AMD Ryzen 7 9800X3D".into()),
            ("Stepping", String::new()),
        ],
    );
}

#[test]
fn section_shows_values_and_dashes() {
    let h = draw(cpu_section);
    assert_shows(&h, &["Processor", "AMD Ryzen 7 9800X3D"]);
    // Empty values show a dash.
    h.get_by_label("-");
}

#[test]
fn settings_page() {
    let info = desktop();
    let sampler = Sampler::fake(info.clone(), snapshot(&info));
    let mut settings = crate::settings::Settings::default();
    let updates = crate::update::UpdateChecker::default();
    let pawnio = crate::pawnio::Installer::default();
    let h = draw(|ui| {
        super::settings::show(ui, &mut settings, &sampler, &updates, &pawnio);
    });
    assert_shows(&h, &["Settings", "Check for updates"]);
}

#[test]
fn screenshots() {
    let mut results = SnapshotResults::new();
    screenshot(
        &mut results,
        "section",
        egui::vec2(360.0, 90.0),
        cpu_section,
    );
    let info = desktop();
    let snap = snapshot(&info);
    let size = egui::vec2(1100.0, 900.0);
    screenshot(&mut results, "cpu_desktop", size, |ui| {
        super::cpu::show(ui, &info, &snap)
    });
    let mut selected = 0;
    screenshot(&mut results, "gpu_desktop", size, |ui| {
        super::gpu::show(ui, &info, &snap, &mut selected)
    });
    // No health findings, so the picture doesn't change with today's date.
    let report = super::health::Report::default();
    screenshot(&mut results, "overview_desktop", size, |ui| {
        super::overview::show(ui, &info, &snap, &report);
    });
    let info = laptop();
    let snap = snapshot(&info);
    screenshot(&mut results, "overview_laptop", size, |ui| {
        super::overview::show(ui, &info, &snap, &report);
    });
}
