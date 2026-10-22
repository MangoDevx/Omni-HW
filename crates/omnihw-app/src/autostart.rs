//! Launch at sign-in via an XDG autostart entry (`~/.config/autostart/omnihw.desktop`).
//!
//! The entry starts Omni-HW with `--autostart` (never asks for root) and, when requested,
//! `--minimized` (window starts minimized).

pub const AUTOSTART_FLAG: &str = "--autostart";
pub const MINIMIZED_FLAG: &str = "--minimized";

/// (launched at sign-in, start minimized) from the command line.
pub fn launch_flags() -> (bool, bool) {
    flags_from(std::env::args().skip(1))
}

fn flags_from(args: impl IntoIterator<Item = String>) -> (bool, bool) {
    let mut autostart = false;
    let mut minimized = false;
    for a in args {
        if a == AUTOSTART_FLAG {
            autostart = true;
        } else if a == MINIMIZED_FLAG {
            minimized = true;
        }
    }
    (autostart, minimized)
}

/// The autostart entry's command line.
fn command_line(exe: &str, minimized: bool) -> String {
    let mut s = format!("\"{exe}\" {AUTOSTART_FLAG}");
    if minimized {
        s.push(' ');
        s.push_str(MINIMIZED_FLAG);
    }
    s
}

/// XDG autostart entry running `cmd`.
fn desktop_entry(cmd: &str) -> String {
    [
        "[Desktop Entry]",
        "Type=Application",
        "Name=Omni-HW",
        "Comment=Hardware information and monitoring",
        &format!("Exec={cmd}"),
        "Icon=omnihw",
        "Terminal=false",
        "X-GNOME-Autostart-enabled=true",
        "",
    ]
    .join("\n")
}

mod imp {
    use std::path::PathBuf;

    /// `$XDG_CONFIG_HOME/autostart/omnihw.desktop` (the config folder's parent is
    /// `$XDG_CONFIG_HOME`).
    fn entry_path() -> Option<PathBuf> {
        let config = crate::settings::config_dir()?;
        Some(config.parent()?.join("autostart").join("omnihw.desktop"))
    }

    pub fn current() -> Option<String> {
        let text = std::fs::read_to_string(entry_path()?).ok()?;
        text.lines()
            .find_map(|l| l.strip_prefix("Exec="))
            .map(str::to_string)
    }

    pub fn write(cmd: &str) -> Result<(), String> {
        let path = entry_path().ok_or("HOME is not set")?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(&path, super::desktop_entry(cmd))
            .map_err(|e| format!("Could not write {}: {e}", path.display()))
    }

    pub fn remove() -> Result<(), String> {
        let Some(path) = entry_path() else {
            return Ok(());
        };
        match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                Err(format!("Could not remove {}: {e}", path.display()))
            }
            _ => Ok(()),
        }
    }
}

/// Add or remove the startup entry for the running executable.
pub fn set(enabled: bool, minimized: bool) -> Result<(), String> {
    if !enabled {
        return imp::remove();
    }
    // An AppImage runs from a temporary mount; $APPIMAGE is the file to start next time.
    let exe = match std::env::var_os("APPIMAGE") {
        Some(p) => std::path::PathBuf::from(p),
        None => std::env::current_exe().map_err(|e| e.to_string())?,
    };
    imp::write(&command_line(&exe.to_string_lossy(), minimized))
}

/// Whether a startup entry for Omni-HW exists.
pub fn is_enabled() -> bool {
    imp::current().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags() {
        let a = |v: &[&str]| flags_from(v.iter().map(|s| s.to_string()));
        assert_eq!(a(&[]), (false, false));
        assert_eq!(a(&["--autostart"]), (true, false));
        assert_eq!(a(&["--autostart", "--minimized"]), (true, true));
    }

    #[test]
    fn autostart_entry() {
        let e = desktop_entry(r#""/opt/omnihw/omnihw" --autostart"#);
        assert!(e.starts_with("[Desktop Entry]\n"));
        assert!(e.contains("\nExec=\"/opt/omnihw/omnihw\" --autostart\n"));
        assert!(e.ends_with("=true\n"));
    }

    #[test]
    fn command() {
        assert_eq!(
            command_line("/opt/omnihw/omnihw", false),
            r#""/opt/omnihw/omnihw" --autostart"#
        );
        assert_eq!(
            command_line("/opt/omnihw/omnihw", true),
            r#""/opt/omnihw/omnihw" --autostart --minimized"#
        );
    }
}
