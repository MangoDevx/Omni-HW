//! Recording compare: overlay two sessions (e.g. before/after a repaste, driver update or
//! XMP change) and show per-sensor deltas. Sensors are matched by group + label
//! ([`Column::key`]), since saved CSVs don't store sensor ids.

use super::recording::{
    ChartArea, PALETTE, chart_area, draw_axes, sensor_picker, series_points, y_range,
};
use crate::export;
use crate::heat::{GREEN, RED};
use crate::sampler::Snapshot;
use egui::{RichText, Stroke, Ui, pos2, vec2};
use omnihw_core::recording::{Column, Recording, format_elapsed};
use omnihw_core::sensors::Unit;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

/// Sensors pre-selected when a comparison is first shown (matched by key suffix).
const DEFAULT_KEYS: [&str; 5] = [
    "CPU Usage / Total CPU Usage",
    "CPU Clocks / Average Core Clock",
    "Power / CPU Package Power",
    "Memory / Physical Memory Load",
    "/ GPU Temperature",
];

#[derive(Clone, PartialEq, Eq)]
enum Source {
    /// The current or last in-memory recording.
    Memory,
    File(PathBuf),
}

struct FileEntry {
    path: PathBuf,
    modified: SystemTime,
}

#[derive(Default)]
pub struct CompareState {
    files: Vec<FileEntry>,
    scanned: bool,
    cache: HashMap<PathBuf, Result<Arc<Recording>, String>>,
    a: Option<Source>,
    b: Option<Source>,
    /// Recording chosen for single-recording views (bottleneck analysis).
    single: Option<Source>,
    selected: BTreeSet<String>,
    filter: String,
    initialized: bool,
}

impl CompareState {
    /// List saved recordings in the report folder, newest first.
    fn scan(&mut self) {
        let mut files: Vec<FileEntry> = std::fs::read_dir(export::report_dir())
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| {
                let n = e.file_name().to_string_lossy().to_string();
                n.starts_with("OmniHW-Recording-") && n.ends_with(".csv")
            })
            .map(|e| FileEntry {
                modified: e
                    .metadata()
                    .and_then(|m| m.modified())
                    .unwrap_or(SystemTime::UNIX_EPOCH),
                path: e.path(),
            })
            .collect();
        files.sort_by_key(|f| std::cmp::Reverse(f.modified));
        self.files = files;
        self.scanned = true;
    }

    /// Parse a CSV once and keep it.
    fn load(&mut self, path: &Path) {
        if !self.cache.contains_key(path) {
            let parsed = std::fs::read_to_string(path)
                .map_err(|e| e.to_string())
                .and_then(|t| Recording::from_csv(&t))
                .map(Arc::new);
            self.cache.insert(path.to_path_buf(), parsed);
        }
    }
}

/// "2026-10-12 01:27:33 UTC" from "OmniHW-Recording-20261012-012733.csv".
fn file_label(path: &Path) -> String {
    let name = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let stamp = name.trim_start_matches("OmniHW-Recording-");
    let b = stamp.as_bytes();
    if b.len() == 15
        && b[8] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 8 || c.is_ascii_digit())
    {
        let s = stamp;
        format!(
            "{}-{}-{} {}:{}:{} UTC",
            &s[0..4],
            &s[4..6],
            &s[6..8],
            &s[9..11],
            &s[11..13],
            &s[13..15]
        )
    } else {
        name
    }
}

fn source_label(
    src: &Source,
    snap: &Snapshot,
    cache: &HashMap<PathBuf, Result<Arc<Recording>, String>>,
) -> String {
    match src {
        Source::Memory => match snap.viewed_recording() {
            Some((rec, _, true)) => format!(
                "Current recording ({})",
                format_elapsed(rec.duration_secs())
            ),
            Some((rec, _, false)) => {
                format!("Last recording ({})", format_elapsed(rec.duration_secs()))
            }
            None => "No recording in memory".into(),
        },
        Source::File(p) => match cache.get(p) {
            Some(Ok(rec)) => format!(
                "{} ({})",
                file_label(p),
                format_elapsed(rec.duration_secs())
            ),
            _ => file_label(p),
        },
    }
}

fn source_picker(
    ui: &mut Ui,
    salt: &str,
    slot: &mut Option<Source>,
    snap: &Snapshot,
    st_files: &[FileEntry],
    cache: &HashMap<PathBuf, Result<Arc<Recording>, String>>,
) {
    let text = slot
        .as_ref()
        .map_or("Choose a recording…".to_string(), |s| {
            source_label(s, snap, cache)
        });
    egui::ComboBox::from_id_salt(salt)
        .width(300.0)
        .height(400.0)
        .selected_text(text)
        .show_ui(ui, |ui| {
            if snap.viewed_recording().is_some() {
                let label = source_label(&Source::Memory, snap, cache);
                ui.selectable_value(slot, Some(Source::Memory), label);
            }
            for f in st_files {
                let src = Source::File(f.path.clone());
                let label = source_label(&src, snap, cache);
                ui.selectable_value(slot, Some(src), label);
            }
        });
}

/// Pick one recording (the in-memory one or a saved CSV) for analysis views.
/// Defaults to the in-memory recording, else the newest saved one.
pub(super) fn pick_single<'a>(
    ui: &mut Ui,
    snap: &'a Snapshot,
    st: &'a mut CompareState,
) -> Result<Option<(&'a Recording, String)>, String> {
    if !st.scanned {
        st.scan();
    }
    if st.single.is_none() {
        st.single = if snap.viewed_recording().is_some() {
            Some(Source::Memory)
        } else {
            st.files.first().map(|f| Source::File(f.path.clone()))
        };
    }
    let mut refresh = false;
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new("Recording").strong());
        source_picker(ui, "single_src", &mut st.single, snap, &st.files, &st.cache);
        refresh = ui
            .button("Refresh list")
            .on_hover_text("Rescan Documents for saved recordings")
            .clicked();
    });
    if refresh {
        st.scan();
    }
    if let Some(Source::File(p)) = st.single.clone() {
        st.load(&p);
    }
    let st: &'a CompareState = st;
    let Some(src) = &st.single else {
        return Ok(None);
    };
    let rec = match src {
        Source::Memory => snap.viewed_recording().map(|(r, _, _)| r),
        Source::File(p) => match st.cache.get(p) {
            Some(Ok(r)) => Some(r.as_ref()),
            Some(Err(e)) => return Err(format!("{}: {e}", file_label(p))),
            None => None,
        },
    };
    Ok(rec.map(|r| (r, source_label(src, snap, &st.cache))))
}

pub fn show(ui: &mut Ui, snap: &Snapshot, st: &mut CompareState) {
    if !st.scanned {
        st.scan();
    }
    let mut refresh = false;
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new("A").strong());
        source_picker(ui, "cmp_a", &mut st.a, snap, &st.files, &st.cache);
        ui.label(RichText::new("B").strong());
        source_picker(ui, "cmp_b", &mut st.b, snap, &st.files, &st.cache);
        refresh = ui
            .button("Refresh list")
            .on_hover_text("Rescan Documents for saved recordings")
            .clicked();
    });
    if refresh {
        st.scan();
    }
    for src in [st.a.clone(), st.b.clone()].into_iter().flatten() {
        if let Source::File(p) = src {
            st.load(&p);
        }
    }

    let CompareState {
        cache,
        a,
        b,
        selected,
        filter,
        initialized,
        ..
    } = st;
    let resolve = |src: &Option<Source>| -> Result<Option<(&Recording, String)>, String> {
        let Some(src) = src else { return Ok(None) };
        let rec = match src {
            Source::Memory => snap.viewed_recording().map(|(r, _, _)| r),
            Source::File(p) => match cache.get(p) {
                Some(Ok(r)) => Some(r.as_ref()),
                Some(Err(e)) => return Err(format!("{}: {e}", file_label(p))),
                None => None,
            },
        };
        Ok(rec.map(|r| (r, source_label(src, snap, cache))))
    };
    let (ra, rb) = match (resolve(a), resolve(b)) {
        (Ok(Some(ra)), Ok(Some(rb))) => (ra, rb),
        (Err(e), _) | (_, Err(e)) => {
            ui.colored_label(
                crate::heat::ink(ui.visuals(), RED),
                format!("Could not load {e}"),
            );
            return;
        }
        _ => {
            ui.add_space(8.0);
            ui.weak("Pick two recordings to compare (e.g. before and after a repaste, driver update or XMP change). Saved recordings are the OmniHW-Recording-*.csv files in Documents.");
            return;
        }
    };
    let ((rec_a, name_a), (rec_b, name_b)) = (ra, rb);

    // Sensors present in both, in A's order.
    let common: Vec<&Column> = rec_a
        .columns
        .iter()
        .filter(|c| rec_b.column_by_key(&c.key()).is_some())
        .collect();
    if common.is_empty() {
        ui.colored_label(
            crate::heat::ink(ui.visuals(), RED),
            "These recordings have no sensors in common.",
        );
        return;
    }
    if !*initialized {
        for c in &common {
            let k = c.key();
            if DEFAULT_KEYS.iter().any(|d| {
                if d.starts_with('/') {
                    k.ends_with(d)
                } else {
                    k == *d
                }
            }) {
                selected.insert(k);
            }
        }
        *initialized = true;
    }

    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("A (solid): {name_a}")).strong());
        ui.label("   ");
        ui.label(RichText::new(format!("B (dashed): {name_b}")).strong());
    });
    ui.separator();

    let h = ui.available_height();
    ui.horizontal_top(|ui| {
        let down = egui::Layout::top_down(egui::Align::Min);
        ui.allocate_ui_with_layout(vec2(260.0, h), down, |ui| {
            ui.set_width(260.0);
            sensor_picker(ui, "cmp_picker", &common, |c| c.key(), selected, filter);
        });
        ui.separator();
        ui.allocate_ui_with_layout(vec2(ui.available_width(), h), down, |ui| {
            charts(ui, rec_a, rec_b, &common, selected);
        });
    });
}

/// (column in A, column in B) for each selected key, in A's order.
type Pair = (usize, usize);

fn charts(
    ui: &mut Ui,
    a: &Recording,
    b: &Recording,
    common: &[&Column],
    selected: &BTreeSet<String>,
) {
    let pairs: Vec<(Pair, Unit)> = common
        .iter()
        .filter(|c| selected.contains(&c.key()))
        .filter_map(|c| {
            Some((
                (a.column_by_key(&c.key())?, b.column_by_key(&c.key())?),
                c.unit,
            ))
        })
        .collect();
    if pairs.is_empty() {
        ui.weak("Select sensors on the left to compare them.");
        return;
    }
    let mut units: Vec<Unit> = Vec::new();
    for (_, u) in &pairs {
        if !units.contains(u) {
            units.push(*u);
        }
    }
    egui::ScrollArea::vertical()
        .id_salt("cmp_charts")
        .auto_shrink(false)
        .show(ui, |ui| {
            for u in units {
                let group: Vec<(usize, Pair)> = pairs
                    .iter()
                    .enumerate()
                    .filter(|(_, (_, pu))| *pu == u)
                    .map(|(i, (p, _))| (i, *p))
                    .collect();
                let title = match u.symbol() {
                    "" => "Values".to_string(),
                    s => s.to_string(),
                };
                ui.label(RichText::new(title).strong());
                chart(ui, a, b, &group, u);
                stats_table(ui, a, b, &group, u);
                ui.add_space(12.0);
            }
        });
}

fn color(i: usize) -> egui::Color32 {
    PALETTE[i % PALETTE.len()]
}

/// A (solid) and B (dashed) overlaid on elapsed time.
fn chart(ui: &mut Ui, a: &Recording, b: &Recording, group: &[(usize, Pair)], unit: Unit) {
    let area = chart_area(ui);
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for &(_, (ca, cb)) in group {
        for s in [a.stats(ca), b.stats(cb)].into_iter().flatten() {
            lo = lo.min(s.min);
            hi = hi.max(s.max);
        }
    }
    if !lo.is_finite() {
        return;
    }
    let (lo, hi) = y_range(lo, hi, unit);
    let dur = a.duration_secs().max(b.duration_secs()).max(1e-6);
    draw_axes(&area, lo, hi, dur, unit);
    let ChartArea {
        plot,
        painter: p,
        resp,
        weak,
    } = area;
    let x_of = |t: f64| plot.left() + plot.width() * (t / dur) as f32;
    let y_of = |v: f64| plot.bottom() - plot.height() * ((v - lo) / (hi - lo)) as f32;
    let buckets = plot.width() as usize;
    for &(i, (ca, cb)) in group {
        let pa = series_points(&a.times, &a.values[ca], buckets, x_of, y_of);
        if pa.len() >= 2 {
            p.line(pa, Stroke::new(1.6, color(i)));
        }
        let pb = series_points(&b.times, &b.values[cb], buckets, x_of, y_of);
        if pb.len() >= 2 {
            p.extend(egui::Shape::dashed_line(
                &pb,
                Stroke::new(1.4, color(i).gamma_multiply(0.75)),
                6.0,
                4.0,
            ));
        }
    }

    if let Some(pos) = resp.hover_pos().filter(|p| plot.x_range().contains(p.x)) {
        let t = ((pos.x - plot.left()) / plot.width()) as f64 * dur;
        p.line_segment(
            [pos2(pos.x, plot.top()), pos2(pos.x, plot.bottom())],
            Stroke::new(1.0, weak),
        );
        let at = |rec: &Recording, c: usize| -> String {
            if rec.is_empty() || t > rec.duration_secs() {
                return "-".into();
            }
            let i = rec.times.partition_point(|&x| x < t).min(rec.len() - 1);
            let v = rec.values[c][i];
            if v.is_finite() {
                unit.format(v as f64)
            } else {
                "-".into()
            }
        };
        resp.on_hover_ui_at_pointer(|ui| {
            ui.strong(format_elapsed(t));
            for &(i, (ca, cb)) in group {
                ui.colored_label(
                    crate::heat::ink(ui.visuals(), color(i)),
                    format!(
                        "{}: A {}  ·  B {}",
                        a.columns[ca].label,
                        at(a, ca),
                        at(b, cb)
                    ),
                );
            }
        });
    }
}

/// Signed delta text, e.g. "+3.2 °C (+5 %)".
fn delta_text(d: f64, base: f64, unit: Unit) -> String {
    let sign = if d > 0.0 { "+" } else { "" };
    let pct = if base.abs() > 1e-9 {
        format!(" ({sign}{:.0} %)", d / base * 100.0)
    } else {
        String::new()
    };
    format!("{sign}{}{pct}", unit.format(d))
}

/// Lower temperature and power are better: color those deltas.
fn delta_rich(text: String, d: f64, unit: Unit) -> RichText {
    let t = RichText::new(text).monospace();
    match unit {
        Unit::Celsius | Unit::Watt if d < 0.0 => t.color(GREEN),
        Unit::Celsius | Unit::Watt if d > 0.0 => t.color(RED),
        _ => t,
    }
}

fn stats_table(ui: &mut Ui, a: &Recording, b: &Recording, group: &[(usize, Pair)], unit: Unit) {
    egui::Grid::new(("cmp_stats", group.first().map_or(0, |g| g.0)))
        .striped(true)
        .num_columns(7)
        .spacing([14.0, 2.0])
        .show(ui, |ui| {
            for h in [
                "Sensor", "Avg A", "Avg B", "Δ Avg", "Max A", "Max B", "Δ Max",
            ] {
                ui.label(RichText::new(h).underline());
            }
            ui.end_row();
            for &(i, (ca, cb)) in group {
                let (Some(sa), Some(sb)) = (a.stats(ca), b.stats(cb)) else {
                    continue;
                };
                ui.colored_label(
                    crate::heat::ink(ui.visuals(), color(i)),
                    format!("{} · {}", a.columns[ca].group, a.columns[ca].label),
                );
                let f = |v: f64| RichText::new(unit.format(v)).monospace();
                ui.label(f(sa.avg));
                ui.label(f(sb.avg));
                let d = sb.avg - sa.avg;
                ui.label(delta_rich(delta_text(d, sa.avg, unit), d, unit));
                ui.label(f(sa.max));
                ui.label(f(sb.max));
                let d = sb.max - sa.max;
                ui.label(delta_rich(delta_text(d, sa.max, unit), d, unit));
                ui.end_row();
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_from_file_names() {
        assert_eq!(
            file_label(Path::new("OmniHW-Recording-20261012-012733.csv")),
            "2026-10-12 01:27:33 UTC"
        );
        assert_eq!(
            file_label(Path::new("OmniHW-Recording-custom.csv")),
            "OmniHW-Recording-custom"
        );
    }

    #[test]
    fn deltas() {
        assert_eq!(delta_text(3.0, 60.0, Unit::Celsius), "+3.0 °C (+5 %)");
        assert_eq!(delta_text(-10.0, 0.0, Unit::Watt), "-10.0 W");
    }
}
