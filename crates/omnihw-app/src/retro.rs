//! The Retro theme: an early-2000s desktop look (Windows 2000 / XP "classic" controls).
//!
//! Most of it is plain egui styling ([`style`]): beige surfaces, black text, square corners, no
//! shadows, chunky scroll bars and a navy selection colour. egui can't draw two-tone 3D edges
//! by itself, so the widget visuals use a few *marker* stroke colours and the [`Bevels`] plugin
//! rewrites the finished frame's shapes once per painted frame: every rectangle stroked with a
//! marker gets a classic raised, sunken or etched edge (crisp, pixel-aligned quads), and other
//! rectangles get their corners squared off. Nothing runs when Retro isn't selected, and the
//! pass only happens when egui actually paints, so idle CPU is unaffected.
//!
//! The font is Tahoma, loaded from `C:\Windows\Fonts` the first time Retro is applied; if it is
//! missing or unreadable the default font is used. Nothing is downloaded.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use egui::epaint::{ClippedShape, Mesh, RectShape, Shadow, Shape, Vertex, WHITE_UV};
use egui::style::{HandleShape, ScrollStyle, WidgetVisuals, Widgets};
use egui::{
    Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Rect, Stroke, TextStyle,
    Visuals, pos2, vec2,
};

// ---- Palette (the Windows XP "classic" system colours) ----

/// Window, panel and button face.
pub const FACE: Color32 = Color32::from_rgb(236, 233, 216);
/// Button face under the pointer.
pub const FACE_HOT: Color32 = Color32::from_rgb(244, 242, 233);
/// Button face while pressed or open.
pub const FACE_DOWN: Color32 = Color32::from_rgb(224, 220, 204);
/// Brightest bevel edge (3D highlight).
pub const HIGHLIGHT: Color32 = Color32::WHITE;
/// Light bevel edge (3D light).
pub const LIGHT: Color32 = Color32::from_rgb(241, 239, 226);
/// Mid bevel edge (3D shadow), also used for etched lines.
pub const SHADOW: Color32 = Color32::from_rgb(172, 168, 153);
/// Darkest bevel edge (3D dark shadow).
pub const DARK_SHADOW: Color32 = Color32::from_rgb(113, 111, 100);
/// Text fields, check boxes, charts and other sunken "wells".
pub const FIELD: Color32 = Color32::WHITE;
/// Scroll bar track.
pub const TRACK: Color32 = Color32::from_rgb(246, 245, 238);
/// Softly lighter surface for cards and striped rows.
pub const FAINT: Color32 = Color32::from_rgb(246, 244, 236);
/// Normal text.
pub const TEXT: Color32 = Color32::from_rgb(28, 28, 24);
/// Strong text and text on pressed widgets.
pub const STRONG_TEXT: Color32 = Color32::BLACK;
/// Secondary ("weak") text, still readable on [`FACE`].
pub const WEAK_TEXT: Color32 = Color32::from_rgb(96, 94, 86);
/// Selection highlight (selected buttons, selected text, slider fill).
pub const SELECTION: Color32 = Color32::from_rgb(49, 106, 197);
/// Top of the title-bar style gradient painted on selected buttons.
pub const SELECTION_TOP: Color32 = Color32::from_rgb(82, 141, 229);
/// Bottom of the selected-button gradient.
pub const SELECTION_BOTTOM: Color32 = Color32::from_rgb(28, 76, 168);
/// Hyperlinks.
pub const LINK: Color32 = Color32::from_rgb(0, 70, 190);
/// Warning text.
pub const WARN: Color32 = Color32::from_rgb(176, 88, 0);
/// Error text.
pub const ERROR: Color32 = Color32::from_rgb(190, 0, 0);

// ---- Markers: stroke/fill colours the bevel pass recognises ----
// Each is within one step of a real palette colour, so a shape the pass doesn't touch (e.g. a
// faded, disabled widget) still looks reasonable.

/// Stroke of raised widgets (buttons, windows, menus).
const RAISED_MARK: Color32 = Color32::from_rgb(113, 111, 101);
/// Stroke of pressed / open widgets.
const SUNKEN_MARK: Color32 = Color32::from_rgb(113, 111, 102);
/// Stroke of group frames and separators.
const ETCH_MARK: Color32 = Color32::from_rgb(172, 168, 154);
/// `bg_fill` of widgets: check boxes, scroll bar thumbs, slider rails and handles.
const KNOB: Color32 = Color32::from_rgb(236, 233, 217);
const KNOB_HOT: Color32 = Color32::from_rgb(244, 242, 234);
const KNOB_DOWN: Color32 = Color32::from_rgb(224, 220, 205);

/// Width of the (always visible) scroll bars, in points.
const SCROLL_BAR_WIDTH: f32 = 16.0;
/// Rails thinner than this (slider tracks) are drawn as sunken channels.
const RAIL_MAX: f32 = 6.0;
/// Corners of other rectangles are rounded at most this much.
const MAX_RADIUS: u8 = 2;

/// Font family name the Retro text styles use (Tahoma first, then egui's defaults).
const FAMILY: &str = "omnihw-retro";
/// Font data key for Tahoma.
const TAHOMA_KEY: &str = "omnihw-tahoma";
/// Where Windows keeps Tahoma.
const TAHOMA_PATH: &str = r"C:\Windows\Fonts\tahoma.ttf";

/// Tahoma has been looked for (it is read at most once per run).
static TAHOMA_TRIED: AtomicBool = AtomicBool::new(false);

/// Register the bevel plugin and the Retro font family. Call once at startup, before the first
/// frame, so the font family exists before any Retro style can refer to it.
pub fn install(ctx: &egui::Context) {
    ctx.add_plugin(Bevels::default());
    ctx.set_fonts(font_definitions(None));
}

/// Switch the Retro look on or off. On: the light style becomes [`style`] (the caller selects
/// the light theme). Off: egui's own light style is restored.
pub fn set_enabled(ctx: &egui::Context, on: bool) {
    let was_on = ctx.with_plugin(|b: &mut Bevels| std::mem::replace(&mut b.enabled, on));
    if on {
        if !TAHOMA_TRIED.swap(true, Ordering::Relaxed)
            && let Some(data) = load_tahoma()
        {
            ctx.set_fonts(font_definitions(Some(data)));
        }
        ctx.set_style_of(egui::Theme::Light, style());
    } else if was_on == Some(true) {
        ctx.set_style_of(egui::Theme::Light, egui::Theme::Light.default_style());
    }
}

/// The complete Retro style (visuals, spacing and text styles).
pub fn style() -> egui::Style {
    let mut style = egui::Theme::Light.default_style();
    style.visuals = visuals();

    let family = FontFamily::Name(FAMILY.into());
    style.text_styles = [
        (TextStyle::Small, FontId::new(10.0, family.clone())),
        (TextStyle::Body, FontId::new(12.5, family.clone())),
        (TextStyle::Button, FontId::new(12.5, family.clone())),
        (TextStyle::Heading, FontId::new(17.0, family)),
        (TextStyle::Monospace, FontId::monospace(12.0)),
    ]
    .into();

    let s = &mut style.spacing;
    s.button_padding = vec2(8.0, 3.0);
    s.interact_size = vec2(40.0, 20.0);
    // Room for the two-pixel sunken edge around the check mark.
    s.icon_width = 14.0;
    s.icon_width_inner = 7.0;
    s.slider_rail_height = 4.0;
    s.scroll = ScrollStyle {
        bar_width: SCROLL_BAR_WIDTH,
        bar_inner_margin: 2.0,
        handle_min_length: SCROLL_BAR_WIDTH,
        ..ScrollStyle::solid()
    };
    style
}

/// Retro colours and shapes. A light theme, so text helpers like `heat::ink` darken status
/// colours for readability.
pub fn visuals() -> Visuals {
    let widget = |face, knob, edge, text, width| WidgetVisuals {
        weak_bg_fill: face,
        bg_fill: knob,
        bg_stroke: Stroke::new(1.0, edge),
        corner_radius: CornerRadius::ZERO,
        fg_stroke: Stroke::new(width, text),
        expansion: 0.0,
    };
    let mut v = Visuals::light();
    v.widgets = Widgets {
        noninteractive: widget(FACE, FACE, ETCH_MARK, TEXT, 1.0),
        inactive: widget(FACE, KNOB, RAISED_MARK, TEXT, 1.0),
        hovered: widget(FACE_HOT, KNOB_HOT, RAISED_MARK, STRONG_TEXT, 1.5),
        active: widget(FACE_DOWN, KNOB_DOWN, SUNKEN_MARK, STRONG_TEXT, 1.5),
        open: widget(FACE_DOWN, KNOB_DOWN, SUNKEN_MARK, STRONG_TEXT, 1.0),
    };
    v.selection.bg_fill = SELECTION;
    v.selection.stroke = Stroke::new(1.0, Color32::WHITE);
    v.weak_text_color = Some(WEAK_TEXT);
    v.hyperlink_color = LINK;
    v.faint_bg_color = FAINT;
    v.extreme_bg_color = FIELD;
    v.text_edit_bg_color = None;
    v.code_bg_color = FAINT;
    v.warn_fg_color = WARN;
    v.error_fg_color = ERROR;
    v.window_corner_radius = CornerRadius::ZERO;
    v.menu_corner_radius = CornerRadius::ZERO;
    v.window_shadow = Shadow::NONE;
    v.popup_shadow = Shadow::NONE;
    v.window_fill = FACE;
    v.window_stroke = Stroke::new(1.0, RAISED_MARK);
    v.window_highlight_topmost = false;
    v.panel_fill = FACE;
    v.text_cursor.stroke = Stroke::new(1.0, STRONG_TEXT);
    v.handle_shape = HandleShape::Rect { aspect_ratio: 0.5 };
    v.slider_trailing_fill = false;
    v
}

/// egui's default fonts plus the Retro family; Tahoma leads it when given.
fn font_definitions(tahoma: Option<Vec<u8>>) -> FontDefinitions {
    let mut defs = FontDefinitions::default();
    let mut family = defs
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    if let Some(data) = tahoma {
        defs.font_data
            .insert(TAHOMA_KEY.into(), Arc::new(FontData::from_owned(data)));
        family.insert(0, TAHOMA_KEY.into());
    }
    defs.families
        .insert(FontFamily::Name(FAMILY.into()), family);
    defs
}

/// Tahoma from the Windows fonts folder, if present and well-formed (egui panics on a font it
/// can't parse, so the file is sanity-checked first).
fn load_tahoma() -> Option<Vec<u8>> {
    let data = std::fs::read(TAHOMA_PATH).ok()?;
    looks_like_font(&data).then_some(data)
}

/// Structural check of a TrueType/OpenType file: known signature, a table directory that fits
/// in the file, and the tables a text renderer needs.
fn looks_like_font(data: &[u8]) -> bool {
    const REQUIRED: [&[u8; 4]; 5] = [b"cmap", b"head", b"hhea", b"hmtx", b"maxp"];
    let be16 = |i: usize| data.get(i..i + 2).map(|b| u16::from_be_bytes([b[0], b[1]]));
    let be32 = |i: usize| {
        data.get(i..i + 4)
            .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize)
    };
    let Some(signature) = data.get(0..4) else {
        return false;
    };
    if ![&[0, 1, 0, 0][..], b"true", b"OTTO"].contains(&signature) {
        return false;
    }
    let Some(count) = be16(4) else {
        return false;
    };
    let mut found = 0;
    for i in 0..usize::from(count) {
        let rec = 12 + 16 * i;
        let (Some(tag), Some(offset), Some(len)) =
            (data.get(rec..rec + 4), be32(rec + 8), be32(rec + 12))
        else {
            return false;
        };
        if offset.checked_add(len).is_none_or(|end| end > data.len()) {
            return false;
        }
        if REQUIRED.iter().any(|t| &t[..] == tag) {
            found += 1;
        }
    }
    found == REQUIRED.len()
}

// ---- The bevel pass ----

/// Output hook that turns marker-stroked rectangles into classic 3D edges.
#[derive(Default)]
pub struct Bevels {
    enabled: bool,
    /// Reused shape buffer, so the pass doesn't allocate a new list every frame.
    scratch: Vec<ClippedShape>,
}

impl egui::Plugin for Bevels {
    fn debug_name(&self) -> &'static str {
        "omnihw_retro_bevels"
    }

    fn output_hook(&mut self, _ctx: &egui::Context, output: &mut egui::FullOutput) {
        if !self.enabled {
            return;
        }
        let mut out = std::mem::take(&mut self.scratch);
        out.clear();
        out.reserve(output.shapes.len() + output.shapes.len() / 4);
        let mut pass = Pass::new(output.pixels_per_point);
        for ClippedShape { clip_rect, shape } in output.shapes.drain(..) {
            pass.restyle(shape, &mut |shape| {
                out.push(ClippedShape { clip_rect, shape })
            });
        }
        self.scratch = std::mem::replace(&mut output.shapes, out);
    }
}

/// How a rectangle's edge is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Edge {
    /// Button, window, menu, scroll thumb.
    Raised,
    /// Pressed or selected button, text field, check box, chart well.
    Sunken,
    /// Group frame.
    Etched,
}

/// What to do with one rectangle.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Look {
    edge: Edge,
    /// Replacement fill, if any.
    fill: Option<Color32>,
    /// Paint the selected-button gradient instead of the fill.
    gradient: bool,
    /// A sunken well; a bar drawn right after it on the same rect is kept inside the edge.
    well: bool,
}

impl Look {
    fn new(edge: Edge) -> Self {
        Look {
            edge,
            fill: None,
            gradient: false,
            well: false,
        }
    }
}

/// Decide how a rectangle painted with the Retro visuals should look; `None` leaves it as is
/// (apart from squared corners).
fn classify(r: &RectShape) -> Option<Look> {
    if r.brush.is_some() || r.blur_width > 0.0 {
        return None;
    }
    let stroked = r.stroke.width > 0.0;
    let marker = |c| stroked && r.stroke.color == c;
    let knob = [KNOB, KNOB_HOT, KNOB_DOWN].contains(&r.fill);
    let size = r.rect.size();
    if marker(ETCH_MARK) {
        return Some(Look::new(Edge::Etched));
    }
    if r.fill == SELECTION && stroked {
        // A selected button: pressed in, with the title-bar gradient.
        return Some(Look {
            gradient: true,
            ..Look::new(Edge::Sunken)
        });
    }
    if knob {
        let field = Look {
            fill: Some(FIELD),
            ..Look::new(Edge::Sunken)
        };
        // Check boxes (marker stroke) and slider rails (thin) are white and sunken.
        return Some(
            if marker(RAISED_MARK) || marker(SUNKEN_MARK) || size.min_elem() <= RAIL_MAX {
                field
            } else {
                Look::new(Edge::Raised) // scroll bar thumb, slider handle
            },
        );
    }
    if r.fill == FIELD {
        if stroked {
            return Some(Look::new(Edge::Sunken)); // text field (focused or not), canvas
        }
        let across = size.min_elem();
        if (across - SCROLL_BAR_WIDTH).abs() < 0.5 && size.max_elem() >= 2.0 * SCROLL_BAR_WIDTH {
            return None; // scroll bar track: recoloured by the caller
        }
        return Some(Look {
            well: true,
            ..Look::new(Edge::Sunken)
        }); // chart, progress bar or meter background
    }
    if marker(SUNKEN_MARK) {
        return Some(Look::new(Edge::Sunken));
    }
    if marker(RAISED_MARK) {
        return Some(Look::new(if r.fill == Color32::TRANSPARENT {
            Edge::Sunken // read-only text field
        } else {
            Edge::Raised
        }));
    }
    None
}

/// Per-frame state of the bevel pass.
struct Pass {
    ppp: f32,
    /// Edge thickness in points: one physical pixel per layer, two at high DPI.
    t: f32,
    /// The well painted by the previous rectangle, for keeping its bar inside the edge.
    last_well: Option<Rect>,
}

impl Pass {
    fn new(pixels_per_point: f32) -> Self {
        let ppp = if pixels_per_point.is_finite() && pixels_per_point > 0.0 {
            pixels_per_point
        } else {
            1.0
        };
        Pass {
            ppp,
            t: ppp.round().max(1.0) / ppp,
            last_well: None,
        }
    }

    fn snap(&self, x: f32) -> f32 {
        (x * self.ppp).round() / self.ppp
    }

    fn snap_rect(&self, r: Rect) -> Rect {
        Rect::from_min_max(
            pos2(self.snap(r.min.x), self.snap(r.min.y)),
            pos2(self.snap(r.max.x), self.snap(r.max.y)),
        )
    }

    /// Restyle one shape, handing the result (possibly several shapes) to `emit` in paint order.
    fn restyle(&mut self, shape: Shape, emit: &mut impl FnMut(Shape)) {
        match shape {
            Shape::Rect(r) => self.rect(r, emit),
            Shape::LineSegment { points, stroke } if stroke.color == ETCH_MARK => {
                self.last_well = None;
                emit(self.etched_line(points, stroke));
            }
            Shape::Vec(shapes) => {
                for s in shapes {
                    self.restyle(s, emit);
                }
            }
            Shape::Noop => {}
            other => {
                self.last_well = None;
                emit(other);
            }
        }
    }

    fn rect(&mut self, mut r: RectShape, emit: &mut impl FnMut(Shape)) {
        let well = self.last_well.take();
        let Some(look) = classify(&r) else {
            if r.fill == FIELD && r.stroke.width <= 0.0 {
                r.fill = TRACK; // scroll bar track
            }
            r.corner_radius = clamp_radius(r.corner_radius);
            // A bar drawn over the well just painted (progress bar, meter) stays inside its edge.
            if let Some(w) = well
                && r.stroke.width <= 0.0
                && (r.rect.min - w.min).length() < 0.5
                && (r.rect.height() - w.height()).abs() < 0.5
            {
                r.rect = r.rect.intersect(w.shrink(2.0 * self.t));
            }
            emit(Shape::Rect(r));
            return;
        };
        let rect = self.snap_rect(r.rect);
        if let Some(fill) = look.fill {
            r.fill = fill;
        }
        let mut mesh = Mesh::default();
        if look.gradient {
            gradient(&mut mesh, rect, SELECTION_TOP, SELECTION_BOTTOM);
            r.fill = Color32::TRANSPARENT;
        }
        r.rect = rect;
        r.corner_radius = CornerRadius::ZERO;
        r.stroke = Stroke::NONE;
        if r.fill != Color32::TRANSPARENT {
            emit(Shape::Rect(r));
        }
        self.bevel(&mut mesh, rect, look.edge);
        emit(Shape::mesh(mesh));
        if look.well {
            self.last_well = Some(rect);
        }
    }

    /// Two one-pixel rings: outer then inner, each with a top-left and a bottom-right colour.
    fn bevel(&self, mesh: &mut Mesh, rect: Rect, edge: Edge) {
        let (outer, inner) = match edge {
            Edge::Raised => ((LIGHT, DARK_SHADOW), (HIGHLIGHT, SHADOW)),
            Edge::Sunken => ((SHADOW, HIGHLIGHT), (DARK_SHADOW, LIGHT)),
            Edge::Etched => ((SHADOW, HIGHLIGHT), (HIGHLIGHT, SHADOW)),
        };
        let t = self.t;
        ring(mesh, rect, t, outer.0, outer.1);
        if rect.width() > 4.0 * t && rect.height() > 4.0 * t {
            ring(mesh, rect.shrink(t), t, inner.0, inner.1);
        }
    }

    /// A separator: a shadow line with a highlight line beside it.
    fn etched_line(&self, [a, b]: [egui::Pos2; 2], stroke: Stroke) -> Shape {
        let t = self.t;
        let mut mesh = Mesh::default();
        if (a.y - b.y).abs() < 0.01 {
            let y = self.snap(a.y - t / 2.0);
            let (x0, x1) = (self.snap(a.x.min(b.x)), self.snap(a.x.max(b.x)));
            mesh.add_colored_rect(Rect::from_x_y_ranges(x0..=x1, y..=y + t), SHADOW);
            mesh.add_colored_rect(
                Rect::from_x_y_ranges(x0..=x1, y + t..=y + 2.0 * t),
                HIGHLIGHT,
            );
        } else if (a.x - b.x).abs() < 0.01 {
            let x = self.snap(a.x - t / 2.0);
            let (y0, y1) = (self.snap(a.y.min(b.y)), self.snap(a.y.max(b.y)));
            mesh.add_colored_rect(Rect::from_x_y_ranges(x..=x + t, y0..=y1), SHADOW);
            mesh.add_colored_rect(
                Rect::from_x_y_ranges(x + t..=x + 2.0 * t, y0..=y1),
                HIGHLIGHT,
            );
        } else {
            return Shape::LineSegment {
                points: [a, b],
                stroke: Stroke::new(stroke.width, SHADOW),
            };
        }
        Shape::mesh(mesh)
    }
}

/// Square corners off to at most [`MAX_RADIUS`].
fn clamp_radius(c: CornerRadius) -> CornerRadius {
    CornerRadius {
        nw: c.nw.min(MAX_RADIUS),
        ne: c.ne.min(MAX_RADIUS),
        sw: c.sw.min(MAX_RADIUS),
        se: c.se.min(MAX_RADIUS),
    }
}

/// A `t`-thick ring just inside `r`: top and left edges in `tl`, bottom and right in `br`
/// (the bottom-right colour owns the two shared corners, as in Windows' `DrawEdge`).
fn ring(mesh: &mut Mesh, r: Rect, t: f32, tl: Color32, br: Color32) {
    let (min, max) = (r.min, r.max);
    mesh.add_colored_rect(Rect::from_min_max(min, pos2(max.x - t, min.y + t)), tl);
    mesh.add_colored_rect(
        Rect::from_min_max(pos2(min.x, min.y + t), pos2(min.x + t, max.y - t)),
        tl,
    );
    mesh.add_colored_rect(Rect::from_min_max(pos2(min.x, max.y - t), max), br);
    mesh.add_colored_rect(
        Rect::from_min_max(pos2(max.x - t, min.y), pos2(max.x, max.y - t)),
        br,
    );
}

/// A vertical gradient filling `r`.
fn gradient(mesh: &mut Mesh, r: Rect, top: Color32, bottom: Color32) {
    let base = mesh.vertices.len() as u32;
    for (pos, color) in [
        (r.left_top(), top),
        (r.right_top(), top),
        (r.left_bottom(), bottom),
        (r.right_bottom(), bottom),
    ] {
        mesh.vertices.push(Vertex {
            pos,
            uv: WHITE_UV,
            color,
        });
    }
    mesh.indices
        .extend([base, base + 1, base + 2, base + 1, base + 3, base + 2]);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect_shape(fill: Color32, stroke: Stroke) -> RectShape {
        RectShape::new(
            Rect::from_min_size(pos2(10.0, 10.0), vec2(80.0, 24.0)),
            CornerRadius::same(6),
            fill,
            stroke,
            egui::StrokeKind::Inside,
        )
    }

    fn edge(fill: Color32, stroke: Color32) -> Option<Edge> {
        classify(&rect_shape(fill, Stroke::new(1.0, stroke))).map(|l| l.edge)
    }

    #[test]
    fn widgets_get_classic_edges() {
        let v = visuals();
        let w = &v.widgets;
        // Buttons are raised, pressed ones sunken, group frames etched.
        assert_eq!(
            edge(w.inactive.weak_bg_fill, w.inactive.bg_stroke.color),
            Some(Edge::Raised)
        );
        assert_eq!(
            edge(w.hovered.weak_bg_fill, w.hovered.bg_stroke.color),
            Some(Edge::Raised)
        );
        assert_eq!(
            edge(w.active.weak_bg_fill, w.active.bg_stroke.color),
            Some(Edge::Sunken)
        );
        assert_eq!(
            edge(Color32::TRANSPARENT, w.noninteractive.bg_stroke.color),
            Some(Edge::Etched)
        );
        // Custom-coloured buttons (e.g. the red alerts button) are raised too.
        assert_eq!(
            edge(crate::heat::RED, w.inactive.bg_stroke.color),
            Some(Edge::Raised)
        );
        // Selected buttons are pressed in with the gradient.
        let sel = classify(&rect_shape(v.selection.bg_fill, w.inactive.bg_stroke));
        assert!(sel.is_some_and(|l| l.gradient && l.edge == Edge::Sunken));
        // Check boxes become white sunken boxes.
        let check = classify(&rect_shape(w.inactive.bg_fill, w.inactive.bg_stroke));
        assert_eq!(check.and_then(|l| l.fill), Some(FIELD));
        // Text fields and chart wells are sunken.
        assert_eq!(
            edge(v.extreme_bg_color, w.inactive.bg_stroke.color),
            Some(Edge::Sunken)
        );
        let well = classify(&rect_shape(v.extreme_bg_color, Stroke::NONE));
        assert!(well.is_some_and(|l| l.well));
        // Ordinary painted rectangles (cards, pills, tiles) are left alone.
        assert_eq!(classify(&rect_shape(FAINT, Stroke::NONE)), None);
        assert_eq!(
            classify(&rect_shape(crate::heat::GREEN, Stroke::NONE)),
            None
        );
    }

    #[test]
    fn scroll_parts() {
        let track = RectShape::filled(
            Rect::from_min_size(pos2(0.0, 0.0), vec2(SCROLL_BAR_WIDTH, 300.0)),
            0,
            FIELD,
        );
        assert_eq!(classify(&track), None, "track is recoloured, not beveled");
        let thumb = RectShape::filled(
            Rect::from_min_size(pos2(0.0, 0.0), vec2(SCROLL_BAR_WIDTH, 60.0)),
            0,
            KNOB,
        );
        assert_eq!(classify(&thumb).map(|l| l.edge), Some(Edge::Raised));
        let rail = RectShape::filled(
            Rect::from_min_size(pos2(0.0, 0.0), vec2(120.0, 4.0)),
            2,
            KNOB,
        );
        assert_eq!(classify(&rail).and_then(|l| l.fill), Some(FIELD));
    }

    #[test]
    fn pass_bevels_and_squares() {
        let v = visuals();
        let mut pass = Pass::new(1.0);
        let mut out = Vec::new();
        let button = rect_shape(FACE, v.widgets.inactive.bg_stroke);
        pass.restyle(Shape::Rect(button), &mut |s| out.push(s));
        assert_eq!(out.len(), 2, "face + bevel mesh");
        match (&out[0], &out[1]) {
            (Shape::Rect(r), Shape::Mesh(m)) => {
                assert_eq!(r.corner_radius, CornerRadius::ZERO);
                assert_eq!(r.stroke, Stroke::NONE);
                assert_eq!(m.vertices.len(), 2 * 4 * 4, "two rings of four quads");
            }
            other => panic!("unexpected shapes {other:?}"),
        }
        out.clear();
        pass.restyle(Shape::Rect(rect_shape(FAINT, Stroke::NONE)), &mut |s| {
            out.push(s)
        });
        match &out[..] {
            [Shape::Rect(r)] => assert_eq!(r.corner_radius, CornerRadius::same(MAX_RADIUS)),
            other => panic!("unexpected shapes {other:?}"),
        }
    }

    #[test]
    fn progress_bar_stays_inside_its_well() {
        let mut pass = Pass::new(1.0);
        let mut out = Vec::new();
        let r = Rect::from_min_size(pos2(0.0, 0.0), vec2(200.0, 18.0));
        pass.restyle(Shape::Rect(RectShape::filled(r, 9, FIELD)), &mut |s| {
            out.push(s)
        });
        let bar = Rect::from_min_size(r.min, vec2(100.0, 18.0));
        pass.restyle(
            Shape::Rect(RectShape::filled(bar, 9, SELECTION)),
            &mut |s| out.push(s),
        );
        match out.last() {
            Some(Shape::Rect(b)) => assert_eq!(
                b.rect,
                Rect::from_min_max(pos2(2.0, 2.0), pos2(100.0, 16.0))
            ),
            other => panic!("unexpected shape {other:?}"),
        }
    }

    #[test]
    fn separators_are_etched() {
        let pass = Pass::new(1.0);
        let line = pass.etched_line(
            [pos2(0.0, 10.0), pos2(50.0, 10.0)],
            Stroke::new(1.0, ETCH_MARK),
        );
        assert!(matches!(line, Shape::Mesh(m) if m.vertices.len() == 8));
    }

    #[test]
    fn font_sanity_check() {
        assert!(!looks_like_font(b""));
        assert!(!looks_like_font(b"not a font at all, just text"));
        // A minimal table directory with the required tables (empty, but in bounds).
        let tags = [b"cmap", b"head", b"hhea", b"hmtx", b"maxp"];
        let mut f = vec![0, 1, 0, 0, 0, tags.len() as u8, 0, 0, 0, 0, 0, 0];
        for tag in tags {
            f.extend_from_slice(tag);
            f.extend_from_slice(&[0; 4]); // checksum
            f.extend_from_slice(&12u32.to_be_bytes()); // offset
            f.extend_from_slice(&0u32.to_be_bytes()); // length
        }
        assert!(looks_like_font(&f));
        // A table that runs past the end of the file is rejected.
        let n = f.len();
        f[n - 4..].copy_from_slice(&1_000_000u32.to_be_bytes());
        assert!(!looks_like_font(&f));
        // Retro font family is always registered.
        let defs = font_definitions(None);
        assert!(defs.families.contains_key(&FontFamily::Name(FAMILY.into())));
    }
}
