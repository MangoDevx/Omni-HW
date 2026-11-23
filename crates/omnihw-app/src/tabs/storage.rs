//! Storage page: every physical drive with its device details, speed test, volumes, NVMe or
//! SATA SMART health and sensors.

use crate::app::OmniApp;
use crate::heat::{GREEN, ORANGE, RED};
use crate::sampler::Snapshot;
use crate::ui::LineSized;
use crate::ui::{self, Table, section};
use gpui_kit::component::button::Button;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::{
    AnyElement, App, Context, ElementId, InteractiveElement, IntoElement, ParentElement,
    SharedString, Styled, Window, div, relative, rems,
};
use omnihw_core::checkup::PcieLink;
use omnihw_core::disktest::{self, DiskTestResult, Verdict};
use omnihw_core::format_bytes;
use omnihw_core::smart::AtaSmart;
use omnihw_core::storage::{BusType, Drive, sata_gen_speed};

pub fn show(app: &mut OmniApp, _window: &mut Window, cx: &mut Context<OmniApp>) -> AnyElement {
    let app = &*app;
    let Some(info) = app.sampler.info() else {
        return ui::column().into_any_element();
    };
    if info.drives.is_empty() {
        return ui::label("No physical drives found.").into_any_element();
    }
    let snap = app.sampler.snapshot();
    ui::column()
        .gap_2p5()
        .children(
            info.drives
                .iter()
                .map(|d| drive(app, d, &snap, &info.checkup.nvme_links, cx)),
        )
        .into_any_element()
}

/// Summary of the key SMART values plus a collapsible table of every attribute.
fn smart_section(app: &OmniApp, d: &Drive, s: &AtaSmart, cx: &mut Context<OmniApp>) -> AnyElement {
    let opt = |v: Option<u64>| v.map(|v| v.to_string()).unwrap_or_else(|| "-".into());
    let mut rows = vec![];
    if let Some(life) = s.life_remaining_pct() {
        rows.push(("Life Remaining", format!("{life} %")));
    }
    if let Some(t) = s.temperature_c() {
        rows.push(("Temperature", format!("{t} °C")));
    }
    rows.push(("Power-On Hours", opt(s.power_on_hours())));
    rows.push(("Power Cycles", opt(s.power_cycles())));
    rows.push(("Reallocated Sectors", opt(s.reallocated_sectors())));
    rows.push(("Pending Sectors", opt(s.pending_sectors())));
    rows.push(("Uncorrectable Errors", opt(s.uncorrectable())));
    rows.push(("CRC (Cable) Errors", opt(s.crc_errors())));
    let key = format!("storage/smart/{}", d.index);
    let open = ui::is_open(app, &key, false);
    let mut out = ui::column()
        .id("sata-smart")
        .pb_1p5()
        .child(section("Health (SATA SMART)", &rows, cx))
        .child(ui::collapse_header(
            key,
            ui::label(format!("SMART attributes ({})", s.attributes.len())),
            open,
            cx,
        ));
    if open {
        let mut table = Table::new(&["ID", "Attribute", "Current", "Worst", "Raw"]).striped();
        for a in &s.attributes {
            table.push_row([
                ui::mono(format!("{:02X}", a.id), cx).into_any_element(),
                ui::label(a.name).into_any_element(),
                ui::mono(a.current.to_string(), cx).into_any_element(),
                ui::mono(a.worst.to_string(), cx).into_any_element(),
                ui::mono(a.raw.to_string(), cx).into_any_element(),
            ]);
        }
        out = out.child(table.build(cx).pl_4());
    }
    out.into_any_element()
}

fn drive(
    app: &OmniApp,
    d: &Drive,
    snap: &Snapshot,
    links: &[PcieLink],
    cx: &mut Context<OmniApp>,
) -> AnyElement {
    // Every drive repeats the same section titles and labels: an id per drive keeps them apart.
    let mut col = ui::column().id(("drive", d.index as usize)).child(
        ui::strong(format!("Disk {}: {}", d.index, d.name()))
            .text_size(gpui_kit::rems(1.0))
            .pb_1(),
    );
    col = col.child(
        section(
            "Device",
            &[
                ("Type", d.kind().to_string()),
                ("Interface", d.bus.name()),
                ("Capacity", format_bytes(d.size_bytes)),
                ("Firmware", d.firmware.clone()),
                ("Serial Number", d.serial.trim_end_matches('.').to_string()),
                ("Removable", if d.removable { "Yes" } else { "No" }.into()),
            ],
            cx,
        )
        .id("device"),
    );
    col = col.child(speed_test(d, snap, disktest::nvme_link(d, links), cx));

    if !d.volumes.is_empty() {
        let mut group = ui::group_box(cx).id("volumes").child(ui::strong("Volumes"));
        for (i, v) in d.volumes.iter().enumerate() {
            let used = v.total_bytes.saturating_sub(v.free_bytes);
            let frac = if v.total_bytes > 0 {
                used as f32 / v.total_bytes as f32
            } else {
                0.0
            };
            let label = if v.label.is_empty() {
                String::new()
            } else {
                format!(" {}", v.label)
            };
            group = group.child(text_bar(
                ("volume", i),
                frac,
                format!(
                    "{}{label} ({}) - {} free of {}",
                    v.mount,
                    v.file_system,
                    format_bytes(v.free_bytes),
                    format_bytes(v.total_bytes)
                ),
                cx,
            ));
        }
        col = col.child(group);
    }

    match &d.nvme_health {
        Some(h) => {
            let warnings = h.warnings();
            let tb = |b: u128| format!("{:.2} TB", b as f64 / 1e12);
            col = col.child(
                section(
                    "Health (NVMe SMART)",
                    &[
                        (
                            "Health",
                            format!("{}% (life used {}%)", h.health_pct(), h.percentage_used),
                        ),
                        (
                            "Available Spare",
                            format!(
                                "{}% (threshold {}%)",
                                h.available_spare_pct, h.spare_threshold_pct
                            ),
                        ),
                        ("Temperature", format!("{} °C", h.temperature_c)),
                        ("Data Written", tb(h.data_written_bytes)),
                        ("Data Read", tb(h.data_read_bytes)),
                        ("Power-On Hours", h.power_on_hours.to_string()),
                        ("Power Cycles", h.power_cycles.to_string()),
                        ("Unsafe Shutdowns", h.unsafe_shutdowns.to_string()),
                        ("Media Errors", h.media_errors.to_string()),
                        (
                            "Critical Warnings",
                            if warnings.is_empty() {
                                "None".into()
                            } else {
                                warnings.join(", ")
                            },
                        ),
                    ],
                    cx,
                )
                .id("nvme-health"),
            );
        }
        None if d.bus == BusType::Nvme => {
            col = col.child(ui::weak(
                "NVMe health log unavailable (try running as administrator).",
                cx,
            ));
        }
        None => {}
    }

    match &d.ata_smart {
        Some(s) => col = col.child(smart_section(app, d, s, cx)),
        None if !matches!(
            d.bus,
            BusType::Nvme
                | BusType::Usb
                | BusType::Virtual
                | BusType::Sd
                | BusType::Mmc
                | BusType::Spaces
        ) =>
        {
            col = col.child(ui::weak(
                "SATA SMART data unavailable (try running as administrator).",
                cx,
            ));
        }
        None => {}
    }

    let prefix = format!("{}/", d.sensor_prefix());
    let rows: Vec<_> = snap
        .table
        .entries()
        .iter()
        .filter(|t| t.reading.id.starts_with(&prefix))
        .collect();
    if !rows.is_empty() {
        col = col
            .child(ui::strong("Sensors"))
            .child(super::sensors::sensor_grid(
                "drive-sensors",
                rows.into_iter(),
                cx,
            ));
    }
    col.into_any_element()
}

/// The "Speed test" group: run button (confirmed in a dialog), live progress and the last
/// result for this drive.
fn speed_test(
    d: &Drive,
    snap: &Snapshot,
    link: Option<&PcieLink>,
    cx: &mut Context<OmniApp>,
) -> impl IntoElement {
    let state = &snap.disk_test;
    let running_here = state.running == Some(d.index);
    let last = state.results.get(&d.index);
    let mut controls = ui::row();
    if running_here {
        controls = controls
            .child(Spinner::new().small())
            .child(ui::label("Testing..."));
    } else {
        let busy = state.running.is_some();
        let can_run = !busy && disktest::pick_volume(d).is_some();
        let tip = if can_run {
            "Measure read and write speed with a temporary file".to_string()
        } else if busy {
            "Another drive is being tested".to_string()
        } else {
            format!(
                "No volume with enough free space (needs {} free)",
                format_bytes(disktest::needed_free_bytes(d))
            )
        };
        let index = d.index;
        controls = controls.child(
            Button::new(("disk-test", index as usize))
                .line_sized()
                .caption("Run speed test")
                .tooltip(tip)
                .disabled(!can_run)
                .on_click(cx.listener(move |this, _, window, cx| {
                    confirm_disk_test(this, index, window, cx)
                })),
        );
    }
    if let Some((_, at)) = last {
        controls = controls.child(ui::weak(format!("Last run {}", ui::ago(*at)), cx));
    }

    let mut group = ui::group_box(cx)
        .id("speed-test")
        .child(ui::strong("Speed test"))
        .child(controls);
    if running_here {
        let (phase, frac) = state.progress.read();
        group = group.child(
            text_bar("disk-test-progress", frac, phase.label().to_string(), cx).w(rems(22.5)),
        );
    }
    match last {
        Some((Ok(r), _)) => group = group.child(results(d, r, link, cx)),
        Some((Err(e), _)) => group = group.child(ui::colored(e.clone(), RED, cx)),
        None if !running_here => {
            group = group.child(ui::weak(
                "Measures sequential and 4K random read/write speed and compares it with \
                 what this drive should deliver.",
                cx,
            ));
        }
        None => {}
    }
    if let Some((_, events)) = state.whea.get(&d.index) {
        group = group.children(ui::whea_warning("disk-whea", events, cx));
    }
    group
}

/// Ask before the speed test on drive `index`, and start it once confirmed.
fn confirm_disk_test(
    app: &mut OmniApp,
    index: u32,
    window: &mut Window,
    cx: &mut Context<OmniApp>,
) {
    let target = app.sampler.info().and_then(|info| {
        let d = info.drives.iter().find(|d| d.index == index)?;
        let v = disktest::pick_volume(d)?;
        Some((format_bytes(disktest::file_bytes(d)), v.mount.clone()))
    });
    let Some((size, mount)) = target else {
        return;
    };
    ui::confirm(
        "Run speed test?",
        format!(
            "Writes a temporary {size} file to {mount} and deletes it afterwards. Takes about 20 \
             seconds; other disk activity will skew results."
        ),
        "Run test",
        window,
        cx,
        move |this, _, cx| {
            this.sampler.run_disk_test(index);
            super::memory::redraw_while(cx, |app| {
                app.sampler.snapshot().disk_test.running.is_some()
            });
        },
    );
}

/// A bar filled to `frac` (0..=1) with `text` on it, like a progress bar with a caption.
fn text_bar(
    id: impl Into<ElementId>,
    frac: f32,
    text: impl Into<SharedString>,
    cx: &App,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    // egui's progress bar: a round-ended selection-coloured fill on the darkest background.
    let p = crate::theme::palette(cx);
    div()
        .id(id)
        .relative()
        .w_full()
        .h(rems(18.0 / 16.0))
        .overflow_hidden()
        .rounded_full()
        .bg(p.extreme)
        .child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .h_full()
                .w(relative(frac.clamp(0.0, 1.0)))
                .rounded_full()
                .bg(p.selection),
        )
        .child(
            div()
                .relative()
                .h_full()
                .flex()
                .items_center()
                .px(gpui_kit::px(8.0))
                .child(ui::label(text).text_color(p.selection_text)),
        )
}

fn mbs(v: f64) -> String {
    if v < 10.0 {
        format!("{v:.1} MB/s")
    } else {
        format!("{v:.0} MB/s")
    }
}

fn results(d: &Drive, r: &DiskTestResult, link: Option<&PcieLink>, cx: &App) -> impl IntoElement {
    let mut cells: Vec<AnyElement> = Vec::new();
    for (k, v) in [
        ("Sequential read", r.seq_read_mbs),
        ("Sequential write", r.seq_write_mbs),
        ("4K random read", r.rnd4k_read_mbs),
        ("4K random write", r.rnd4k_write_mbs),
        ("4K random read (QD32)", r.rnd4k_read_q32_mbs),
    ] {
        cells.push(ui::label(k).into_any_element());
        cells.push(ui::mono(mbs(v), cx).into_any_element());
    }
    cells.push(ui::label("Typical sequential read").into_any_element());
    cells.push(
        ui::label(match &r.expected {
            Some(e) => format!(
                "{:.0}-{:.0} MB/s ({})",
                e.seq_read_mbs.0, e.seq_read_mbs.1, e.reason
            ),
            None => "Depends on the port or enclosure".into(),
        })
        .into_any_element(),
    );
    // Two results can read the same: an id per cell keeps their labels apart.
    let grid = ui::grid(2).children(
        cells
            .into_iter()
            .enumerate()
            .map(|(i, c)| div().id(i).child(c)),
    );
    let mut out = ui::column().pt_1().child(ui::hug(grid));
    if let Some(v) = r.verdict() {
        let (text, color) = match v {
            Verdict::Good => ("Normal", GREEN),
            Verdict::Low => ("Slow", ORANGE),
            Verdict::VeryLow => ("Very slow", RED),
        };
        out = out.child(
            ui::wrap_row()
                .child(ui::pill(text, color, false, cx))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(ui::label(verdict_text(d, v))),
                ),
        );
    }
    if let Some(note) = link_note(d, link) {
        out = out.child(ui::colored(note, ORANGE, cx));
    }
    out.child(ui::weak(
        format!(
            "{} test file, deleted afterwards. 4K random tests run at queue depth 1 unless noted.",
            format_bytes(r.file_mb << 20)
        ),
        cx,
    ))
}

fn verdict_text(d: &Drive, v: Verdict) -> &'static str {
    match (v, d.bus, d.ssd) {
        (Verdict::Good, ..) => "Sequential read is in the normal range for this drive.",
        (_, BusType::Nvme, _) => {
            "Slower than this drive's link allows. A nearly full drive, heat throttling or \
             background disk activity can cause this."
        }
        (_, _, Some(false)) => {
            "Slower than a typical hard drive. A failing drive, a very full disk or background \
             disk activity can cause this."
        }
        _ => {
            "Slower than a SATA SSD usually reads. A nearly full or worn drive, the SATA cable \
             or port, or background disk activity can cause this."
        }
    }
}

/// A warning when the drive's link runs below what it supports.
fn link_note(d: &Drive, link: Option<&PcieLink>) -> Option<String> {
    if let Some(l) = link {
        let ((cg, cw), (mg, mw)) = (l.current, l.max);
        if cg > 0 && cw > 0 && (cg < mg || cw < mw) {
            return Some(format!(
                "The PCIe link runs at {cg}.0 x{cw} but the drive supports {mg}.0 x{mw}; a \
                 different M.2 slot may make it faster."
            ));
        }
    }
    let s = d.sata_link?;
    (s.current_gen < s.max_gen).then(|| {
        format!(
            "The SATA link runs at {} but the drive supports {}; try another SATA port or cable.",
            sata_gen_speed(s.current_gen),
            sata_gen_speed(s.max_gen)
        )
    })
}

#[cfg(test)]
mod tests {
    use super::mbs;
    use crate::app::Tab;
    use crate::settings::Settings;
    use crate::tabs::page_tests::{assert_shows, go, machines, open, page, snapshot, texts};
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{AppContext, TestAppContext};
    use omnihw_core::disktest;
    use omnihw_core::testkit::desktop;
    use std::time::SystemTime;

    #[test]
    fn speeds_keep_one_decimal_only_when_small() {
        assert_eq!(mbs(3.25), "3.2 MB/s");
        assert_eq!(mbs(512.4), "512 MB/s");
    }

    #[gpui_kit::test]
    fn draws_every_drive_on_both_machines(cx: &mut TestAppContext) {
        for info in machines() {
            let shown = page(cx, &info, snapshot(&info), Tab::Storage);
            assert_shows(
                &shown,
                &[
                    &info.drives[0].model,
                    "Device",
                    "Speed test",
                    "Run speed test",
                    "Volumes",
                ],
            );
        }
        let info = desktop();
        let shown = page(cx, &info, snapshot(&info), Tab::Storage);
        assert_shows(&shown, &["ST4000DM004-2U9104", "Health (NVMe SMART)"]);
    }

    #[gpui_kit::test]
    fn empty_snapshot_draws(cx: &mut TestAppContext) {
        let info = desktop();
        let shown = page(cx, &info, Default::default(), Tab::Storage);
        assert_shows(&shown, &[&info.drives[0].model, "Run speed test"]);
        let mut none = desktop();
        none.drives.clear();
        let shown = page(cx, &none, Default::default(), Tab::Storage);
        assert_shows(&shown, &["No physical drives found."]);
    }

    #[gpui_kit::test]
    fn running_test_and_results(cx: &mut TestAppContext) {
        let info = desktop();
        let index = info.drives[0].index;
        // A test in progress shows its phase on the progress bar.
        let mut snap = snapshot(&info);
        snap.disk_test.running = Some(index);
        let shown = page(cx, &info, snap, Tab::Storage);
        assert_shows(&shown, &["Testing...", "Preparing test file"]);

        // A failed test shows why.
        let mut snap = snapshot(&info);
        snap.disk_test
            .results
            .insert(index, (Err("No space left".into()), SystemTime::now()));
        let shown = page(cx, &info, snap, Tab::Storage);
        assert_shows(&shown, &["No space left", "Last run just now"]);
    }

    #[gpui_kit::test]
    fn speed_test_asks_first(cx: &mut TestAppContext) {
        let info = desktop();
        let d = &info.drives[0];
        assert!(disktest::pick_volume(d).is_some(), "the drive has room");
        let (window, app) = open(cx, info.clone(), snapshot(&info), Settings::default());
        go(cx, &app, Tab::Storage);
        assert!(!texts(cx, window).iter().any(|t| t == "Run test"));
        // Only opens the dialog; the test never runs because it is not confirmed. (The
        // dialog's title and text are not labels the tests can see, only its buttons.)
        cx.update_window(window, |_, window, cx| {
            window.click(("disk-test", d.index as usize), cx);
        })
        .unwrap();
        assert_shows(&texts(cx, window), &["Run test", "Cancel"]);
        assert_eq!(
            cx.read(|cx| app.read(cx).sampler.snapshot().disk_test.running),
            None
        );
    }
}
