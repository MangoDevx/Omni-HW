//! Heatmap coloring: thresholds, severity levels and a base -> orange -> red color ramp.
//!
//! Colors always agree with [`Severity`]: anything at or above `crit` is fully red, values in
//! the warning band blend from orange toward red, and normal values use the caller's base color.

use egui::Color32;

pub const ORANGE: Color32 = Color32::from_rgb(255, 152, 0);
pub const RED: Color32 = Color32::from_rgb(229, 57, 53);
/// Mild warnings (e.g. Warning-level events, below orange errors).
pub const YELLOW: Color32 = Color32::from_rgb(253, 216, 53);
pub const GREEN: Color32 = Color32::from_rgb(76, 175, 80);
/// Informational blue (suggestions, CPU accents).
pub const BLUE: Color32 = Color32::from_rgb(66, 133, 244);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Normal,
    Warning,
    Critical,
}

impl Severity {
    pub fn color(self) -> Option<Color32> {
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
    pub fn color(&self, base: Color32, v: f64) -> Color32 {
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
pub fn ink(v: &egui::Visuals, c: Color32) -> Color32 {
    if v.dark_mode {
        c
    } else {
        lerp(c, Color32::BLACK, 0.35)
    }
}

/// A bar fill that keeps the theme's normal text readable on top of it: slightly dimmed in dark
/// mode, a pastel tint in light mode.
pub fn soft_fill(v: &egui::Visuals, c: Color32) -> Color32 {
    if v.dark_mode {
        c.gamma_multiply(0.85)
    } else {
        lerp(c, Color32::WHITE, 0.55)
    }
}

/// Black or white, whichever reads better on `fill`.
pub fn text_on(fill: Color32) -> Color32 {
    let lum = 0.2126 * fill.r() as f32 + 0.7152 * fill.g() as f32 + 0.0722 * fill.b() as f32;
    if lum > 150.0 {
        Color32::BLACK
    } else {
        Color32::WHITE
    }
}

pub fn lerp(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: Color32 = Color32::from_rgb(0, 0, 255);

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
        assert_eq!(text_on(Color32::from_rgb(250, 250, 250)), Color32::BLACK);
        assert_eq!(text_on(Color32::from_rgb(30, 40, 60)), Color32::WHITE);
        assert_eq!(text_on(RED), Color32::WHITE);
        assert_eq!(text_on(ORANGE), Color32::BLACK);
        let dark = egui::Visuals::dark();
        let light = egui::Visuals::light();
        assert_eq!(ink(&dark, ORANGE), ORANGE);
        assert_ne!(ink(&light, ORANGE), ORANGE);
        assert_ne!(soft_fill(&light, GREEN), soft_fill(&dark, GREEN));
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
