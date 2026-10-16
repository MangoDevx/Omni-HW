//! User settings, saved as `key=value` lines in `%APPDATA%/OmniHW/settings.ini`.
//! Unknown keys and malformed values are ignored, so older/newer files stay readable.

use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    /// Follow Windows' light/dark setting.
    System,
    Dark,
    Light,
    /// Early-2000s desktop look: beveled controls on beige surfaces (a light theme).
    Retro,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavStyle {
    /// Pages grouped into categories with dropdown menus.
    Grouped,
    /// Every page as its own tab in one row.
    Classic,
}

/// Page keys accepted for `start_page`, with friendly names.
pub const PAGES: [(&str, &str); 12] = [
    ("overview", "Overview"),
    ("health", "Health Check"),
    ("cpu", "CPU"),
    ("caches", "Caches"),
    ("mainboard", "Mainboard"),
    ("memory", "Memory"),
    ("gpu", "GPU"),
    ("storage", "Storage"),
    ("sensors", "Sensors"),
    ("recording", "Recording"),
    ("alerts", "Alerts"),
    ("about", "About"),
];

/// Allowed sampling intervals in milliseconds.
pub const INTERVALS_MS: [u64; 4] = [250, 500, 1000, 2000];

pub const UI_SCALE_RANGE: std::ops::RangeInclusive<f32> = 0.75..=2.0;

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub theme: Theme,
    /// Zoom factor for the whole UI.
    pub ui_scale: f32,
    pub nav_style: NavStyle,
    /// Key from [`PAGES`] shown when the app starts.
    pub start_page: String,
    pub sample_interval_ms: u64,
    pub launch_at_startup: bool,
    /// Start minimized when launched at Windows startup.
    pub start_minimized: bool,
    /// Minimizing hides the window to the notification area (system tray) instead of the taskbar.
    pub minimize_to_tray: bool,
    /// Ask for administrator rights at start (SATA drive health).
    pub ask_admin_at_start: bool,
    /// Windows notifications and taskbar flash for fired alerts (they always show in-app).
    pub alert_notifications: bool,
    /// Where recordings and reports are saved; `None` = Documents.
    pub data_dir: Option<PathBuf>,
    /// Check GitHub for a newer release at start (the app's only network request).
    pub check_updates_at_start: bool,
    /// Keep dismissed Health Check findings dismissed after a restart.
    pub remember_dismissed: bool,
    /// Show the Gaming category and gaming suggestions in Health Check and on Overview.
    pub show_gaming: bool,
    /// Keys of dismissed Health Check findings (see `dismissed::key`). Only saved to disk when
    /// `remember_dismissed` is on.
    pub dismissed: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme: Theme::System,
            ui_scale: 1.0,
            nav_style: NavStyle::Grouped,
            start_page: "overview".into(),
            sample_interval_ms: 1000,
            launch_at_startup: false,
            start_minimized: false,
            minimize_to_tray: true,
            ask_admin_at_start: true,
            alert_notifications: true,
            data_dir: None,
            check_updates_at_start: false,
            remember_dismissed: true,
            show_gaming: true,
            dismissed: Vec::new(),
        }
    }
}

fn bool_str(b: bool) -> &'static str {
    if b { "true" } else { "false" }
}

fn parse_bool(v: &str) -> Option<bool> {
    match v.to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Some(true),
        "false" | "0" | "no" | "off" => Some(false),
        _ => None,
    }
}

impl Settings {
    /// `%APPDATA%/OmniHW/settings.ini`.
    pub fn path() -> Option<PathBuf> {
        Some(
            PathBuf::from(std::env::var_os("APPDATA")?)
                .join("OmniHW")
                .join("settings.ini"),
        )
    }

    pub fn to_ini(&self) -> String {
        let theme = match self.theme {
            Theme::System => "system",
            Theme::Dark => "dark",
            Theme::Light => "light",
            Theme::Retro => "retro",
        };
        let nav = match self.nav_style {
            NavStyle::Grouped => "grouped",
            NavStyle::Classic => "classic",
        };
        let data_dir = self
            .data_dir
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        // One `dismissed=` line per key, only when they should survive a restart.
        let dismissed: String = if self.remember_dismissed {
            self.dismissed
                .iter()
                .map(|k| format!("dismissed={}\n", k.replace(['\r', '\n'], " ")))
                .collect()
        } else {
            String::new()
        };
        format!(
            "# Omni-HW settings\n\
             theme={theme}\n\
             ui_scale={}\n\
             nav_style={nav}\n\
             start_page={}\n\
             sample_interval_ms={}\n\
             launch_at_startup={}\n\
             start_minimized={}\n\
             minimize_to_tray={}\n\
             ask_admin_at_start={}\n\
             alert_notifications={}\n\
             data_dir={data_dir}\n\
             check_updates_at_start={}\n\
             remember_dismissed={}\n\
             show_gaming={}\n\
             {dismissed}",
            self.ui_scale,
            self.start_page,
            self.sample_interval_ms,
            bool_str(self.launch_at_startup),
            bool_str(self.start_minimized),
            bool_str(self.minimize_to_tray),
            bool_str(self.ask_admin_at_start),
            bool_str(self.alert_notifications),
            bool_str(self.check_updates_at_start),
            bool_str(self.remember_dismissed),
            bool_str(self.show_gaming),
        )
    }

    /// Parse settings text; anything missing or invalid keeps its default.
    pub fn from_ini(text: &str) -> Settings {
        let mut s = Settings::default();
        for line in text.trim_start_matches('\u{feff}').lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let v = v.trim();
            match k.trim() {
                "theme" => match v {
                    "system" => s.theme = Theme::System,
                    "dark" => s.theme = Theme::Dark,
                    "light" => s.theme = Theme::Light,
                    "retro" => s.theme = Theme::Retro,
                    _ => {}
                },
                "ui_scale" => {
                    if let Ok(x) = v.parse::<f32>()
                        && x.is_finite()
                    {
                        s.ui_scale = x.clamp(*UI_SCALE_RANGE.start(), *UI_SCALE_RANGE.end());
                    }
                }
                "nav_style" => match v {
                    "grouped" => s.nav_style = NavStyle::Grouped,
                    "classic" => s.nav_style = NavStyle::Classic,
                    _ => {}
                },
                "start_page" => {
                    if PAGES.iter().any(|(key, _)| *key == v) {
                        s.start_page = v.to_string();
                    }
                }
                "sample_interval_ms" => {
                    if let Ok(ms) = v.parse::<u64>()
                        && INTERVALS_MS.contains(&ms)
                    {
                        s.sample_interval_ms = ms;
                    }
                }
                "launch_at_startup" => {
                    s.launch_at_startup = parse_bool(v).unwrap_or(s.launch_at_startup)
                }
                "start_minimized" => s.start_minimized = parse_bool(v).unwrap_or(s.start_minimized),
                "ask_admin_at_start" => {
                    s.ask_admin_at_start = parse_bool(v).unwrap_or(s.ask_admin_at_start)
                }
                "minimize_to_tray" => {
                    s.minimize_to_tray = parse_bool(v).unwrap_or(s.minimize_to_tray)
                }
                "alert_notifications" => {
                    s.alert_notifications = parse_bool(v).unwrap_or(s.alert_notifications)
                }
                "data_dir" => s.data_dir = (!v.is_empty()).then(|| PathBuf::from(v)),
                "check_updates_at_start" => {
                    s.check_updates_at_start = parse_bool(v).unwrap_or(s.check_updates_at_start)
                }
                "remember_dismissed" => {
                    s.remember_dismissed = parse_bool(v).unwrap_or(s.remember_dismissed)
                }
                "show_gaming" => s.show_gaming = parse_bool(v).unwrap_or(s.show_gaming),
                "dismissed" if !v.is_empty() => s.dismissed.push(v.to_string()),
                _ => {}
            }
        }
        s
    }

    /// Load from disk, or defaults if the file is missing.
    pub fn load() -> Settings {
        Self::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|t| Self::from_ini(&t))
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        let path = Self::path().ok_or("APPDATA is not set")?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(&path, self.to_ini())
            .map_err(|e| format!("Could not save {}: {e}", path.display()))
    }

    /// Friendly name of the start page.
    pub fn start_page_name(&self) -> &'static str {
        PAGES
            .iter()
            .find(|(k, _)| *k == self.start_page)
            .map_or("Overview", |(_, n)| n)
    }
}

/// Apply theme and UI scale.
pub fn apply_visuals(ctx: &egui::Context, s: &Settings) {
    // Retro replaces the light style while selected; any other choice restores egui's own.
    crate::retro::set_enabled(ctx, s.theme == Theme::Retro);
    ctx.set_theme(match s.theme {
        Theme::System => egui::ThemePreference::System,
        Theme::Dark => egui::ThemePreference::Dark,
        Theme::Light | Theme::Retro => egui::ThemePreference::Light,
    });
    ctx.set_zoom_factor(
        s.ui_scale
            .clamp(*UI_SCALE_RANGE.start(), *UI_SCALE_RANGE.end()),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let s = Settings {
            theme: Theme::Light,
            ui_scale: 1.25,
            nav_style: NavStyle::Classic,
            start_page: "health".into(),
            sample_interval_ms: 500,
            launch_at_startup: true,
            start_minimized: true,
            minimize_to_tray: false,
            ask_admin_at_start: false,
            alert_notifications: false,
            data_dir: Some(PathBuf::from(r"D:\Omni HW\data")),
            check_updates_at_start: true,
            remember_dismissed: true,
            show_gaming: false,
            dismissed: vec!["Uptime|Info|Windows has been running for  days".into()],
        };
        assert_eq!(Settings::from_ini(&s.to_ini()), s);
        for theme in [Theme::System, Theme::Dark, Theme::Light, Theme::Retro] {
            let t = Settings { theme, ..s.clone() };
            assert_eq!(Settings::from_ini(&t.to_ini()).theme, theme);
        }
        // With remembering off, dismissals are kept for the session only.
        let session_only = Settings {
            remember_dismissed: false,
            ..s.clone()
        };
        assert!(
            Settings::from_ini(&session_only.to_ini())
                .dismissed
                .is_empty()
        );
        assert_eq!(
            Settings::from_ini(&Settings::default().to_ini()),
            Settings::default()
        );
    }

    #[test]
    fn tolerant_parsing() {
        let s = Settings::from_ini(
            "\u{feff}# comment\nunknown=1\ntheme=purple\nui_scale=9\nstart_page=nowhere\n\
             sample_interval_ms=333\nlaunch_at_startup=yes\nnot a line\ndata_dir=\n",
        );
        assert_eq!(s.theme, Theme::System);
        assert_eq!(s.ui_scale, 2.0, "clamped");
        assert_eq!(s.start_page, "overview");
        assert_eq!(s.sample_interval_ms, 1000);
        assert!(s.launch_at_startup);
        assert_eq!(s.data_dir, None);
        assert_eq!(Settings::from_ini(""), Settings::default());
    }

    #[test]
    fn page_names() {
        let s = Settings {
            start_page: "gpu".into(),
            ..Default::default()
        };
        assert_eq!(s.start_page_name(), "GPU");
    }
}
