//! Heatmap coloring: thresholds, severity levels and a base -> orange -> red color ramp.
//!
//! Colors always agree with [`Severity`]: anything at or above `crit` is fully red, values in
//! the warning band blend from orange toward red, and normal values use the caller's base color.

use gpui_kit::Rgba;

/// An opaque colour from 8-bit channels.
pub const fn rgb8(r: u8, g: u8, b: u8) -> Rgba {
    Rgba {
        r: r as f32 / 255.0,
        g: g as f32 / 255.0,
        b: b as f32 / 255.0,
        a: 1.0,
    }
}

pub const BLACK: Rgba = rgb8(0, 0, 0);
pub const WHITE: Rgba = rgb8(255, 255, 255);
pub const ORANGE: Rgba = rgb8(255, 152, 0);
pub const RED: Rgba = rgb8(229, 57, 53);
/// Mild warnings (e.g. Warning-level events, below orange errors; the administrator label).
pub const YELLOW: Rgba = rgb8(253, 216, 53);
pub const GREEN: Rgba = rgb8(76, 175, 80);
/// Informational blue (suggestions, CPU accents).
pub const BLUE: Rgba = rgb8(66, 133, 244);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Normal,
    Warning,
    Critical,
}

impl Severity {
    pub fn color(self) -> Option<Rgba> {
        match self {
            Severity::Normal => None,
            Severity::Warning => Some(ORANGE),
            Severity::Critical => Some(RED),
        }
    }
}

/// Thresholds on a "higher is worse" value: `warn` and `crit` start those levels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scale {
    pub warn: f64,
    pub crit: f64,
}

/// Busiest-thread load of a core, in %.
pub const CORE_LOAD: Scale = Scale {
    warn: 85.0,
    crit: 98.0,
};
/// Fraction of RAM in use.
pub const MEMORY_USED: Scale = Scale {
    warn: 0.80,
    crit: 0.92,
};
/// Fraction of VRAM in use.
pub const VRAM_USED: Scale = Scale {
    warn: 0.85,
    crit: 0.95,
};
/// GPU core temperature, °C.
pub const GPU_TEMP: Scale = Scale {
    warn: 75.0,
    crit: 85.0,
};
/// Drive wear: 100 - health %. Health below 80 % warns, below 50 % is critical.
pub const DRIVE_WEAR: Scale = Scale {
    warn: 20.0,
    crit: 50.0,
};

impl Scale {
    pub fn severity(&self, v: f64) -> Severity {
        if v >= self.crit {
            Severity::Critical
        } else if v >= self.warn {
            Severity::Warning
        } else {
            Severity::Normal
        }
    }

    /// `base` when normal, orange at `warn` blending toward red, and red from `crit` up.
    pub fn color(&self, base: Rgba, v: f64) -> Rgba {
        match self.severity(v) {
            Severity::Normal => base,
            Severity::Warning => lerp(
                ORANGE,
                RED,
                (0.6 * (v - self.warn) / (self.crit - self.warn)) as f32,
            ),
            Severity::Critical => RED,
        }
    }
}

/// A status color used as text on the normal background: unchanged in dark mode, darkened in
/// light mode so orange/green/blue stay readable on white.
pub fn ink(dark: bool, c: Rgba) -> Rgba {
    if dark { c } else { lerp(c, BLACK, 0.35) }
}

/// A bar fill that keeps the theme's normal text readable on top of it: slightly dimmed in dark
/// mode, a pastel tint in light mode.
pub fn soft_fill(dark: bool, c: Rgba) -> Rgba {
    if dark {
        fade(c, 0.85)
    } else {
        lerp(c, WHITE, 0.55)
    }
}

/// `c` with its opacity multiplied by `alpha` (tinted fills, dimmed glyphs).
pub fn fade(c: Rgba, alpha: f32) -> Rgba {
    Rgba {
        a: c.a * alpha.clamp(0.0, 1.0),
        ..c
    }
}

/// Black or white, whichever reads better on `fill`.
pub fn text_on(fill: Rgba) -> Rgba {
    let lum = 0.2126 * fill.r + 0.7152 * fill.g + 0.0722 * fill.b;
    if lum > 150.0 / 255.0 { BLACK } else { WHITE }
}

/// Blend opaque colours: `a` at `t` = 0, `b` at `t` = 1.
pub fn lerp(a: Rgba, b: Rgba, t: f32) -> Rgba {
    let t = t.clamp(0.0, 1.0);
    let m = |x: f32, y: f32| x + (y - x) * t;
    Rgba {
        r: m(a.r, b.r),
        g: m(a.g, b.g),
        b: m(a.b, b.b),
        a: 1.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: Rgba = rgb8(0, 0, 255);

    #[test]
    fn severity_levels() {
        assert_eq!(CORE_LOAD.severity(50.0), Severity::Normal);
        assert_eq!(CORE_LOAD.severity(85.0), Severity::Warning);
        assert_eq!(CORE_LOAD.severity(99.0), Severity::Critical);
        assert_eq!(DRIVE_WEAR.severity(100.0 - 97.0), Severity::Normal);
        assert_eq!(DRIVE_WEAR.severity(100.0 - 70.0), Severity::Warning);
        assert_eq!(DRIVE_WEAR.severity(100.0 - 40.0), Severity::Critical);
    }

    #[test]
    fn readable_text_helpers() {
        assert_eq!(text_on(rgb8(250, 250, 250)), BLACK);
        assert_eq!(text_on(rgb8(30, 40, 60)), WHITE);
        assert_eq!(text_on(RED), WHITE);
        assert_eq!(text_on(ORANGE), BLACK);
        assert_eq!(ink(true, ORANGE), ORANGE);
        assert_ne!(ink(false, ORANGE), ORANGE);
        assert_ne!(soft_fill(false, GREEN), soft_fill(true, GREEN));
        assert_eq!(fade(RED, 0.5).a, 0.5);
    }

    #[test]
    fn color_matches_severity() {
        assert_eq!(GPU_TEMP.color(BASE, 60.0), BASE);
        assert_eq!(GPU_TEMP.color(BASE, 75.0), ORANGE);
        let mid = GPU_TEMP.color(BASE, 80.0);
        assert!(mid != ORANGE && mid != RED, "warning band blends");
        // Whenever severity says critical (red text), the color is exactly red.
        for t in [85.0, 89.0, 95.0, 120.0] {
            assert_eq!(GPU_TEMP.severity(t), Severity::Critical);
            assert_eq!(GPU_TEMP.color(BASE, t), RED);
        }
    }
}
