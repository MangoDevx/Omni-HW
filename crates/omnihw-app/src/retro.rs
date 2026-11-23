//! The Retro theme: an early-2000s desktop look (Windows 2000 / XP "classic" controls).
//!
//! It is the light theme with its colours replaced by the classic system palette: beige
//! surfaces, black text, square corners, no shadows and a navy selection colour, set in Tahoma
//! (a font Windows ships; if it is missing the system UI font is used). Nothing is downloaded.

use crate::heat::rgb8;
use gpui_kit::component::Theme;
use gpui_kit::{Hsla, Rgba, px};

// ---- Palette (the Windows XP "classic" system colours) ----

/// Window, panel and button face.
pub const FACE: Rgba = rgb8(236, 233, 216);
/// Button face under the pointer.
pub const FACE_HOT: Rgba = rgb8(244, 242, 233);
/// Button face while pressed or open.
pub const FACE_DOWN: Rgba = rgb8(224, 220, 204);
/// Mid bevel edge (3D shadow), also used for borders and etched lines.
pub const SHADOW: Rgba = rgb8(172, 168, 153);
/// Darkest bevel edge (3D dark shadow): text field and button outlines.
pub const DARK_SHADOW: Rgba = rgb8(113, 111, 100);
/// Text fields, check boxes, charts and other sunken "wells".
pub const FIELD: Rgba = rgb8(255, 255, 255);
/// Scroll bar track.
pub const TRACK: Rgba = rgb8(246, 245, 238);
/// Softly lighter surface for cards and striped rows.
pub const FAINT: Rgba = rgb8(246, 244, 236);
/// Navigation sidebar: a shade darker than the face.
pub const SIDEBAR: Rgba = rgb8(226, 222, 204);
/// Normal text.
pub const TEXT: Rgba = rgb8(28, 28, 24);
/// Strong text and text on pressed widgets.
pub const STRONG_TEXT: Rgba = rgb8(0, 0, 0);
/// Secondary ("weak") text, still readable on [`FACE`].
pub const WEAK_TEXT: Rgba = rgb8(96, 94, 86);
/// Selection highlight (selected buttons, selected text, slider fill).
pub const SELECTION: Rgba = rgb8(49, 106, 197);
/// Selection under the pointer.
pub const SELECTION_HOT: Rgba = rgb8(82, 141, 229);
/// Selection while pressed.
pub const SELECTION_DOWN: Rgba = rgb8(28, 76, 168);
/// Hyperlinks.
pub const LINK: Rgba = rgb8(0, 70, 190);
/// Warning text.
pub const WARN: Rgba = rgb8(176, 88, 0);
/// Error text.
pub const ERROR: Rgba = rgb8(190, 0, 0);

/// Font the Retro theme uses.
pub const FONT: &str = "Tahoma";
/// Retro body text size at 100 % UI scale, in points.
pub const FONT_SIZE: f32 = 13.0;

/// Replace the (light) theme's colours and shapes with the Retro look.
pub fn apply(t: &mut Theme) {
    let c = |c: Rgba| Hsla::from(c);
    let white = c(FIELD);

    t.background = c(FACE);
    t.foreground = c(TEXT);
    t.border = c(SHADOW);
    t.muted = c(FAINT);
    t.muted_foreground = c(WEAK_TEXT);
    t.accent = c(FACE_HOT);
    t.accent_foreground = c(STRONG_TEXT);
    t.group_box = c(FACE);
    t.group_box_foreground = c(TEXT);
    t.popover = c(FACE);
    t.popover_foreground = c(TEXT);
    t.input = c(DARK_SHADOW);
    t.ring = c(SELECTION);
    t.caret = c(STRONG_TEXT);
    t.selection = c(SELECTION).opacity(0.35);
    t.overlay = c(STRONG_TEXT).opacity(0.25);
    t.window_border = c(DARK_SHADOW);

    t.primary = c(SELECTION);
    t.primary_hover = c(SELECTION_HOT);
    t.primary_active = c(SELECTION_DOWN);
    t.primary_foreground = white;
    t.secondary = c(FACE);
    t.secondary_hover = c(FACE_HOT);
    t.secondary_active = c(FACE_DOWN);
    t.secondary_foreground = c(TEXT);

    t.button = c(FACE);
    t.button_hover = c(FACE_HOT);
    t.button_active = c(FACE_DOWN);
    t.button_foreground = c(TEXT);
    t.button_secondary = c(FACE);
    t.button_secondary_hover = c(FACE_HOT);
    t.button_secondary_active = c(FACE_DOWN);
    t.button_secondary_foreground = c(TEXT);
    t.button_primary = c(SELECTION);
    t.button_primary_hover = c(SELECTION_HOT);
    t.button_primary_active = c(SELECTION_DOWN);
    t.button_primary_foreground = white;

    t.link = c(LINK);
    t.link_hover = c(SELECTION);
    t.link_active = c(SELECTION_DOWN);
    t.danger = c(ERROR);
    t.warning = c(WARN);

    t.colors.list = white;
    t.list_even = c(FAINT);
    t.list_head = c(FACE);
    t.list_hover = c(FACE_HOT);
    t.list_active = c(SELECTION).opacity(0.25);
    t.list_active_border = c(SELECTION);
    t.table = white;
    t.table_even = c(FAINT);
    t.table_head = c(FACE);
    t.table_head_foreground = c(TEXT);
    t.table_hover = c(FACE_HOT);
    t.table_active = c(SELECTION).opacity(0.25);
    t.table_active_border = c(SELECTION);
    t.table_row_border = c(SHADOW);

    t.scrollbar = c(TRACK);
    t.scrollbar_thumb = c(SHADOW);
    t.scrollbar_thumb_hover = c(DARK_SHADOW);
    t.slider_bar = c(SELECTION);
    t.slider_thumb = c(FACE);
    t.switch = c(SHADOW);
    t.switch_thumb = c(FACE);
    t.progress_bar = c(SELECTION);

    t.sidebar = c(SIDEBAR);
    t.sidebar_foreground = c(TEXT);
    t.sidebar_border = c(SHADOW);
    t.sidebar_accent = c(FACE_HOT);
    t.sidebar_accent_foreground = c(STRONG_TEXT);
    t.sidebar_primary = c(SELECTION);
    t.sidebar_primary_foreground = white;
    t.tab = c(FACE);
    t.tab_bar = c(FACE);
    t.tab_active = c(FACE_HOT);
    t.tab_active_foreground = c(STRONG_TEXT);
    t.tab_foreground = c(TEXT);
    t.title_bar = c(FACE);
    t.title_bar_border = c(SHADOW);
    t.status_bar = c(FACE);
    t.status_bar_border = c(SHADOW);

    t.radius = px(0.0);
    t.radius_lg = px(0.0);
    t.shadow = false;
    t.font_family = FONT.into();
}
