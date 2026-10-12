//! Bottleneck view: was a recorded session limited by the GPU, the CPU, or neither?

use super::compare::{CompareState, pick_single};
use super::recording::{ChartArea, chart_area, draw_axes, series_points};
use crate::heat::{ORANGE, RED};
use crate::sampler::Snapshot;
use egui::{Align2, Color32, FontId, Rect, RichText, Sense, Stroke, Ui, pos2, vec2};
use omnihw_core::bottleneck::{self, Analysis, Limit, Verdict};
use omnihw_core::gpu::GpuAdapter;
use omnihw_core::recording::{Recording, format_elapsed};
use omnihw_core::sensors::Unit;

const GPU_COLOR: Color32 = Color32::from_rgb(38, 166, 154);
const CPU_COLOR: Color32 = Color32::from_rgb(66, 133, 244);
const NONE_COLOR: Color32 = Color32::from_rgb(140, 140, 140);
const IDLE_COLOR: Color32 = Color32::from_rgb(70, 70, 70);

fn limit_color(l: Limit) -> Color32 {
    match l {
        Limit::GpuBound => GPU_COLOR,
        Limit::CpuBound => CPU_COLOR,
        Limit::NotLimited => NONE_COLOR,
        Limit::Idle => IDLE_COLOR,
    }
}

fn verdict_color(v: Verdict) -> Color32 {
    match v {
        Verdict::GpuBound => GPU_COLOR,
        Verdict::CpuBound => CPU_COLOR,
        Verdict::Mixed => ORANGE,
        Verdict::NotLimited | Verdict::Idle => NONE_COLOR,
    }
}

pub fn show(ui: &mut Ui, snap: &Snapshot, st: &mut CompareState, gpus: &[GpuAdapter]) {
    let picked = pick_single(ui, snap, st);
    ui.add_space(4.0);
    let (rec, name) = match picked {
        Ok(Some(r)) => r,
        Ok(None) => {
            ui.weak("Record a gaming session (Record button), then analyze it here to see whether the CPU or the GPU limited your frame rate.");
            return;
        }
        Err(e) => {
            ui.colored_label(
                crate::heat::ink(ui.visuals(), RED),
                format!("Could not load {e}"),
            );
            return;
        }
    };
    // VRAM size for the "VRAM full" factor, matched by GPU name (CSV recordings don't store LUIDs).
    let vram_total_mb = |gpu_name: &str| {
        gpus.iter()
            .find(|g| g.name == gpu_name)
            .map(|g| g.dedicated_vram as f64 / 1_048_576.0)
    };
    let analysis = bottleneck::find_columns(rec)
        .and_then(|c| bottleneck::analyze(rec, vram_total_mb(&c.gpu_name)));
    let a = match analysis {
        Ok(a) => a,
        Err(e) => {
            ui.colored_label(crate::heat::ink(ui.visuals(), ORANGE), e);
            return;
        }
    };

    egui::ScrollArea::vertical()
        .id_salt("bottleneck")
        .auto_shrink(false)
        .show(ui, |ui| body(ui, rec, &name, &a));
}

fn body(ui: &mut Ui, rec: &Recording, name: &str, a: &Analysis) {
    ui.label(
        RichText::new(a.headline())
            .size(24.0)
            .strong()
            .color(verdict_color(a.verdict)),
    );
    ui.weak(format!(
        "{name}  ·  {}  ·  {} active samples, {} idle  ·  GPU: {}",
        format_elapsed(rec.duration_secs()),
        a.active,
        a.idle,
        a.columns.gpu_name
    ));
    ui.add_space(8.0);
    share_bar(ui, a);
    ui.add_space(10.0);

    ui.label(RichText::new("GPU load and busiest CPU thread").strong());
    load_chart(ui, rec, a);
    timeline_strip(ui, rec, a);
    legend(ui);
    ui.add_space(10.0);

    ui.label(RichText::new("What this means").strong());
    ui.add(egui::Label::new(a.explanation()).wrap());
    if !a.factors.is_empty() {
        ui.add_space(6.0);
        ui.label(RichText::new("Contributing factors").strong());
        for f in &a.factors {
            bullet(ui, f, Some(ORANGE));
        }
    }
    ui.add_space(6.0);
    ui.label(RichText::new("Tips").strong());
    for t in a.tips() {
        bullet(ui, &t, None);
    }
    if !a.caveats.is_empty() {
        ui.add_space(6.0);
        ui.label(RichText::new("Notes").strong());
        for c in &a.caveats {
            bullet(ui, c, None);
        }
    }
    ui.add_space(6.0);
    ui.weak(format!(
        "How it's judged: GPU-bound when GPU load is at least {:.0} %; CPU-bound when GPU load is under {:.0} % while one CPU thread is at least {:.0} % busy (or total CPU at least {:.0} %); otherwise not limited by either. Samples with GPU and CPU below {:.0} % count as idle.",
        bottleneck::GPU_BOUND,
        bottleneck::GPU_HEADROOM,
        bottleneck::THREAD_BUSY,
        bottleneck::CPU_TOTAL_BUSY,
        bottleneck::IDLE
    ));
}

fn bullet(ui: &mut Ui, text: &str, color: Option<Color32>) {
    ui.horizontal_wrapped(|ui| {
        ui.label("•");
        let t = RichText::new(text);
        ui.add(
            egui::Label::new(match color {
                Some(c) => t.color(c),
                None => t,
            })
            .wrap(),
        );
    });
}

/// Stacked bar of GPU-bound / CPU-bound / not-limited shares of active time.
fn share_bar(ui: &mut Ui, a: &Analysis) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 4, ui.visuals().extreme_bg_color);
    let (g, c, n) = a.percentages();
    let mut x = rect.left();
    for (pct, color, label) in [
        (g, GPU_COLOR, "GPU-bound"),
        (c, CPU_COLOR, "CPU-bound"),
        (n, NONE_COLOR, "Neither"),
    ] {
        let w = rect.width() * (pct / 100.0) as f32;
        if w <= 0.0 {
            continue;
        }
        let seg = Rect::from_min_max(pos2(x, rect.top()), pos2(x + w, rect.bottom()));
        p.rect_filled(seg, 0, color);
        let text = format!("{label} {pct:.0} %");
        if w > 90.0 {
            p.text(
                seg.center(),
                Align2::CENTER_CENTER,
                text,
                FontId::proportional(12.0),
                crate::heat::text_on(color),
            );
        }
        x += w;
    }
}

fn load_chart(ui: &mut Ui, rec: &Recording, a: &Analysis) {
    let area = chart_area(ui);
    let dur = rec.duration_secs().max(1e-6);
    draw_axes(&area, 0.0, 100.0, dur, Unit::Percent);
    let ChartArea {
        plot,
        painter: p,
        resp,
        weak,
    } = area;
    let x_of = |t: f64| plot.left() + plot.width() * (t / dur) as f32;
    let y_of = |v: f64| plot.bottom() - plot.height() * (v / 100.0) as f32;
    let buckets = plot.width() as usize;
    let gpu = &rec.values[a.columns.gpu_load];
    for (vals, color) in [(gpu, GPU_COLOR), (&a.busiest_thread, CPU_COLOR)] {
        let pts = series_points(&rec.times, vals, buckets, x_of, y_of);
        if pts.len() >= 2 {
            p.line(pts, Stroke::new(1.6, color));
        }
    }
    if let Some(pos) = resp.hover_pos().filter(|p| plot.x_range().contains(p.x)) {
        let t = ((pos.x - plot.left()) / plot.width()) as f64 * dur;
        let i = rec.times.partition_point(|&x| x < t).min(rec.len() - 1);
        p.line_segment(
            [pos2(pos.x, plot.top()), pos2(pos.x, plot.bottom())],
            Stroke::new(1.0, weak),
        );
        let fmt = |v: f32| {
            if v.is_finite() {
                format!("{v:.0} %")
            } else {
                "-".into()
            }
        };
        resp.on_hover_ui_at_pointer(|ui| {
            ui.strong(format_elapsed(rec.times[i]));
            ui.colored_label(
                crate::heat::ink(ui.visuals(), GPU_COLOR),
                format!("GPU load: {}", fmt(gpu[i])),
            );
            ui.colored_label(
                crate::heat::ink(ui.visuals(), CPU_COLOR),
                format!("Busiest CPU thread: {}", fmt(a.busiest_thread[i])),
            );
            if let Some(Some(l)) = a.timeline.get(i) {
                ui.label(match l {
                    Limit::GpuBound => "GPU-bound",
                    Limit::CpuBound => "CPU-bound",
                    Limit::NotLimited => "Not limited by CPU or GPU",
                    Limit::Idle => "Idle",
                });
            }
        });
    }
}

/// Per-sample class as a colored strip, aligned with the chart's plot area above.
fn timeline_strip(ui: &mut Ui, rec: &Recording, a: &Analysis) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 16.0), Sense::hover());
    // Same horizontal insets as `chart_area`'s plot rect.
    let strip = Rect::from_min_max(
        pos2(rect.left() + 64.0, rect.top() + 2.0),
        pos2(rect.right() - 8.0, rect.bottom() - 2.0),
    );
    let p = ui.painter_at(rect);
    p.rect_filled(strip, 2, ui.visuals().extreme_bg_color);
    p.text(
        pos2(strip.left() - 6.0, strip.center().y),
        Align2::RIGHT_CENTER,
        "Limit",
        FontId::proportional(10.0),
        ui.visuals().weak_text_color(),
    );
    let dur = rec.duration_secs().max(1e-6);
    let x_of = |t: f64| strip.left() + strip.width() * (t / dur) as f32;
    let n = rec.len();
    let mut i = 0;
    // Merge runs of the same class into one rectangle.
    while i < n {
        let class = a.timeline[i];
        let mut j = i + 1;
        while j < n && a.timeline[j] == class {
            j += 1;
        }
        if let Some(l) = class {
            let x0 = x_of(rec.times[i]);
            let x1 = if j < n {
                x_of(rec.times[j])
            } else {
                strip.right()
            };
            p.rect_filled(
                Rect::from_min_max(
                    pos2(x0, strip.top()),
                    pos2(x1.max(x0 + 1.0), strip.bottom()),
                ),
                0,
                limit_color(l),
            );
        }
        i = j;
    }
}

fn legend(ui: &mut Ui) {
    ui.horizontal_wrapped(|ui| {
        ui.add_space(64.0);
        for (color, label) in [
            (GPU_COLOR, "GPU-bound / GPU load"),
            (CPU_COLOR, "CPU-bound / busiest thread"),
            (NONE_COLOR, "Neither (frame cap, V-Sync)"),
            (IDLE_COLOR, "Idle"),
        ] {
            let (r, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
            ui.painter().rect_filled(r, 2, color);
            ui.label(RichText::new(label).small());
            ui.add_space(8.0);
        }
    });
}
