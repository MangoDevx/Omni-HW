//! Recording page: start/stop, pick sensors, and chart them over the whole session. Its other
//! modes compare two recordings ([`super::compare`]) and analyze one for a CPU or GPU
//! bottleneck ([`super::bottleneck`]).
//!
//! Charts are drawn on a [`canvas`]: everything the paint needs (points as fractions of the
//! plot area, colours) is worked out first, the canvas only scales it into its bounds. Axis
//! labels and the hover read-out are real labels laid over the chart, so tests and screen
//! readers see them. A chart is rebuilt only when the app is notified (new sample, click,
//! the pointer moving to another sample); nothing animates.

use crate::app::OmniApp;
use crate::heat::{self, rgb8};
use crate::sampler::{Sampler, Snapshot};
use crate::{export, ui};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::separator::Separator;
use gpui_kit::component::{ActiveTheme, Selectable, Sizable, h_flex, v_flex};
use gpui_kit::{
    AnyElement, AppContext, Bounds, Context, Div, Entity, Hsla, InteractiveElement, IntoElement,
    MouseMoveEvent, ParentElement, PathBuilder, Pixels, Point, Rgba, SharedString,
    StatefulInteractiveElement, Styled, Window, canvas, div, fill, point, px, relative, rems, size,
};
use omnihw_core::recording::{Column, Recording, format_elapsed};
use omnihw_core::sensors::Unit;
use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::rc::Rc;

pub(super) const PALETTE: [Rgba; 8] = [
    heat::BLUE,
    rgb8(38, 166, 154),
    rgb8(245, 156, 26),
    rgb8(171, 71, 188),
    rgb8(239, 83, 80),
    rgb8(124, 179, 66),
    rgb8(92, 107, 192),
    rgb8(141, 110, 99),
];

/// Sensors pre-selected when nothing has been chosen yet.
const DEFAULT_SELECTION: [&str; 7] = [
    "cpu/load/total",
    "cpu/clock/avg",
    "power/RAPL_Package0_PKG",
    "cpu/power/package",
    "cpu/temp/package",
    "mem/load",
    "/temp",
];

#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    #[default]
    Session,
    /// Compare two recordings.
    Compare,
    /// CPU vs GPU bottleneck analysis of one recording.
    Bottleneck,
}

#[derive(Default)]
pub struct ViewState {
    /// Ids of the sensors charted in Session mode.
    selected: BTreeSet<String>,
    /// The sensor picker's filter field (created on first show).
    filter: Option<Entity<InputState>>,
    /// Whether each sensor group of the picker started open (see [`sensor_picker`]).
    groups_open: HashMap<String, bool>,
    /// The default selection was applied.
    initialized: bool,
    mode: Mode,
    pub(super) compare: super::compare::CompareState,
    analysis: super::bottleneck::AnalysisCache,
    /// Where each chart was drawn and which sample the pointer is over.
    charts: ChartHover,
}

/// Start or stop recording; returns a status message for the status bar.
pub fn toggle(sampler: &Sampler) -> String {
    if sampler.snapshot().recording.is_some() {
        match sampler.stop_recording() {
            Some(p) => format!("Recording saved to {}", p.display()),
            None => "Not recording".into(),
        }
    } else {
        match sampler.start_recording(&crate::export::report_dir(), &crate::export::timestamp()) {
            Ok(p) => format!("Recording to {}", p.display()),
            Err(e) => e,
        }
    }
}

pub fn show(app: &mut OmniApp, window: &mut Window, cx: &mut Context<OmniApp>) -> AnyElement {
    let st = &mut app.views.recording;
    if st.filter.is_none() {
        st.filter = Some(filter_input(window, cx));
    }
    st.compare.ensure_filter(window, cx);
    let mode = st.mode;
    // File work (listing and parsing saved recordings) happens before the snapshot lock is
    // taken, so reading files never holds up the sampler.
    if mode == Mode::Bottleneck {
        let in_memory = app.sampler.snapshot().viewed_recording().is_some();
        app.views.recording.compare.default_single(in_memory);
    }
    if mode != Mode::Session {
        app.views.recording.compare.load_files();
    }
    // Series are downsampled to about two points per pixel column of the widest possible plot.
    let buckets = f32::from(window.viewport_size().width).max(1.0) as usize;

    let snap = app.sampler.snapshot();
    let ViewState {
        selected,
        filter,
        groups_open,
        initialized,
        compare,
        analysis,
        charts,
        ..
    } = &mut app.views.recording;
    let toggled = &app.views.toggled;

    let page = v_flex()
        .flex_1()
        .min_h_0()
        .w_full()
        .gap_1()
        .child(toolbar(&snap, mode, cx));

    match mode {
        Mode::Compare => {
            return page
                .child(Separator::horizontal())
                .child(super::compare::show(
                    &snap, compare, charts, toggled, buckets, cx,
                ))
                .into_any_element();
        }
        Mode::Bottleneck => {
            let gpus = app.sampler.info().map_or(&[][..], |i| &i.gpus[..]);
            return page
                .child(Separator::horizontal())
                .child(super::bottleneck::show(
                    &snap, compare, analysis, charts, gpus, buckets, cx,
                ))
                .into_any_element();
        }
        Mode::Session => {}
    }
    let Some((rec, _, _)) = snap.viewed_recording() else {
        return page.into_any_element();
    };
    if !*initialized {
        for c in &rec.columns {
            if DEFAULT_SELECTION.iter().any(|d| {
                if d.starts_with('/') {
                    c.id.ends_with(d)
                } else {
                    c.id == *d
                }
            }) {
                selected.insert(c.id.clone());
            }
        }
        *initialized = true;
    }
    let cols: Vec<&Column> = rec.columns.iter().collect();
    let picker = sensor_picker(
        "rec_picker",
        &cols,
        |c| c.id.clone(),
        selected,
        filter.as_ref().expect("created above"),
        groups_open,
        toggled,
        |this| &mut this.views.recording.selected,
        cx,
    );
    let charts = session_charts(rec, selected, charts, buckets, cx);
    page.child(Separator::horizontal())
        .child(split(picker, charts))
        .into_any_element()
}

/// Mode buttons, the page's own Start/Stop button and what is recorded.
fn toolbar(snap: &Snapshot, mode: Mode, cx: &mut Context<OmniApp>) -> Div {
    let active = snap.recording.is_some();
    let mode_button =
        |id: &'static str, label: &'static str, m: Mode, cx: &mut Context<OmniApp>| {
            Button::new(id)
                .small()
                .label(label)
                .selected(mode == m)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.views.recording.mode = m;
                    cx.notify();
                }))
        };
    let start_stop = Button::new("rec-toggle")
        .small()
        .label(if active {
            "Stop recording"
        } else {
            "Start recording"
        })
        .on_click(cx.listener(|this, _, _, cx| {
            let msg = toggle(&this.sampler);
            this.set_status(msg, cx);
        }));
    let start_stop = if active {
        start_stop.danger()
    } else {
        start_stop
    };
    let mut row = ui::wrap_row()
        .child(mode_button("mode-session", "Session", Mode::Session, cx))
        .child(mode_button("mode-compare", "Compare", Mode::Compare, cx))
        .child(mode_button(
            "mode-bottleneck",
            "Bottleneck",
            Mode::Bottleneck,
            cx,
        ))
        .child(Separator::vertical().h(rems(1.25)))
        .child(start_stop);
    match snap.viewed_recording() {
        Some((rec, path, live)) => {
            let what = if live { "Recording" } else { "Last recording" };
            row = row.child(ui::label(format!(
                "{what}: {}  ·  {} samples  ·  {} sensors",
                format_elapsed(rec.duration_secs()),
                rec.len(),
                rec.columns.len()
            )));
            let shown = path.clone();
            row = row.child(
                Button::new("show-csv")
                    .small()
                    .label("Show CSV")
                    .tooltip(path.display().to_string())
                    .on_click(move |_, _, _| export::open_in_explorer(&shown, true)),
            );
            if !live {
                let opened = path.clone();
                row = row.child(
                    Button::new("open-csv")
                        .small()
                        .label("Open CSV")
                        .on_click(move |_, _, _| export::open_in_explorer(&opened, false)),
                );
            }
        }
        None => {
            row = row.child(ui::weak(
                "Records every sensor at the sampling interval until you stop. Data is saved as CSV in Documents.",
                cx,
            ));
        }
    }
    if let Some(e) = snap.recording.as_ref().and_then(|r| r.write_error.as_ref()) {
        row = row.child(ui::colored(
            format!("CSV write failed: {e} (still recording in memory)"),
            heat::ORANGE,
            cx,
        ));
    }
    row
}

/// Width of the sensor picker, in rems.
const PICKER_WIDTH: f32 = 16.25;

/// Sensor picker on the left, charts on the right; each scrolls on its own and together they
/// fill the rest of the page.
pub(super) fn split(picker: Div, charts: AnyElement) -> Div {
    h_flex()
        .flex_1()
        .min_h_0()
        .w_full()
        .items_stretch()
        .gap_2()
        .child(
            v_flex()
                .flex_none()
                .w(rems(PICKER_WIDTH))
                .min_h_0()
                .child(picker),
        )
        .child(Separator::vertical())
        .child(v_flex().flex_1().min_w_0().min_h_0().child(charts))
}

/// One chart and stats table per unit of the selected sensors.
fn session_charts(
    rec: &Recording,
    selected: &BTreeSet<String>,
    charts: &ChartHover,
    buckets: usize,
    cx: &mut Context<OmniApp>,
) -> AnyElement {
    let cols: Vec<usize> = (0..rec.columns.len())
        .filter(|&i| selected.contains(&rec.columns[i].id))
        .collect();
    if cols.is_empty() {
        return ui::weak("Select sensors on the left to chart them.", cx).into_any_element();
    }
    if rec.len() < 2 {
        return ui::weak("Collecting samples…", cx).into_any_element();
    }
    // One chart per unit so values share a meaningful axis.
    let mut units: Vec<Unit> = Vec::new();
    for &c in &cols {
        if !units.contains(&rec.columns[c].unit) {
            units.push(rec.columns[c].unit);
        }
    }
    let times = Rc::new(rec.times.clone());
    let mut list = v_flex().id("rec_charts").flex_1().min_h_0().gap_1();
    for (n, u) in units.into_iter().enumerate() {
        let group: Vec<usize> = cols
            .iter()
            .copied()
            .filter(|&c| rec.columns[c].unit == u)
            .collect();
        let chart = chart(rec, &group, &cols, u, n, &times, charts, buckets, cx);
        list = list.child(
            v_flex()
                .id(("unit", n))
                .w_full()
                .gap_1()
                .child(ui::strong(unit_title(u)))
                .child(chart)
                .child(stats_table(rec, &group, &cols, u, cx))
                .child(ui::space(12.0)),
        );
    }
    list.overflow_y_scrollbar().into_any_element()
}

/// Heading of a unit's chart: its symbol, or "Values" for unitless sensors.
pub(super) fn unit_title(u: Unit) -> String {
    match u.symbol() {
        "" => "Values".to_string(),
        s => s.to_string(),
    }
}

/// The filter field of a sensor picker; the page is redrawn as it is typed in.
pub(super) fn filter_input(window: &mut Window, cx: &mut Context<OmniApp>) -> Entity<InputState> {
    let state = cx.new(|cx| InputState::new(window, cx).placeholder("Filter sensors"));
    cx.subscribe(&state, |_, _, event: &InputEvent, cx| {
        if matches!(event, InputEvent::Change) {
            cx.notify();
        }
    })
    .detach();
    state
}

/// Grouped, filterable checkbox list of columns. `key` gives the id stored in the selection,
/// which `set` reaches from the app in click handlers. A group starts open when one of its
/// sensors is selected or a filter is typed the first time it is shown; after that it keeps
/// whatever the user toggled (`groups_open` remembers the starting state, the app's toggled
/// set the changes).
#[allow(clippy::too_many_arguments)]
pub(super) fn sensor_picker(
    salt: &'static str,
    cols: &[&Column],
    key: impl Fn(&Column) -> String,
    selected: &BTreeSet<String>,
    filter: &Entity<InputState>,
    groups_open: &mut HashMap<String, bool>,
    toggled: &HashSet<SharedString>,
    set: fn(&mut OmniApp) -> &mut BTreeSet<String>,
    cx: &mut Context<OmniApp>,
) -> Div {
    let top = ui::row()
        .child(Input::new(filter).small().w(rems(9.5)))
        .child(
            Button::new((salt, usize::MAX))
                .small()
                .label("Clear")
                .on_click(cx.listener(move |this, _, _, cx| {
                    set(this).clear();
                    cx.notify();
                })),
        );
    let filter = filter.read(cx).value().to_lowercase();
    let mut groups: Vec<&str> = Vec::new();
    for c in cols {
        if !groups.contains(&c.group.as_str()) {
            groups.push(&c.group);
        }
    }
    let mut list = v_flex().flex_1().min_h_0().gap_0p5();
    for g in groups {
        let matching: Vec<(usize, &&Column)> = cols
            .iter()
            .enumerate()
            .filter(|(_, c)| c.group == g)
            .filter(|(_, c)| {
                filter.is_empty()
                    || c.label.to_lowercase().contains(&filter)
                    || g.to_lowercase().contains(&filter)
            })
            .collect();
        if matching.is_empty() {
            continue;
        }
        let group_key = format!("{salt}/{g}");
        let any_selected = matching.iter().any(|(_, c)| selected.contains(&key(c)));
        let default_open = *groups_open
            .entry(group_key.clone())
            .or_insert(any_selected || !filter.is_empty());
        let open = toggled.contains(group_key.as_str()) != default_open;
        list = list.child(ui::collapse_header(
            group_key,
            ui::strong(g.to_string()),
            open,
            cx,
        ));
        if !open {
            continue;
        }
        let mut body = v_flex().pl_4().gap_0p5();
        for (i, c) in matching {
            let k = key(c);
            let on = selected.contains(&k);
            body = body.child(
                Checkbox::new((salt, i))
                    .label(c.label.clone())
                    .checked(on)
                    .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                        if *checked {
                            set(this).insert(k.clone());
                        } else {
                            set(this).remove(&k);
                        }
                        cx.notify();
                    })),
            );
        }
        list = list.child(body);
    }
    v_flex()
        .flex_1()
        .min_h_0()
        .gap_1()
        .child(top)
        .child(Separator::horizontal())
        .child(
            // One scroll position per picker (both are built here).
            list.id(salt).overflow_y_scrollbar().id((salt, 0usize)),
        )
}

/// Color for a selected column: palette entry by its position among all selected columns,
/// so series in the same chart don't share a color (until more than the palette size are picked).
fn series_color(selected: &[usize], col: usize) -> Rgba {
    let i = selected.iter().position(|&c| c == col).unwrap_or(0);
    PALETTE[i % PALETTE.len()]
}

/// Padded y range for raw data bounds; percentages always include 0..100.
pub(super) fn y_range(lo: f64, hi: f64, unit: Unit) -> (f64, f64) {
    if unit == Unit::Percent {
        return (0.0, hi.max(100.0));
    }
    let pad = ((hi - lo) * 0.08).max(hi.abs() * 0.02).max(1e-6);
    let lo = (lo - pad).max(if lo >= 0.0 { 0.0 } else { f64::NEG_INFINITY });
    (lo, hi + pad)
}

/// Value labels for the five grid lines (bottom to top) and elapsed-time labels for the
/// bottom edge (left to right).
pub(super) fn axis_labels(lo: f64, hi: f64, dur: f64, unit: Unit) -> (Vec<String>, Vec<String>) {
    let ys = (0..=4)
        .map(|k| unit.format(lo + (hi - lo) * k as f64 / 4.0))
        .collect();
    let xs = (0..=4)
        .map(|k| format_elapsed(dur * k as f64 / 4.0))
        .collect();
    (ys, xs)
}

/// Points for one series, downsampled to about two per pixel column (min and max) so long
/// recordings stay fast and spikes stay visible. `x_of` and `y_of` map time and value into
/// the plot (as fractions of it: x from the left, y from the bottom).
pub(super) fn series_points(
    times: &[f64],
    vals: &[f32],
    buckets: usize,
    x_of: impl Fn(f64) -> f32,
    y_of: impl Fn(f64) -> f32,
) -> Vec<Point<f32>> {
    let mut pts = Vec::with_capacity(buckets * 2);
    let n = vals.len();
    let per = n.div_ceil(buckets.max(1)).max(1);
    let mut i = 0;
    while i < n {
        let end = (i + per).min(n);
        let (mut mn, mut mx, mut tmn, mut tmx) = (f32::INFINITY, f32::NEG_INFINITY, 0.0, 0.0);
        for (&v, &t) in vals[i..end].iter().zip(&times[i..end]) {
            if v.is_finite() {
                if v < mn {
                    mn = v;
                    tmn = t;
                }
                if v > mx {
                    mx = v;
                    tmx = t;
                }
            }
        }
        if mn.is_finite() {
            let (a, b) = if tmn <= tmx {
                ((tmn, mn), (tmx, mx))
            } else {
                ((tmx, mx), (tmn, mn))
            };
            pts.push(point(x_of(a.0), y_of(a.1 as f64)));
            if per > 1 {
                pts.push(point(x_of(b.0), y_of(b.1 as f64)));
            }
        }
        i = end;
    }
    pts
}

/// Line chart of the selected columns over the whole recording, with a hover readout.
#[allow(clippy::too_many_arguments)]
fn chart(
    rec: &Recording,
    cols: &[usize],
    selected: &[usize],
    unit: Unit,
    n: usize,
    times: &Rc<Vec<f64>>,
    charts: &ChartHover,
    buckets: usize,
    cx: &mut Context<OmniApp>,
) -> AnyElement {
    let key = SESSION_CHARTS + n;
    let dur = rec.duration_secs().max(1e-6);
    let snap = Snap::Samples {
        times: times.clone(),
        dur,
    };
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for &c in cols {
        if let Some(s) = rec.stats(c) {
            lo = lo.min(s.min);
            hi = hi.max(s.max);
        }
    }
    if !lo.is_finite() {
        return chart_element(Chart::empty(key, snap), charts, cx);
    }
    let (lo, hi) = y_range(lo, hi, unit);
    let (y_labels, x_labels) = axis_labels(lo, hi, dur, unit);
    let x_of = |t: f64| (t / dur) as f32;
    let y_of = |v: f64| ((v - lo) / (hi - lo)) as f32;
    let lines = cols
        .iter()
        .map(|&c| Line {
            pts: series_points(&rec.times, &rec.values[c], buckets, x_of, y_of),
            color: series_color(selected, c),
            width: 1.5,
            dashed: false,
        })
        .collect();

    // Hover: vertical cursor and values at the nearest sample.
    let mut cursor = None;
    let mut readout = Vec::new();
    if let Some(h) = charts.hovered(key) {
        let i = h.at.min(rec.len() - 1);
        cursor = Some(x_of(rec.times[i]));
        readout.push((format_elapsed(rec.times[i]), None));
        for &c in cols {
            let v = rec.values[c][i];
            let text = if v.is_finite() {
                unit.format(v as f64)
            } else {
                "-".into()
            };
            readout.push((
                format!("{}: {text}", rec.columns[c].label),
                Some(series_color(selected, c)),
            ));
        }
    }
    chart_element(
        Chart {
            key,
            lines,
            y_labels,
            x_labels,
            snap,
            cursor,
            readout,
        },
        charts,
        cx,
    )
}

/// Per-sensor summary over the recording: where it started and ended, and its range.
fn stats_table(
    rec: &Recording,
    cols: &[usize],
    selected: &[usize],
    unit: Unit,
    cx: &mut Context<OmniApp>,
) -> Div {
    let mut table =
        ui::Table::new(&["Sensor", "First", "Last", "Change", "Min", "Avg", "Max"]).striped();
    for &c in cols {
        let Some(s) = rec.stats(c) else { continue };
        let f = |v: f64| ui::mono(unit.format(v), cx).into_any_element();
        let d = s.last - s.first;
        let sign = if d > 0.0 { "+" } else { "" };
        table.push_row([
            ui::colored(
                format!("{} · {}", rec.columns[c].group, rec.columns[c].label),
                series_color(selected, c),
                cx,
            )
            .into_any_element(),
            f(s.first),
            f(s.last),
            ui::mono(format!("{sign}{}", unit.format(d)), cx).into_any_element(),
            f(s.min),
            f(s.avg),
            f(s.max),
        ]);
    }
    table.build(cx)
}

// ---- Charts ---------------------------------------------------------------------------------

/// Identifies a chart on the page (for its plot bounds and the hover). Each mode numbers its
/// charts from its own base.
pub(super) type ChartKey = usize;
const SESSION_CHARTS: ChartKey = 0;
pub(super) const COMPARE_CHARTS: ChartKey = 1000;
pub(super) const BOTTLENECK_CHARTS: ChartKey = 2000;

/// Height of a chart and the margins of its plot area (room for the axis labels), in rems.
const CHART_HEIGHT: f32 = 11.25;
pub(super) const PLOT_LEFT: f32 = 4.0;
pub(super) const PLOT_RIGHT: f32 = 0.5;
const PLOT_TOP: f32 = 0.625;
const PLOT_BOTTOM: f32 = 1.25;

/// The sample (or pixel column) the pointer is over in one chart.
#[derive(Clone, Copy, Debug)]
pub(super) struct Hover {
    key: ChartKey,
    /// Sample index ([`Snap::Samples`]) or pixel column ([`Snap::Free`]).
    pub at: usize,
    /// Where the cursor line goes, as a fraction of the plot width.
    pub frac: f32,
}

/// Where each chart's plot area was last drawn (filled in by the canvas, read by the mouse
/// handler) and what the pointer is over.
#[derive(Default)]
pub(super) struct ChartHover {
    hover: Option<Hover>,
    plots: Rc<RefCell<HashMap<ChartKey, Bounds<Pixels>>>>,
}

impl ChartHover {
    /// The hover of chart `key`, if the pointer is over it.
    pub fn hovered(&self, key: ChartKey) -> Option<Hover> {
        self.hover.filter(|h| h.key == key)
    }
}

/// How the pointer position is turned into a [`Hover`].
pub(super) enum Snap {
    /// Snap to the nearest sample at or after the pointer's time (`times` in seconds over a
    /// plot `dur` seconds wide).
    Samples { times: Rc<Vec<f64>>, dur: f64 },
    /// Follow the pointer, one step per pixel column.
    Free,
}

impl Snap {
    fn hover(&self, key: ChartKey, frac: f32, width: Pixels) -> Hover {
        match self {
            Snap::Samples { times, dur } if !times.is_empty() => {
                let t = frac as f64 * dur;
                let i = times.partition_point(|&x| x < t).min(times.len() - 1);
                Hover {
                    key,
                    at: i,
                    frac: (times[i] / dur) as f32,
                }
            }
            _ => Hover {
                key,
                at: (frac * f32::from(width)) as usize,
                frac,
            },
        }
    }
}

/// One series of a chart, its points as fractions of the plot area.
pub(super) struct Line {
    pub pts: Vec<Point<f32>>,
    pub color: Rgba,
    pub width: f32,
    pub dashed: bool,
}

/// Everything a chart shows, worked out before it is drawn.
pub(super) struct Chart {
    pub key: ChartKey,
    pub lines: Vec<Line>,
    /// Labels of the five grid lines, bottom to top (none: no axes).
    pub y_labels: Vec<String>,
    /// Elapsed-time labels along the bottom, left to right.
    pub x_labels: Vec<String>,
    pub snap: Snap,
    /// Hover cursor, as a fraction of the plot width.
    pub cursor: Option<f32>,
    /// Hover read-out: the first line is the time (bold), the rest in their series colour.
    pub readout: Vec<(String, Option<Rgba>)>,
}

impl Chart {
    /// A chart area with no data.
    pub fn empty(key: ChartKey, snap: Snap) -> Chart {
        Chart {
            key,
            lines: Vec::new(),
            y_labels: Vec::new(),
            x_labels: Vec::new(),
            snap,
            cursor: None,
            readout: Vec::new(),
        }
    }
}

/// Stroke `line` scaled into the plot by `at`.
fn paint_line(window: &mut Window, line: &Line, at: impl Fn(Point<f32>) -> Point<Pixels>) {
    if line.pts.len() < 2 {
        return;
    }
    let mut path = PathBuilder::stroke(px(line.width));
    if line.dashed {
        path = path.dash_array(&[px(6.0), px(4.0)]);
    }
    path.move_to(at(line.pts[0]));
    for p in &line.pts[1..] {
        path.line_to(at(*p));
    }
    if let Ok(path) = path.build() {
        window.paint_path(path, line.color);
    }
}

/// A full-width chart: background, grid lines with value labels, time labels along the
/// bottom, the series, and (while hovered) a cursor line with a read-out of the values.
pub(super) fn chart_element(
    chart: Chart,
    charts: &ChartHover,
    cx: &mut Context<OmniApp>,
) -> AnyElement {
    let Chart {
        key,
        lines,
        y_labels,
        x_labels,
        snap,
        cursor,
        readout,
    } = chart;
    let theme = cx.theme();
    let weak: Hsla = theme.muted_foreground;
    let (bg, popover, border, radius) = (theme.muted, theme.popover, theme.border, theme.radius);
    let grid = weak.opacity(0.4);
    let axes = !y_labels.is_empty();

    let plots = charts.plots.clone();
    let plot = canvas(
        move |bounds, _, _| {
            plots.borrow_mut().insert(key, bounds);
        },
        move |b, (), window, _| {
            let (w, h) = (b.size.width, b.size.height);
            if axes {
                for k in 0..=4 {
                    let y = b.bottom() - h * (k as f32 / 4.0);
                    window.paint_quad(fill(
                        Bounds {
                            origin: point(b.left(), y - px(0.25)),
                            size: size(w, px(0.5)),
                        },
                        grid,
                    ));
                }
            }
            let at = |p: Point<f32>| point(b.left() + w * p.x, b.bottom() - h * p.y);
            for line in &lines {
                paint_line(window, line, at);
            }
            if let Some(x) = cursor {
                window.paint_quad(fill(
                    Bounds {
                        origin: point(b.left() + w * x - px(0.5), b.top()),
                        size: size(px(1.0), h),
                    },
                    weak,
                ));
            }
        },
    )
    .absolute()
    .left(rems(PLOT_LEFT))
    .right(rems(PLOT_RIGHT))
    .top(rems(PLOT_TOP))
    .bottom(rems(PLOT_BOTTOM));

    let y_axis = div()
        .absolute()
        .left_0()
        .w(rems(PLOT_LEFT - 0.375))
        .top(rems(PLOT_TOP))
        .bottom(rems(PLOT_BOTTOM))
        .children(y_labels.into_iter().enumerate().map(|(k, text)| {
            h_flex()
                .id(("y", k))
                .absolute()
                .left_0()
                .right_0()
                .top(relative(1.0 - k as f32 / 4.0))
                .mt(rems(-0.5))
                .h(rems(1.0))
                .justify_end()
                .child(ui::label(text).text_xs().text_color(weak))
        }));
    let x_axis = div()
        .absolute()
        .left(rems(PLOT_LEFT))
        .right(rems(PLOT_RIGHT))
        .bottom_0()
        .h(rems(PLOT_BOTTOM))
        .children(x_labels.into_iter().enumerate().map(|(k, text)| {
            let cell = h_flex()
                .id(("x", k))
                .absolute()
                .top(rems(0.125))
                .w(rems(6.0));
            let cell = match k {
                0 => cell.left_0(),
                4 => cell.right_0().justify_end(),
                _ => cell
                    .left(relative(k as f32 / 4.0))
                    .ml(rems(-3.0))
                    .justify_center(),
            };
            cell.child(ui::label(text).text_xs().text_color(weak))
        }));
    let readout = cursor.filter(|_| !readout.is_empty()).map(|x| {
        let rows = readout.into_iter().enumerate().map(|(i, (text, color))| {
            let line = match color {
                Some(c) => ui::colored(text, c, cx),
                None => ui::strong(text),
            };
            div().id(i).child(line)
        });
        let card = v_flex()
            .absolute()
            .top(rems(0.25))
            .px_2()
            .py_1()
            .gap_0p5()
            .bg(popover)
            .border_1()
            .border_color(border)
            .rounded(radius)
            .text_xs()
            .children(rows);
        // Beside the cursor, on the side with more room.
        let card = if x < 0.5 {
            card.left(relative(x)).ml(rems(0.75))
        } else {
            card.right(relative(1.0 - x)).mr(rems(0.75))
        };
        div()
            .id("readout")
            .absolute()
            .left(rems(PLOT_LEFT))
            .right(rems(PLOT_RIGHT))
            .top(rems(PLOT_TOP))
            .bottom(rems(PLOT_BOTTOM))
            .child(card)
    });

    ui::clickable(("chart", key))
        .relative()
        .flex_none()
        .w_full()
        .h(rems(CHART_HEIGHT))
        .rounded(px(4.0))
        .bg(bg)
        .child(plot)
        .child(y_axis)
        .child(x_axis)
        .children(readout)
        .on_mouse_move(cx.listener(move |this, e: &MouseMoveEvent, _, cx| {
            hover_at(this, key, &snap, e.position, cx)
        }))
        .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
            let charts = &mut this.views.recording.charts;
            if !*hovered && charts.hovered(key).is_some() {
                charts.hover = None;
                cx.notify();
            }
        }))
        .into_any_element()
}

/// The pointer moved over chart `key`: redraw only when it is over another sample (or pixel
/// column), or left the plot area.
fn hover_at(
    this: &mut OmniApp,
    key: ChartKey,
    snap: &Snap,
    pos: Point<Pixels>,
    cx: &mut Context<OmniApp>,
) {
    let charts = &mut this.views.recording.charts;
    let Some(b) = charts.plots.borrow().get(&key).copied() else {
        return;
    };
    let frac = (pos.x - b.left()) / b.size.width;
    let new = (0.0..=1.0)
        .contains(&frac)
        .then(|| snap.hover(key, frac, b.size.width));
    let id = |h: Option<Hover>| h.map(|h| (h.key, h.at));
    if id(charts.hover) != id(new) {
        charts.hover = new;
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Tab;
    use crate::settings::Settings;
    use crate::tabs::page_tests::{assert_shows, go, machines, open, snapshot, texts};
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{AnyWindowHandle, TestAppContext};
    use omnihw_core::SystemInfo;
    use omnihw_core::testkit::desktop;

    /// A snapshot with a stopped ten-sample recording of every sensor.
    fn with_recording(info: &SystemInfo) -> Snapshot {
        let mut snap = snapshot(info);
        let mut rec = Recording::new(0.0, &snap.table);
        for i in 0..10 {
            rec.push(i as f64, &snap.table);
        }
        snap.last_recording = Some((rec, "OmniHW-Recording-20261012-012733.csv".into()));
        snap
    }

    /// Open the Recording page; the Compare and Bottleneck modes won't list the real
    /// Documents folder.
    fn open_page(
        cx: &mut TestAppContext,
        info: &SystemInfo,
        snap: Snapshot,
    ) -> (AnyWindowHandle, Entity<OmniApp>) {
        let (window, app) = open(cx, info.clone(), snap, Settings::default());
        go(cx, &app, Tab::Recording);
        cx.update(|cx| app.update(cx, |app, _| app.views.recording.compare.skip_scan()));
        (window, app)
    }

    fn click(cx: &mut TestAppContext, window: AnyWindowHandle, id: &'static str) {
        cx.update_window(window, |_, window, cx| window.click(id, cx))
            .unwrap();
    }

    fn mode(cx: &mut TestAppContext, app: &Entity<OmniApp>) -> Mode {
        cx.read(|cx| app.read(cx).views.recording.mode)
    }

    #[gpui_kit::test]
    fn draws_without_a_recording(cx: &mut TestAppContext) {
        for info in machines() {
            for snap in [Snapshot::default(), snapshot(&info)] {
                let (window, _) = open_page(cx, &info, snap);
                assert_shows(
                    &texts(cx, window),
                    &[
                        "Session",
                        "Compare",
                        "Bottleneck",
                        "Start recording",
                        "Records every sensor at the sampling interval",
                    ],
                );
            }
        }
    }

    #[gpui_kit::test]
    fn mode_buttons_switch_modes(cx: &mut TestAppContext) {
        let info = desktop();
        let (window, app) = open_page(cx, &info, snapshot(&info));
        texts(cx, window);
        click(cx, window, "mode-compare");
        assert_eq!(mode(cx, &app), Mode::Compare);
        assert_shows(
            &texts(cx, window),
            &["Pick two recordings to compare", "Refresh list"],
        );
        click(cx, window, "mode-bottleneck");
        assert_eq!(mode(cx, &app), Mode::Bottleneck);
        assert_shows(
            &texts(cx, window),
            &["Record a gaming session", "Recording", "Start recording"],
        );
        click(cx, window, "mode-session");
        assert_eq!(mode(cx, &app), Mode::Session);
        assert_shows(&texts(cx, window), &["Start recording"]);
    }

    #[gpui_kit::test]
    fn charts_a_recording(cx: &mut TestAppContext) {
        for info in machines() {
            let (window, app) = open_page(cx, &info, with_recording(&info));
            let shown = texts(cx, window);
            assert_shows(
                &shown,
                &[
                    "Last recording: ",
                    "10 samples",
                    "Show CSV",
                    "Open CSV",
                    "Clear",
                    // The default selection is charted, with its stats table and axes.
                    "First",
                    "Change",
                    "0:09",
                ],
            );
            assert!(cx.read(|cx| !app.read(cx).views.recording.selected.is_empty()));

            // Pointing at the first chart shows the values under the pointer.
            cx.update_window(window, |_, window, cx| window.hover(("chart", 0usize), cx))
                .unwrap();
            let hover = cx.read(|cx| app.read(cx).views.recording.charts.hover);
            let hover = hover.expect("hovering the chart sets the hover sample");
            assert_eq!(hover.key, 0);
            assert!(hover.at < 10);
            assert!(texts(cx, window).len() > shown.len(), "a read-out appears");

            // Clear empties the selection.
            cx.update_window(window, |_, window, cx| {
                window.click(("rec_picker", usize::MAX), cx)
            })
            .unwrap();
            assert!(cx.read(|cx| app.read(cx).views.recording.selected.is_empty()));
            assert_shows(&texts(cx, window), &["Select sensors on the left"]);
        }
    }

    #[gpui_kit::test]
    fn compares_and_analyzes_a_recording(cx: &mut TestAppContext) {
        let info = desktop();
        let (window, app) = open_page(cx, &info, with_recording(&info));
        cx.update(|cx| {
            app.update(cx, |app, _| {
                app.views.recording.mode = Mode::Compare;
                app.views.recording.compare.compare_memory();
            })
        });
        assert_shows(
            &texts(cx, window),
            &["A (solid): Last recording", "Avg A", "Δ Avg", "Max B"],
        );
        click(cx, window, "mode-bottleneck");
        // The in-memory recording is picked; it is analyzed or says why it can't be.
        let shown = texts(cx, window);
        assert_shows(&shown, &["Last recording (0:09)"]);
    }

    #[test]
    fn y_ranges() {
        assert_eq!(y_range(10.0, 50.0, Unit::Percent), (0.0, 100.0));
        assert_eq!(y_range(10.0, 120.0, Unit::Percent), (0.0, 120.0));
        let (lo, hi) = y_range(40.0, 60.0, Unit::Celsius);
        assert!((0.0..40.0).contains(&lo) && hi > 60.0);
        // Positive data never gets a negative axis; flat data still gets a range.
        let (lo, hi) = y_range(0.5, 0.5, Unit::Watt);
        assert!(lo >= 0.0 && hi > lo);
        let (lo, _) = y_range(-5.0, 5.0, Unit::Celsius);
        assert!(lo < -5.0);
    }

    #[test]
    fn downsampling_keeps_spikes() {
        let times: Vec<f64> = (0..100).map(|i| i as f64).collect();
        let mut vals = vec![1.0f32; 100];
        vals[42] = 9.0;
        vals[10] = f32::NAN;
        let pts = series_points(&times, &vals, 10, |t| t as f32, |v| v as f32);
        // Two points (min and max) per bucket of ten samples.
        assert_eq!(pts.len(), 20);
        assert!(pts.iter().any(|p| p.y == 9.0 && p.x == 42.0));
        // One point per sample when there is room for all.
        let pts = series_points(&times, &vals, 200, |t| t as f32, |v| v as f32);
        assert_eq!(pts.len(), 99, "the NaN sample is skipped");
    }

    #[test]
    fn colors_and_axes() {
        assert_eq!(series_color(&[3, 7], 7), PALETTE[1]);
        assert_eq!(series_color(&[3, 7], 5), PALETTE[0]);
        let (ys, xs) = axis_labels(0.0, 100.0, 60.0, Unit::Percent);
        assert_eq!(ys.len(), 5);
        assert_eq!(xs.first().map(String::as_str), Some("0:00"));
        assert_eq!(xs.last().map(String::as_str), Some("1:00"));
    }
}
