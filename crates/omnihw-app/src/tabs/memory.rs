//! Memory page: totals and load, the quick memory test and the installed modules (SMBIOS).

use crate::app::OmniApp;
use crate::heat::{GREEN, ORANGE, RED};
use crate::sampler::Snapshot;
use crate::ui::{self, Table, or_dash, section};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::Button;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::{
    AnyElement, App, Context, InteractiveElement, IntoElement, ParentElement, Rgba, Styled, Window,
    div,
};
use omnihw_core::memtest::{MemTestResult, Verdict};
use std::time::Duration;

pub fn show(app: &mut OmniApp, _window: &mut Window, cx: &mut Context<OmniApp>) -> AnyElement {
    let Some(info) = app.sampler.info() else {
        return ui::column().into_any_element();
    };
    let snap = app.sampler.snapshot();
    let dimms: Vec<_> = info
        .smbios
        .iter()
        .flat_map(|s| s.populated_memory())
        .collect();
    let first = dimms.first();
    let usable_mb = info.memory.map(|m| m.total_bytes / (1024 * 1024));
    let installed_mb: u64 = dimms.iter().map(|d| d.size_mb).sum();
    let modules = match &info.smbios {
        Some(s) if s.memory_slots > 0 => format!("{} of {} slots", dimms.len(), s.memory_slots),
        _ => dimms.len().to_string(),
    };

    let mut rows = vec![
        (
            "Type",
            first.map(|d| d.memory_type.to_string()).unwrap_or_default(),
        ),
        (
            "Installed",
            if installed_mb > 0 {
                format!("{} GB", installed_mb / 1024)
            } else {
                String::new()
            },
        ),
        (
            "Usable",
            usable_mb
                .map(|m| format!("{:.1} GB", m as f64 / 1024.0))
                .unwrap_or_default(),
        ),
        ("Modules", modules),
    ];
    if let Some(d) = first {
        rows.push((
            "Speed (configured)",
            format!("{} MT/s", d.configured_speed_mts),
        ));
    }
    if let Some(t) = snap.table.get("mem/load") {
        rows.push(("Load", format!("{:.0} %", t.reading.value)));
    }
    let mut page = ui::column()
        .child(section("General", &rows, cx).id("general"))
        .child(quick_test(&snap, cx));

    if dimms.is_empty() {
        return page.into_any_element();
    }
    let mut table = Table::new(&[
        "Slot",
        "Size",
        "Type",
        "Rated",
        "Configured",
        "Ranks",
        "Manufacturer",
        "Part Number",
    ])
    .striped();
    for d in &dimms {
        table = table.text_row([
            d.locator.clone(),
            format!("{} GB", d.size_mb / 1024),
            format!("{} {}", d.memory_type, d.form_factor),
            format!("{} MT/s", d.speed_mts),
            format!("{} MT/s", d.configured_speed_mts),
            if d.rank > 0 {
                d.rank.to_string()
            } else {
                "-".into()
            },
            or_dash(&d.manufacturer).to_string(),
            or_dash(&d.part_number).to_string(),
        ]);
    }
    page = page
        .child(ui::strong("Modules (SMBIOS)"))
        .child(table.build(cx));
    page.into_any_element()
}

/// The "Quick test" group: run button, live phase while it runs, the last result and any
/// hardware errors logged during it.
fn quick_test(snap: &Snapshot, cx: &mut Context<OmniApp>) -> impl IntoElement {
    let state = &snap.mem_test;
    let mut controls = ui::row();
    if state.running {
        let (phase, step, total) = state.progress.read();
        controls = controls
            .child(Spinner::new().small())
            .child(ui::label(format!(
                "Testing… {} ({step}/{total})",
                phase.name()
            )));
    } else {
        controls = controls.child(
            Button::new("run-mem-test")
                .label("Run memory test")
                .tooltip("Takes a few seconds; close heavy apps first for a fair result")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.sampler.run_mem_test();
                    cx.notify();
                    redraw_while(cx, |app| app.sampler.snapshot().mem_test.running);
                })),
        );
    }
    if let Some((_, at)) = &state.result {
        controls = controls.child(ui::weak(format!("Last run {}", ui::ago(*at)), cx));
    }

    let mut group = ui::group_box(cx)
        .id("quick-test")
        .child(ui::strong("Quick test"))
        .child(ui::weak(
            "Measures read, write and copy bandwidth and latency in a few seconds. \
             It uses most CPU cores briefly; nothing is changed.",
            cx,
        ))
        .child(ui::space(4.0))
        .child(controls);
    match &state.result {
        Some((Ok(r), _)) => group = group.child(results(r, cx)),
        Some((Err(e), _)) => group = group.child(ui::colored(e.clone(), RED, cx)),
        None => {}
    }
    group.children(ui::whea_warning("mem-whea", &state.whea_during, cx))
}

fn results(r: &MemTestResult, cx: &App) -> impl IntoElement {
    let mut rows = vec![
        ("Read", format!("{:.1} GB/s", r.read_gbs)),
        ("Read (1 core)", format!("{:.1} GB/s", r.read_single_gbs)),
        ("Write", format!("{:.1} GB/s", r.write_gbs)),
        ("Copy", format!("{:.1} GB/s", r.copy_gbs)),
        ("Latency", format!("{:.1} ns", r.latency_ns)),
    ];
    if let Some(e) = &r.expected {
        rows.push((
            "Expected",
            format!(
                "~{:.1} GB/s theoretical ({}-{}, {} channel{}{})",
                e.theoretical_gbs,
                e.memory_type,
                e.mts,
                e.channels,
                if e.channels == 1 { "" } else { "s" },
                if e.channels_estimated {
                    ", estimated"
                } else {
                    ""
                },
            ),
        ));
    }
    if let Some(p) = r.efficiency_pct() {
        rows.push(("Read efficiency", format!("{p:.0} %")));
    }
    let single_on = match r.read_cpu {
        Some(id) => format!("CPU {}", id.number),
        None => "one thread".to_string(),
    };
    let mut out = ui::column()
        .pt_1()
        .child(ui::key_values(&rows))
        .child(ui::weak(
            format!(
                "{} threads ({single_on} for the 1-core read), {} MB buffer. One core can't use \
                 every channel, so its read is lower. Copy counts bytes read plus bytes written.",
                r.threads, r.buffer_mb
            ),
            cx,
        ));
    let Some(e) = &r.expected else {
        return out.child(ui::weak(
            "No memory speed in SMBIOS, so there's nothing to compare with.",
            cx,
        ));
    };
    let below_rated = e.rated_mts > e.mts;
    if let Some(v) = r.bandwidth_verdict() {
        let text = match v {
            Verdict::Good => "About what this memory should deliver.".to_string(),
            _ if below_rated => format!(
                "Lower than expected, and the memory runs at {} MT/s although it's rated for {}. \
                 Enable XMP/EXPO in the BIOS, and check that the DIMMs are in the slots the \
                 manual recommends for dual channel.",
                e.mts, e.rated_mts
            ),
            _ => "Lower than expected for this memory. Check that the DIMMs are in the slots \
                the manual recommends for dual channel and that XMP/EXPO is enabled. \
                Background load also lowers the result."
                .to_string(),
        };
        out = out.child(verdict_line("Bandwidth", v, text, cx));
    }
    if let Some(v) = r.latency_verdict() {
        let (lo, hi) = e.latency_typical_ns;
        let text = match v {
            Verdict::Good => format!(
                "Within the typical {lo:.0}-{hi:.0} ns for {}.",
                e.memory_type
            ),
            _ => format!(
                "Slower than the typical {lo:.0}-{hi:.0} ns for {}. Loose timings, low memory \
                 speed or background load raise latency.",
                e.memory_type
            ),
        };
        out = out.child(verdict_line("Latency", v, text, cx));
    }
    out
}

fn verdict_line(what: &'static str, v: Verdict, text: String, cx: &App) -> impl IntoElement {
    // An id per line: both lines can show the same pill text.
    ui::wrap_row()
        .id(what)
        .child(ui::label(format!("{what}:")))
        .child(ui::pill(v.name(), verdict_color(v), true, cx))
        .child(div().flex_1().min_w_0().child(ui::label(text)))
}

fn verdict_color(v: Verdict) -> Rgba {
    match v {
        Verdict::Good => GREEN,
        Verdict::Low => ORANGE,
        Verdict::VeryLow => RED,
    }
}

/// Redraw every 200 ms for as long as `running` says a test started by a button is going, so
/// its progress stays current (the sampler only wakes the UI once per sample). Started from
/// the button's handler, never from `show`; it ends with one last redraw when the test is done.
pub(super) fn redraw_while(cx: &mut Context<OmniApp>, running: fn(&OmniApp) -> bool) {
    cx.spawn(async move |this, cx| {
        loop {
            cx.background_executor()
                .timer(Duration::from_millis(200))
                .await;
            let more = this.update(cx, |this, cx| {
                cx.notify();
                running(this)
            });
            if !matches!(more, Ok(true)) {
                break;
            }
        }
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use crate::app::Tab;
    use crate::tabs::page_tests::{assert_shows, machines, page, snapshot};
    use gpui_kit::TestAppContext;
    use omnihw_core::memtest::MemTestResult;
    use std::time::SystemTime;

    #[gpui_kit::test]
    fn draws_for_both_machines(cx: &mut TestAppContext) {
        for info in machines() {
            let part = if info.battery.is_some() {
                "MT62F1G32D2DS-026"
            } else {
                "KF560C30-16"
            };
            let shown = page(cx, &info, snapshot(&info), Tab::Memory);
            assert_shows(
                &shown,
                &[
                    "General",
                    "Installed",
                    "Quick test",
                    "Run memory test",
                    "Modules (SMBIOS)",
                    "Part Number",
                    part,
                ],
            );
        }
    }

    #[gpui_kit::test]
    fn empty_snapshot_draws(cx: &mut TestAppContext) {
        let info = &machines()[0];
        let shown = page(cx, info, Default::default(), Tab::Memory);
        assert_shows(&shown, &["General", "Run memory test"]);
    }

    #[gpui_kit::test]
    fn running_test_and_results(cx: &mut TestAppContext) {
        let info = &machines()[0];
        // A test in progress shows its phase instead of the button.
        let mut snap = snapshot(info);
        snap.mem_test.running = true;
        let shown = page(cx, info, snap, Tab::Memory);
        assert_shows(&shown, &["Testing… preparing"]);
        assert!(!shown.iter().any(|t| t == "Run memory test"));

        // A finished test shows its numbers.
        let mut snap = snapshot(info);
        let result = MemTestResult {
            read_gbs: 61.2,
            read_single_gbs: 28.4,
            write_gbs: 55.0,
            copy_gbs: 58.1,
            latency_ns: 72.5,
            threads: 16,
            read_cpu: None,
            buffer_mb: 1024,
            expected: None,
        };
        snap.mem_test.result = Some((Ok(result), SystemTime::now()));
        let shown = page(cx, info, snap, Tab::Memory);
        assert_shows(
            &shown,
            &[
                "61.2 GB/s",
                "72.5 ns",
                "Last run just now",
                "one thread for the 1-core read",
                "nothing to compare with",
            ],
        );

        // A failed test shows why.
        let mut snap = snapshot(info);
        snap.mem_test.result = Some((Err("Not enough free memory".into()), SystemTime::now()));
        let shown = page(cx, info, snap, Tab::Memory);
        assert_shows(&shown, &["Not enough free memory"]);
    }
}
