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

mod bevels;
mod font;

use bevels::Bevels;
use font::{FAMILY, TAHOMA_TRIED, font_definitions, load_tahoma};
use std::sync::atomic::Ordering;

use egui::epaint::Shadow;
use egui::style::{HandleShape, ScrollStyle, WidgetVisuals, Widgets};
use egui::{
    Color32, CornerRadius, FontFamily, FontId, Rect, Stroke, TextStyle, Visuals, pos2, vec2,
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
