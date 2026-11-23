//! Recording compare: overlay two sessions (e.g. before/after a repaste, driver update or
//! XMP change) and show per-sensor deltas. Sensors are matched by group + label
//! ([`Column::key`]), since saved CSVs don't store sensor ids.

use super::recording::{
    COMPARE_CHARTS, Chart, ChartHover, Line, PALETTE, Snap, axis_labels, chart_element,
    filter_input, sensor_picker, series_points, split, unit_title, y_range,
};
use crate::app::OmniApp;
use crate::export;
use crate::heat::{GREEN, RED};
use crate::sampler::Snapshot;
use crate::ui;
use gpui_kit::component::button::Button;
use gpui_kit::component::input::InputState;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::separator::Separator;
use gpui_kit::component::{Sizable, v_flex};
use gpui_kit::{
    AnyElement, Context, Div, ElementId, Entity, InteractiveElement, IntoElement, ParentElement,
    Rgba, SharedString, Styled, Window,
};
use omnihw_core::recording::{Column, Recording, format_elapsed};
use omnihw_core::sensors::Unit;
use std::collections::{BTreeSet, HashMap, HashSet};
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
    /// The sensor picker's filter field (created on first show).
    filter: Option<Entity<InputState>>,
    /// Whether each sensor group of the picker started open.
    groups_open: HashMap<String, bool>,
    initialized: bool,
    /// "Refresh list" was clicked: rescan on the next redraw.
    rescan: bool,
}

/// Selects one of the recording slots of the compare state, from the app (for click handlers).
type Slot = fn(&mut OmniApp) -> &mut Option<Source>;

impl CompareState {
    /// Do this view's file work: list saved recordings (first time and after "Refresh list") and
    /// parse the CSVs picked as A, B or the single recording. Call it before taking the snapshot
    /// lock, so reading files never holds up the sampler. Files already parsed are kept, so
    /// nothing is read again on later redraws.
    pub fn load_files(&mut self) {
        if !self.scanned || std::mem::take(&mut self.rescan) {
            self.scan();
        }
        let picked = [&self.a, &self.b, &self.single];
        let paths: Vec<PathBuf> = picked
            .into_iter()
            .filter_map(|s| match s {
                Some(Source::File(p)) => Some(p.clone()),
                _ => None,
            })
            .collect();
        for p in paths {
            self.load(&p);
        }
    }

    /// Pick the recording for single-recording views if none is picked yet: the in-memory one
    /// (`in_memory` says whether there is one), else the newest saved one. Call it before
    /// [`Self::load_files`], which then parses the file picked here.
    pub fn default_single(&mut self, in_memory: bool) {
        if self.single.is_some() {
            return;
        }
        if !self.scanned {
            self.scan();
        }
        self.single = if in_memory {
            Some(Source::Memory)
        } else {
            self.files.first().map(|f| Source::File(f.path.clone()))
        };
    }

    /// Create the sensor picker's filter field on first use.
    pub fn ensure_filter(&mut self, window: &mut Window, cx: &mut Context<OmniApp>) {
        if self.filter.is_none() {
            self.filter = Some(filter_input(window, cx));
        }
    }

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

    /// Tests: treat the file list as read (and empty), so the real Documents folder isn't.
    #[cfg(test)]
    pub(super) fn skip_scan(&mut self) {
        self.scanned = true;
    }

    /// Tests: compare the in-memory recording with itself.
    #[cfg(test)]
    pub(super) fn compare_memory(&mut self) {
        self.a = Some(Source::Memory);
        self.b = Some(Source::Memory);
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

/// Drop-down of the in-memory recording (if any) and the saved ones; the choice goes into the
/// slot `slot` selects.
fn source_picker(
    id: impl Into<ElementId>,
    current: &Option<Source>,
    slot: Slot,
    snap: &Snapshot,
    st: &CompareState,
    cx: &mut Context<OmniApp>,
) -> AnyElement {
    let text = current
        .as_ref()
        .map_or("Choose a recording…".to_string(), |s| {
            source_label(s, snap, &st.cache)
        });
    let mut sources: Vec<Source> = Vec::new();
    if snap.viewed_recording().is_some() {
        sources.push(Source::Memory);
    }
    sources.extend(st.files.iter().map(|f| Source::File(f.path.clone())));
    let options = sources
        .iter()
        .map(|s| SharedString::from(source_label(s, snap, &st.cache)))
        .collect();
    let chosen = current
        .as_ref()
        .and_then(|c| sources.iter().position(|s| s == c));
    ui::dropdown(id, text, options, chosen, cx, move |this, i, _, _| {
        *slot(this) = Some(sources[i].clone());
    })
}

/// The "Recording" picker row of single-recording views. Pick the default with
/// [`CompareState::default_single`] first.
pub(super) fn single_picker(snap: &Snapshot, st: &CompareState, cx: &mut Context<OmniApp>) -> Div {
    ui::wrap_row()
        .child(ui::strong("Recording"))
        .child(source_picker(
            "single_src",
            &st.single,
            |this| &mut this.views.recording.compare.single,
            snap,
            st,
            cx,
        ))
        .child(refresh_button(cx))
}

/// The recording picked for single-recording views (the in-memory one or a parsed CSV) and
/// its name.
pub(super) fn pick_single<'a>(
    snap: &'a Snapshot,
    st: &'a CompareState,
) -> Result<Option<(&'a Recording, String)>, String> {
    let Some(src) = &st.single else {
        return Ok(None);
    };
    resolve(src, snap, &st.cache)
}

/// The recording `src` names, if it's in memory or parsed, and its name.
fn resolve<'a>(
    src: &Source,
    snap: &'a Snapshot,
    cache: &'a HashMap<PathBuf, Result<Arc<Recording>, String>>,
) -> Result<Option<(&'a Recording, String)>, String> {
    let rec = match src {
        Source::Memory => snap.viewed_recording().map(|(r, _, _)| r),
        Source::File(p) => match cache.get(p) {
            Some(Ok(r)) => Some(r.as_ref()),
            Some(Err(e)) => return Err(format!("{}: {e}", file_label(p))),
            None => None,
        },
    };
    Ok(rec.map(|r| (r, source_label(src, snap, cache))))
}

/// "Refresh list": rescans on the next redraw, outside the snapshot lock.
fn refresh_button(cx: &mut Context<OmniApp>) -> Button {
    Button::new("refresh-recordings")
        .small()
        .label("Refresh list")
        .tooltip("Rescan Documents for saved recordings")
        .on_click(cx.listener(|this, _, _, cx| {
            this.views.recording.compare.rescan = true;
            cx.notify();
        }))
}

/// First column index of every sensor key ("group / label") in a recording.
fn key_index(rec: &Recording) -> HashMap<String, usize> {
    let mut map = HashMap::with_capacity(rec.columns.len());
    for (i, c) in rec.columns.iter().enumerate() {
        map.entry(c.key()).or_insert(i);
    }
    map
}

pub(super) fn show(
    snap: &Snapshot,
    st: &mut CompareState,
    charts: &ChartHover,
    toggled: &HashSet<SharedString>,
    buckets: usize,
    cx: &mut Context<OmniApp>,
) -> AnyElement {
    let pickers = ui::wrap_row()
        .child(ui::strong("A"))
        .child(source_picker(
            "cmp_a",
            &st.a,
            |this| &mut this.views.recording.compare.a,
            snap,
            st,
            cx,
        ))
        .child(ui::strong("B"))
        .child(source_picker(
            "cmp_b",
            &st.b,
            |this| &mut this.views.recording.compare.b,
            snap,
            st,
            cx,
        ))
        .child(refresh_button(cx));
    let page = v_flex().flex_1().min_h_0().w_full().gap_1().child(pickers);

    let CompareState {
        cache,
        a,
        b,
        selected,
        filter,
        groups_open,
        initialized,
        ..
    } = st;
    let resolve = |src: &Option<Source>| match src {
        Some(src) => resolve(src, snap, cache),
        None => Ok(None),
    };
    let (ra, rb) = match (resolve(a), resolve(b)) {
        (Ok(Some(ra)), Ok(Some(rb))) => (ra, rb),
        (Err(e), _) | (_, Err(e)) => {
            return page
                .child(ui::colored(format!("Could not load {e}"), RED, cx))
                .into_any_element();
        }
        _ => {
            return page
                .child(ui::space(8.0))
                .child(ui::weak("Pick two recordings to compare (e.g. before and after a repaste, driver update or XMP change). Saved recordings are the OmniHW-Recording-*.csv files in Documents.", cx))
                .into_any_element();
        }
    };
    let ((rec_a, name_a), (rec_b, name_b)) = (ra, rb);

    // Sensors present in both, in A's order, with their key and (A, B) columns.
    let (keys_a, keys_b) = (key_index(rec_a), key_index(rec_b));
    let common: Vec<Common> = rec_a
        .columns
        .iter()
        .filter_map(|c| {
            let key = c.key();
            let pair = (*keys_a.get(&key)?, *keys_b.get(&key)?);
            Some(Common {
                column: c,
                key,
                pair,
            })
        })
        .collect();
    if common.is_empty() {
        return page
            .child(ui::colored(
                "These recordings have no sensors in common.",
                RED,
                cx,
            ))
            .into_any_element();
    }
    if !*initialized {
        for c in &common {
            let k = &c.key;
            if DEFAULT_KEYS.iter().any(|d| {
                if d.starts_with('/') {
                    k.ends_with(d)
                } else {
                    k == d
                }
            }) {
                selected.insert(k.clone());
            }
        }
        *initialized = true;
    }

    let names = ui::row()
        .gap_6()
        .child(ui::strong(format!("A (solid): {name_a}")))
        .child(ui::strong(format!("B (dashed): {name_b}")));
    let columns: Vec<&Column> = common.iter().map(|c| c.column).collect();
    let picker = sensor_picker(
        "cmp_picker",
        &columns,
        |c| c.key(),
        selected,
        filter.as_ref().expect("created by ensure_filter"),
        groups_open,
        toggled,
        |this| &mut this.views.recording.compare.selected,
        cx,
    );
    let charts = compare_charts(rec_a, rec_b, &common, selected, charts, buckets, cx);
    page.child(names)
        .child(Separator::horizontal())
        .child(split(picker, charts))
        .into_any_element()
}

/// (column in A, column in B) for one sensor.
type Pair = (usize, usize);

/// A sensor present in both recordings.
struct Common<'a> {
    column: &'a Column,
    key: String,
    pair: Pair,
}

fn compare_charts(
    a: &Recording,
    b: &Recording,
    common: &[Common],
    selected: &BTreeSet<String>,
    charts: &ChartHover,
    buckets: usize,
    cx: &mut Context<OmniApp>,
) -> AnyElement {
    let pairs: Vec<(Pair, Unit)> = common
        .iter()
        .filter(|c| selected.contains(&c.key))
        .map(|c| (c.pair, c.column.unit))
        .collect();
    if pairs.is_empty() {
        return ui::weak("Select sensors on the left to compare them.", cx).into_any_element();
    }
    let mut units: Vec<Unit> = Vec::new();
    for (_, u) in &pairs {
        if !units.contains(u) {
            units.push(*u);
        }
    }
    let mut list = v_flex().id("cmp_charts").flex_1().min_h_0().gap_1();
    for (n, u) in units.into_iter().enumerate() {
        let group: Vec<(usize, Pair)> = pairs
            .iter()
            .enumerate()
            .filter(|(_, (_, pu))| *pu == u)
            .map(|(i, (p, _))| (i, *p))
            .collect();
        let chart = chart(a, b, &group, u, n, charts, buckets, cx);
        list = list.child(
            v_flex()
                .id(("unit", n))
                .w_full()
                .gap_1()
                .child(ui::strong(unit_title(u)))
                .child(chart)
                .child(stats_table(a, b, &group, u, cx))
                .child(ui::space(12.0)),
        );
    }
    list.overflow_y_scrollbar().into_any_element()
}

fn color(i: usize) -> Rgba {
    PALETTE[i % PALETTE.len()]
}

/// A (solid) and B (dashed) overlaid on elapsed time.
#[allow(clippy::too_many_arguments)]
fn chart(
    a: &Recording,
    b: &Recording,
    group: &[(usize, Pair)],
    unit: Unit,
    n: usize,
    charts: &ChartHover,
    buckets: usize,
    cx: &mut Context<OmniApp>,
) -> AnyElement {
    let key = COMPARE_CHARTS + n;
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for &(_, (ca, cb)) in group {
        for s in [a.stats(ca), b.stats(cb)].into_iter().flatten() {
            lo = lo.min(s.min);
            hi = hi.max(s.max);
        }
    }
    if !lo.is_finite() {
        return chart_element(Chart::empty(key, Snap::Free), charts, cx);
    }
    let (lo, hi) = y_range(lo, hi, unit);
    let dur = a.duration_secs().max(b.duration_secs()).max(1e-6);
    let (y_labels, x_labels) = axis_labels(lo, hi, dur, unit);
    let x_of = |t: f64| (t / dur) as f32;
    let y_of = |v: f64| ((v - lo) / (hi - lo)) as f32;
    let mut lines = Vec::with_capacity(group.len() * 2);
    for &(i, (ca, cb)) in group {
        lines.push(Line {
            pts: series_points(&a.times, &a.values[ca], buckets, x_of, y_of),
            color: color(i),
            width: 1.6,
            dashed: false,
        });
        lines.push(Line {
            pts: series_points(&b.times, &b.values[cb], buckets, x_of, y_of),
            color: crate::heat::fade(color(i), 0.75),
            width: 1.4,
            dashed: true,
        });
    }

    let mut cursor = None;
    let mut readout = Vec::new();
    if let Some(h) = charts.hovered(key) {
        let t = h.frac as f64 * dur;
        cursor = Some(h.frac);
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
        readout.push((format_elapsed(t), None));
        for &(i, (ca, cb)) in group {
            readout.push((
                format!(
                    "{}: A {}  ·  B {}",
                    a.columns[ca].label,
                    at(a, ca),
                    at(b, cb)
                ),
                Some(color(i)),
            ));
        }
    }
    chart_element(
        Chart {
            key,
            lines,
            y_labels,
            x_labels,
            snap: Snap::Free,
            cursor,
            readout,
        },
        charts,
        cx,
    )
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

/// Lower temperature and power are better: the colour of those deltas.
fn delta_color(d: f64, unit: Unit) -> Option<Rgba> {
    match unit {
        Unit::Celsius | Unit::Watt if d < 0.0 => Some(GREEN),
        Unit::Celsius | Unit::Watt if d > 0.0 => Some(RED),
        _ => None,
    }
}

fn stats_table(
    a: &Recording,
    b: &Recording,
    group: &[(usize, Pair)],
    unit: Unit,
    cx: &mut Context<OmniApp>,
) -> Div {
    let mut table = ui::Table::new(&[
        "Sensor", "Avg A", "Avg B", "Δ Avg", "Max A", "Max B", "Δ Max",
    ])
    .striped();
    for &(i, (ca, cb)) in group {
        let (Some(sa), Some(sb)) = (a.stats(ca), b.stats(cb)) else {
            continue;
        };
        let f = |v: f64| ui::mono(unit.format(v), cx).into_any_element();
        let delta = |d: f64, base: f64| {
            let text = ui::mono(delta_text(d, base, unit), cx);
            match delta_color(d, unit) {
                Some(c) => text.text_color(c),
                None => text,
            }
            .into_any_element()
        };
        table.push_row([
            ui::colored(
                format!("{} · {}", a.columns[ca].group, a.columns[ca].label),
                color(i),
                cx,
            )
            .into_any_element(),
            f(sa.avg),
            f(sb.avg),
            delta(sb.avg - sa.avg, sa.avg),
            f(sa.max),
            f(sb.max),
            delta(sb.max - sa.max, sa.max),
        ]);
    }
    table.build(cx)
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
        assert_eq!(delta_color(-1.0, Unit::Celsius), Some(GREEN));
        assert_eq!(delta_color(2.0, Unit::Watt), Some(RED));
        assert_eq!(delta_color(2.0, Unit::Percent), None);
    }
}
