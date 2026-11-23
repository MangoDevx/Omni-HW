//! Bottleneck view: was a recorded session limited by the GPU, the CPU, or neither?

use super::compare::{CompareState, pick_single, single_picker};
use super::recording::{
    BOTTLENECK_CHARTS, Chart, ChartHover, Line, PLOT_LEFT, PLOT_RIGHT, Snap, axis_labels,
    chart_element, series_points,
};
use crate::app::OmniApp;
use crate::heat::{self, ORANGE, RED, rgb8};
use crate::sampler::Snapshot;
use crate::ui;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{ActiveTheme, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AnyElement, Bounds, Context, Div, ElementId, InteractiveElement, IntoElement, ParentElement,
    Rgba, Styled, canvas, div, fill, point, px, relative, rems, size,
};
use omnihw_core::bottleneck::{self, Analysis, Limit, Verdict};
use omnihw_core::gpu::GpuAdapter;
use omnihw_core::recording::{Recording, format_elapsed};
use omnihw_core::sensors::Unit;
use std::rc::Rc;

const GPU_COLOR: Rgba = rgb8(38, 166, 154);
const CPU_COLOR: Rgba = heat::BLUE;
const NONE_COLOR: Rgba = rgb8(140, 140, 140);
const IDLE_COLOR: Rgba = rgb8(70, 70, 70);

fn limit_color(l: Limit) -> Rgba {
    match l {
        Limit::GpuBound => GPU_COLOR,
        Limit::CpuBound => CPU_COLOR,
        Limit::NotLimited => NONE_COLOR,
        Limit::Idle => IDLE_COLOR,
    }
}

fn verdict_color(v: Verdict) -> Rgba {
    match v {
        Verdict::GpuBound => GPU_COLOR,
        Verdict::CpuBound => CPU_COLOR,
        Verdict::Mixed => ORANGE,
        Verdict::NotLimited | Verdict::Idle => NONE_COLOR,
    }
}

/// The analysis of the recording last shown, so it's redone only when the recording changes
/// (another pick, or a new sample in the live one) instead of on every redraw.
#[derive(Default)]
pub struct AnalysisCache {
    /// (recording name, start time bits, sample count) the result is for.
    key: Option<(String, u64, usize)>,
    result: Option<Result<Analysis, String>>,
}

impl AnalysisCache {
    fn get(
        &mut self,
        rec: &Recording,
        name: &str,
        gpus: &[GpuAdapter],
    ) -> &Result<Analysis, String> {
        let key = (name.to_string(), rec.started_unix.to_bits(), rec.len());
        if self.key.as_ref() != Some(&key) {
            self.key = Some(key);
            self.result = None;
        }
        self.result.get_or_insert_with(|| {
            // VRAM size for the "VRAM full" factor, matched by GPU name (CSV recordings don't
            // store LUIDs).
            let vram_total_mb = |gpu_name: &str| {
                gpus.iter()
                    .find(|g| g.name == gpu_name)
                    .map(|g| g.dedicated_vram as f64 / 1_048_576.0)
            };
            bottleneck::find_columns(rec)
                .and_then(|c| bottleneck::analyze(rec, vram_total_mb(&c.gpu_name)))
        })
    }
}

/// The view: the recording picker, then the analysis of the picked recording. The recording
/// to analyze defaults to the in-memory one, else the newest saved one (see
/// [`CompareState::default_single`]).
pub(super) fn show(
    snap: &Snapshot,
    st: &mut CompareState,
    cache: &mut AnalysisCache,
    charts: &ChartHover,
    gpus: &[GpuAdapter],
    buckets: usize,
    cx: &mut Context<OmniApp>,
) -> AnyElement {
    let page = v_flex()
        .flex_1()
        .min_h_0()
        .w_full()
        .gap_1()
        .child(single_picker(snap, st, cx))
        .child(ui::space(4.0));
    let (rec, name) = match pick_single(snap, st) {
        Ok(Some(r)) => r,
        Ok(None) => {
            return page
                .child(ui::weak("Record a gaming session (Record button), then analyze it here to see whether the CPU or the GPU limited your frame rate.", cx))
                .into_any_element();
        }
        Err(e) => {
            return page
                .child(ui::colored(format!("Could not load {e}"), RED, cx))
                .into_any_element();
        }
    };
    let a = match cache.get(rec, &name, gpus) {
        Ok(a) => a,
        Err(e) => {
            return page
                .child(ui::colored(e.clone(), ORANGE, cx))
                .into_any_element();
        }
    };
    page.child(
        body(rec, &name, a, charts, buckets, cx)
            .id("bottleneck")
            .flex_1()
            .min_h_0()
            .overflow_y_scrollbar(),
    )
    .into_any_element()
}

fn body(
    rec: &Recording,
    name: &str,
    a: &Analysis,
    charts: &ChartHover,
    buckets: usize,
    cx: &mut Context<OmniApp>,
) -> Div {
    let mut col = ui::column()
        .child(
            ui::label(a.headline())
                .text_2xl()
                .text_color(verdict_color(a.verdict)),
        )
        .child(ui::weak(
            format!(
                "{name}  ·  {}  ·  {} active samples, {} idle  ·  GPU: {}",
                format_elapsed(rec.duration_secs()),
                a.active,
                a.idle,
                a.columns.gpu_name
            ),
            cx,
        ))
        .child(ui::space(8.0))
        .child(share_bar(a, cx))
        .child(ui::space(10.0))
        .child(ui::strong("GPU load and busiest CPU thread"))
        .child(load_chart(rec, a, charts, buckets, cx))
        .child(timeline_strip(rec, a, cx))
        .child(legend())
        .child(ui::space(10.0))
        .child(ui::strong("What this means"))
        .child(ui::label(a.explanation()));
    if !a.factors.is_empty() {
        col = col
            .child(ui::space(6.0))
            .child(ui::strong("Contributing factors"));
        for (i, f) in a.factors.iter().enumerate() {
            col = col.child(bullet(("factor", i), f, Some(ORANGE)));
        }
    }
    col = col.child(ui::space(6.0)).child(ui::strong("Tips"));
    for (i, t) in a.tips().iter().enumerate() {
        col = col.child(bullet(("tip", i), t, None));
    }
    if !a.caveats.is_empty() {
        col = col.child(ui::space(6.0)).child(ui::strong("Notes"));
        for (i, c) in a.caveats.iter().enumerate() {
            col = col.child(bullet(("note", i), c, None));
        }
    }
    col.child(ui::space(6.0)).child(ui::weak(
        format!(
            "How it's judged: GPU-bound when GPU load is at least {:.0} %; CPU-bound when GPU load is under {:.0} % while one CPU thread is at least {:.0} % busy (or total CPU at least {:.0} %); otherwise not limited by either. Samples with GPU and CPU below {:.0} % count as idle.",
            bottleneck::GPU_BOUND,
            bottleneck::GPU_HEADROOM,
            bottleneck::THREAD_BUSY,
            bottleneck::CPU_TOTAL_BUSY,
            bottleneck::IDLE
        ),
        cx,
    ))
}

/// A bulleted line that wraps under its own text. `id` must be unique on the page.
fn bullet(id: impl Into<ElementId>, text: &str, color: Option<Rgba>) -> impl IntoElement {
    let text = ui::label(text.to_string()).flex_1().min_w_0();
    h_flex()
        .id(id)
        .items_start()
        .gap_1()
        .child(ui::label("•"))
        .child(match color {
            Some(c) => text.text_color(c),
            None => text,
        })
}

/// Stacked bar of GPU-bound / CPU-bound / not-limited shares of active time. A segment shows
/// its name and share when it is wide enough to hold them.
fn share_bar(a: &Analysis, cx: &mut Context<OmniApp>) -> Div {
    let (g, c, n) = a.percentages();
    let segments = [
        (g, GPU_COLOR, "GPU-bound"),
        (c, CPU_COLOR, "CPU-bound"),
        (n, NONE_COLOR, "Neither"),
    ]
    .into_iter()
    .filter(|(pct, _, _)| *pct > 0.0)
    .map(|(pct, color, label)| {
        h_flex()
            .flex_none()
            .h_full()
            .w(relative((pct / 100.0) as f32))
            .justify_center()
            .overflow_hidden()
            .bg(color)
            .text_size(crate::ui::small_size())
            .when(pct >= 12.0, |s| {
                s.child(
                    ui::label(format!("{label} {pct:.0} %"))
                        .whitespace_nowrap()
                        .text_color(heat::text_on(color)),
                )
            })
    });
    h_flex()
        .w_full()
        .h(rems(1.5))
        .rounded(px(4.0))
        .overflow_hidden()
        .bg(cx.theme().muted)
        .children(segments)
}

fn load_chart(
    rec: &Recording,
    a: &Analysis,
    charts: &ChartHover,
    buckets: usize,
    cx: &mut Context<OmniApp>,
) -> AnyElement {
    let key = BOTTLENECK_CHARTS;
    let dur = rec.duration_secs().max(1e-6);
    let (y_labels, x_labels) = axis_labels(0.0, 100.0, dur, Unit::Percent);
    let x_of = |t: f64| (t / dur) as f32;
    let y_of = |v: f64| (v / 100.0) as f32;
    let gpu = &rec.values[a.columns.gpu_load];
    let lines = [(gpu, GPU_COLOR), (&a.busiest_thread, CPU_COLOR)]
        .into_iter()
        .map(|(vals, color)| Line {
            pts: series_points(&rec.times, vals, buckets, x_of, y_of),
            color,
            width: 1.6,
            dashed: false,
        })
        .collect();
    let mut cursor = None;
    let mut readout = Vec::new();
    if let Some(h) = charts.hovered(key)
        && !rec.is_empty()
    {
        let i = h.at.min(rec.len() - 1);
        cursor = Some(h.frac);
        let fmt = |v: f32| {
            if v.is_finite() {
                format!("{v:.0} %")
            } else {
                "-".into()
            }
        };
        readout.push((format_elapsed(rec.times[i]), None));
        readout.push((format!("GPU load: {}", fmt(gpu[i])), Some(GPU_COLOR)));
        readout.push((
            format!("Busiest CPU thread: {}", fmt(a.busiest_thread[i])),
            Some(CPU_COLOR),
        ));
        if let Some(Some(l)) = a.timeline.get(i) {
            let text = match l {
                Limit::GpuBound => "GPU-bound",
                Limit::CpuBound => "CPU-bound",
                Limit::NotLimited => "Not limited by CPU or GPU",
                Limit::Idle => "Idle",
            };
            readout.push((text.to_string(), Some(cx.theme().foreground.into())));
        }
    }
    chart_element(
        Chart {
            key,
            lines,
            y_labels,
            x_labels,
            snap: Snap::Samples {
                times: Rc::new(rec.times.clone()),
                dur,
            },
            cursor,
            readout,
        },
        charts,
        cx,
    )
}

/// Per-sample class as a colored strip, aligned with the chart's plot area above.
fn timeline_strip(rec: &Recording, a: &Analysis, cx: &mut Context<OmniApp>) -> Div {
    let dur = rec.duration_secs().max(1e-6);
    let x_of = |t: f64| (t / dur) as f32;
    let n = rec.len();
    // Merge runs of the same class into one rectangle: (start, end, colour) as fractions of
    // the strip width.
    let mut runs: Vec<(f32, f32, Rgba)> = Vec::new();
    let mut i = 0;
    while i < n {
        let class = a.timeline[i];
        let mut j = i + 1;
        while j < n && a.timeline[j] == class {
            j += 1;
        }
        if let Some(l) = class {
            let x0 = x_of(rec.times[i]);
            let x1 = if j < n { x_of(rec.times[j]) } else { 1.0 };
            runs.push((x0, x1, limit_color(l)));
        }
        i = j;
    }
    let bg = cx.theme().muted;
    let strip = canvas(
        |_, _, _| {},
        move |b, (), window, _| {
            window.paint_quad(fill(b, bg));
            let w = b.size.width;
            for (x0, x1, color) in runs {
                let left = b.left() + w * x0;
                let right = (b.left() + w * x1).max(left + px(1.0));
                window.paint_quad(fill(
                    Bounds {
                        origin: point(left, b.top()),
                        size: size(right - left, b.size.height),
                    },
                    color,
                ));
            }
        },
    )
    .absolute()
    .left(rems(PLOT_LEFT))
    .right(rems(PLOT_RIGHT))
    .top(px(2.0))
    .bottom(px(2.0));
    div()
        .relative()
        .flex_none()
        .w_full()
        .h(rems(1.0))
        .child(
            h_flex()
                .absolute()
                .left_0()
                .top_0()
                .bottom_0()
                .w(rems(PLOT_LEFT - 0.375))
                .justify_end()
                .child(ui::small_weak("Limit", cx)),
        )
        .child(strip)
}

fn legend() -> Div {
    ui::wrap_row().pl(rems(PLOT_LEFT)).children(
        [
            (GPU_COLOR, "GPU-bound / GPU load"),
            (CPU_COLOR, "CPU-bound / busiest thread"),
            (NONE_COLOR, "Neither (frame cap, V-Sync)"),
            (IDLE_COLOR, "Idle"),
        ]
        .into_iter()
        .map(|(color, label)| {
            h_flex()
                .gap_1()
                .mr_2()
                .child(
                    div()
                        .flex_none()
                        .size(rems(0.625))
                        .rounded(px(2.0))
                        .bg(color),
                )
                .child(ui::label(label).text_size(crate::ui::small_size()))
        }),
    )
}
