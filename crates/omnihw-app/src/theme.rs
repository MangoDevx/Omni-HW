//! Applies the theme and UI scale settings to gpui-component's global theme.
//!
//! Dark and Light use the palette, fonts and text sizes Omni-HW had before it moved to gpui
//! (egui's stock look), so the app looks the way its users know it: Ubuntu Light for text,
//! Hack for numbers, grey body text with brighter "strong" text, and a teal selection colour.
//! Retro replaces all of that with the classic Windows look (see [`crate::retro`]).

use crate::heat::rgb8;
use crate::settings::{Settings, Theme, UI_SCALE_RANGE};
use gpui_kit::component::{Theme as KitTheme, ThemeMode};
use gpui_kit::{App, Global, Hsla, Pixels, Rgba, Window, px};
use std::borrow::Cow;

/// The rem size at 100 % UI scale, in pixels. Sizes in the pages are written as
/// `rems(points / 16.0)`, so one point is one pixel at 100 % and the UI scale setting
/// (which scales the rem) scales the whole page.
const REM: f32 = 16.0;

/// Body text size, in points (egui's stock `Body` and `Button` size).
pub const BODY: f32 = 13.0;
/// Small text (captions, hints), in points (egui's stock `Small` size).
pub const SMALL: f32 = 9.0;
/// Line height as a multiple of the text size: the embedded fonts' own line spacing, as
/// egui used it (gpui's default is the golden ratio, which spreads text out).
pub const LINE_HEIGHT: f32 = 1.15;

/// Text font of the Dark and Light themes, embedded so it looks the same on every PC.
pub const FONT: &str = "Ubuntu";
/// Monospace font of every theme, embedded.
pub const MONO_FONT: &str = "Hack";
/// Black-and-white emoji font for the sidebar's page glyphs, embedded.
pub const EMOJI_FONT: &str = "Noto Emoji";
/// Icon font for the glyphs [`EMOJI_FONT`] lacks, embedded.
pub const ICON_FONT: &str = "emoji";

/// The colours pages draw with beyond what gpui-component's theme names: egui's widget
/// states, the strong and weak text colours and the backgrounds behind charts. Read it with
/// [`palette`].
#[derive(Clone, Copy)]
pub struct Palette {
    pub dark: bool,
    /// Page and panel background.
    pub panel: Rgba,
    /// Navigation sidebar background, a shade darker than the page.
    pub sidebar: Rgba,
    /// Behind charts, text fields and bars ("extreme" background).
    pub extreme: Rgba,
    /// Striped table rows and other barely tinted surfaces.
    pub faint: Rgba,
    /// Separators, frames and indentation lines.
    pub stroke: Rgba,
    /// Normal text.
    pub text: Rgba,
    /// Strong text (values, titles).
    pub strong: Rgba,
    /// Secondary text.
    pub weak: Rgba,
    /// Button and check box face.
    pub widget: Rgba,
    /// Text on buttons and other inactive widgets.
    pub widget_text: Rgba,
    /// Button face under the pointer.
    pub hover: Rgba,
    /// Outline of a widget under the pointer.
    pub hover_stroke: Rgba,
    /// Text on a widget under the pointer.
    pub hover_text: Rgba,
    /// Button face while pressed.
    pub active: Rgba,
    /// Selected rows, selected buttons, slider fill.
    pub selection: Rgba,
    /// Text and accents on [`Palette::selection`].
    pub selection_text: Rgba,
    /// Hyperlinks and the Tabs bar's underline.
    pub link: Rgba,
    pub warn: Rgba,
}

impl Global for Palette {}

thread_local! {
    /// The active palette, for text helpers that have no `App` at hand (`ui::strong`). gpui
    /// builds every element on the one UI thread, where [`apply`] sets it.
    static CURRENT: std::cell::Cell<Palette> = const { std::cell::Cell::new(DARK) };
}

/// The active theme's [`Palette`] (see [`palette`]).
pub fn current() -> Palette {
    CURRENT.with(|c| c.get())
}

/// Average advance of a glyph the table below doesn't cover, in ems.
const AVERAGE_ADVANCE: f32 = 0.55;

thread_local! {
    /// Advance widths of the printable ASCII characters in the UI font, in ems, measured by
    /// [`apply`]: label columns are sized from them while the element tree is built, when
    /// text can't be laid out yet.
    static ADVANCES: std::cell::Cell<[f32; 95]> = const { std::cell::Cell::new([AVERAGE_ADVANCE; 95]) };
}

/// Width of `text` on one line in the UI font, in ems.
pub fn text_width(text: &str) -> f32 {
    let table = ADVANCES.with(|a| a.get());
    text.chars()
        .map(|ch| match ch as usize {
            c @ 32..=126 => table[c - 32],
            _ => AVERAGE_ADVANCE,
        })
        .sum()
}

/// Measures the UI font's ASCII advance widths for [`text_width`].
fn measure_advances(family: &'static str, cx: &App) {
    let ts = cx.text_system();
    let id = ts.resolve_font(&gpui_kit::font(family));
    let size = px(REM);
    let mut table = [AVERAGE_ADVANCE; 95];
    for (i, w) in table.iter_mut().enumerate() {
        if let Ok(a) = ts.advance(id, size, (32 + i as u8) as char) {
            *w = a.width / size;
        }
    }
    ADVANCES.with(|a| a.set(table));
}

/// Mixes `c` with `bg` as egui draws a colour faded to `alpha`.
const fn over(c: u8, alpha: f32, bg: u8) -> u8 {
    (c as f32 * alpha + bg as f32 * (1.0 - alpha)) as u8
}

const fn gray(v: u8) -> Rgba {
    rgb8(v, v, v)
}

/// egui's stock dark visuals.
pub const DARK: Palette = Palette {
    dark: true,
    panel: gray(27),
    // `panel_fill.lerp_to_gamma(BLACK, 0.3)`.
    sidebar: gray(19),
    extreme: gray(10),
    faint: gray(32),
    stroke: gray(60),
    text: gray(140),
    strong: gray(255),
    // The text colour at egui's `weak_text_alpha` (0.6) over the panel.
    weak: gray(over(140, 0.6, 27)),
    widget: gray(60),
    widget_text: gray(180),
    hover: gray(70),
    hover_stroke: gray(150),
    hover_text: gray(240),
    active: gray(55),
    selection: rgb8(0, 92, 128),
    selection_text: rgb8(192, 222, 255),
    link: rgb8(90, 170, 255),
    warn: rgb8(255, 143, 0),
};

/// egui's stock light visuals.
pub const LIGHT: Palette = Palette {
    dark: false,
    panel: gray(248),
    sidebar: gray(239),
    extreme: gray(255),
    // egui's `faint_bg_color`: the panel lifted by 5 (as in dark mode).
    faint: gray(253),
    stroke: gray(190),
    text: gray(80),
    strong: gray(0),
    weak: gray(over(80, 0.6, 248)),
    widget: gray(230),
    widget_text: gray(60),
    hover: gray(220),
    hover_stroke: gray(105),
    hover_text: gray(0),
    active: gray(165),
    selection: rgb8(144, 209, 255),
    selection_text: rgb8(0, 83, 125),
    link: rgb8(0, 155, 255),
    warn: rgb8(255, 100, 0),
};

/// The active theme's [`Palette`].
pub fn palette(cx: &App) -> &Palette {
    cx.try_global::<Palette>().unwrap_or(&DARK)
}

/// Embeds the fonts the themes use. Call once at startup, before the first window opens.
pub fn load_fonts(cx: &mut App) {
    use epaint_default_fonts::{EMOJI_ICON, HACK_REGULAR, NOTO_EMOJI_REGULAR, UBUNTU_LIGHT};
    let fonts = [UBUNTU_LIGHT, HACK_REGULAR, NOTO_EMOJI_REGULAR, EMOJI_ICON];
    if let Err(e) = cx
        .text_system()
        .add_fonts(fonts.into_iter().map(Cow::Borrowed).collect())
    {
        eprintln!("could not load the embedded fonts: {e}");
    }
}

/// Apply theme and UI scale. `window` is the main window when there is one (used to read the
/// system's light/dark setting).
pub fn apply(s: &Settings, window: Option<&mut Window>, cx: &mut App) {
    // Loads the mode's stock colours, so ours are applied over them afterwards.
    match s.theme {
        Theme::System => KitTheme::sync_system_appearance(window, cx),
        Theme::Dark => KitTheme::change(ThemeMode::Dark, None, cx),
        Theme::Light | Theme::Retro => KitTheme::change(ThemeMode::Light, None, cx),
    }
    let retro = s.theme == Theme::Retro;
    let dark = KitTheme::global(cx).is_dark();
    let scale = s
        .ui_scale
        .clamp(*UI_SCALE_RANGE.start(), *UI_SCALE_RANGE.end());
    let palette = if retro {
        crate::retro::PALETTE
    } else if dark {
        DARK
    } else {
        LIGHT
    };
    cx.set_global(palette);
    CURRENT.with(|c| c.set(palette));
    KitTheme::update(cx, |t| {
        if retro {
            crate::retro::apply(t);
        } else {
            apply_palette(t, &palette);
            t.font_family = FONT.into();
            t.radius = px(2.0);
            t.radius_lg = px(4.0);
            t.shadow = true;
        }
        t.mono_font_family = MONO_FONT.into();
        t.font_size = px(REM * scale);
        t.mono_font_size = px(BODY * scale);
    });
    measure_advances(if retro { crate::retro::FONT } else { FONT }, cx);
}

/// Body text size in pixels at the current UI scale, for the window root.
pub fn body_size(cx: &App) -> Pixels {
    KitTheme::global(cx).font_size * (BODY / REM)
}

/// Sets gpui-component's colours from an egui palette, so its buttons, menus, inputs and
/// scroll bars match the pages.
fn apply_palette(t: &mut KitTheme, p: &Palette) {
    let c = |c: Rgba| Hsla::from(c);
    let selection = c(p.selection);
    let on_selection = c(p.selection_text);

    t.background = c(p.panel);
    t.foreground = c(p.text);
    t.border = c(p.stroke);
    t.muted = c(p.faint);
    t.muted_foreground = c(p.weak);
    t.accent = c(p.hover);
    t.accent_foreground = c(p.hover_text);
    t.group_box = c(p.panel);
    t.group_box_foreground = c(p.text);
    t.popover = c(p.panel);
    t.popover_foreground = c(p.widget_text);
    t.input = c(p.stroke);
    t.ring = c(p.selection_text).opacity(0.6);
    t.caret = c(p.strong);
    t.selection = selection.opacity(0.7);
    t.window_border = c(p.stroke);

    t.primary = selection;
    t.primary_hover = selection.blend(c(p.strong).opacity(0.1));
    t.primary_active = selection.blend(c(p.panel).opacity(0.2));
    t.primary_foreground = on_selection;
    t.secondary = c(p.widget);
    t.secondary_hover = c(p.hover);
    t.secondary_active = c(p.active);
    t.secondary_foreground = c(p.widget_text);

    t.button = c(p.widget);
    t.button_hover = c(p.hover);
    t.button_active = c(p.active);
    t.button_foreground = c(p.widget_text);
    t.button_secondary = c(p.widget);
    t.button_secondary_hover = c(p.hover);
    t.button_secondary_active = c(p.active);
    t.button_secondary_foreground = c(p.widget_text);
    t.button_primary = selection;
    t.button_primary_hover = t.primary_hover;
    t.button_primary_active = t.primary_active;
    t.button_primary_foreground = on_selection;

    t.link = c(p.link);
    t.link_hover = c(p.link).blend(c(p.strong).opacity(0.2));
    t.link_active = c(p.link);
    t.warning = c(p.warn);

    t.colors.list = c(p.panel);
    t.list_even = c(p.faint);
    t.list_head = c(p.panel);
    t.list_hover = c(p.hover);
    t.list_active = selection.opacity(0.6);
    t.list_active_border = on_selection;
    t.table = c(p.panel);
    t.table_even = c(p.faint);
    t.table_head = c(p.panel);
    t.table_head_foreground = c(p.text);
    t.table_hover = c(p.hover);
    t.table_active = selection.opacity(0.6);
    t.table_active_border = on_selection;
    t.table_row_border = c(p.stroke);

    t.scrollbar = c(p.panel).opacity(0.0);
    t.scrollbar_thumb = c(p.widget);
    t.scrollbar_thumb_hover = c(p.hover_stroke);
    t.slider_bar = selection;
    t.slider_thumb = c(p.widget_text);
    t.switch = c(p.widget);
    t.switch_thumb = c(p.widget_text);
    t.progress_bar = selection;

    t.sidebar = c(p.sidebar);
    t.sidebar_foreground = c(p.text);
    t.sidebar_border = c(p.stroke);
    t.sidebar_accent = c(p.hover);
    t.sidebar_accent_foreground = c(p.hover_text);
    t.sidebar_primary = selection;
    t.sidebar_primary_foreground = on_selection;
    t.tab = c(p.panel);
    t.tab_bar = c(p.panel);
    t.tab_active = c(p.panel);
    t.tab_active_foreground = c(p.strong);
    t.tab_foreground = c(p.widget_text);
    t.title_bar = c(p.panel);
    t.title_bar_border = c(p.stroke);
    t.status_bar = c(p.panel);
    t.status_bar_border = c(p.stroke);
}
