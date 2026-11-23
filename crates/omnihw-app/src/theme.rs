//! Applies the theme and UI scale settings to gpui-component's global theme.

use crate::settings::{Settings, Theme, UI_SCALE_RANGE};
use gpui_kit::component::{Theme as KitTheme, ThemeMode};
use gpui_kit::{App, Pixels, SharedString, Window, px};
use std::sync::OnceLock;

/// Body text size at 100 % UI scale, in points. It is also the rem size, so the UI scale
/// setting scales text and spacing together.
const FONT_SIZE: f32 = 14.0;

/// The stock look, remembered at startup so leaving the Retro theme restores it.
struct Stock {
    font_family: SharedString,
    radius: Pixels,
    radius_lg: Pixels,
    shadow: bool,
}

static STOCK: OnceLock<Stock> = OnceLock::new();

/// Apply theme and UI scale. `window` is the main window when there is one (used to read the
/// system's light/dark setting).
pub fn apply(s: &Settings, window: Option<&mut Window>, cx: &mut App) {
    let stock = STOCK.get_or_init(|| {
        let t = KitTheme::global(cx);
        Stock {
            font_family: t.font_family.clone(),
            radius: t.radius,
            radius_lg: t.radius_lg,
            shadow: t.shadow,
        }
    });
    // Loads the mode's stock colours, so Retro's are applied (or dropped) afterwards.
    match s.theme {
        Theme::System => KitTheme::sync_system_appearance(window, cx),
        Theme::Dark => KitTheme::change(ThemeMode::Dark, None, cx),
        Theme::Light | Theme::Retro => KitTheme::change(ThemeMode::Light, None, cx),
    }
    let retro = s.theme == Theme::Retro;
    let scale = s
        .ui_scale
        .clamp(*UI_SCALE_RANGE.start(), *UI_SCALE_RANGE.end());
    KitTheme::update(cx, |t| {
        if retro {
            crate::retro::apply(t);
            t.font_size = px(crate::retro::FONT_SIZE * scale);
        } else {
            t.font_family = stock.font_family.clone();
            t.radius = stock.radius;
            t.radius_lg = stock.radius_lg;
            t.shadow = stock.shadow;
            t.font_size = px(FONT_SIZE * scale);
        }
    });
}
