//! Launch at sign-in: on Windows via `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, on
//! Linux via an XDG autostart entry (`~/.config/autostart/omnihw.desktop`).
//!
//! The Run entry starts Omni-HW with `--autostart` (never shows the UAC prompt) and, when
//! requested, `--minimized` (window starts minimized).

pub const AUTOSTART_FLAG: &str = "--autostart";
pub const MINIMIZED_FLAG: &str = "--minimized";

/// (launched by Windows at sign-in, start minimized) from the command line.
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

/// The Run value's command line.
fn command_line(exe: &str, minimized: bool) -> String {
    let mut s = format!("\"{exe}\" {AUTOSTART_FLAG}");
    if minimized {
        s.push(' ');
        s.push_str(MINIMIZED_FLAG);
    }
    s
}

#[cfg(windows)]
mod imp {
    use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows::Win32::System::Registry::{
        HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
    };
    use windows::core::{PCWSTR, w};

    const RUN_KEY: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Run");
    const VALUE: PCWSTR = w!("Omni-HW");

    pub fn current() -> Option<String> {
        let mut buf = [0u16; 1024];
        let mut len = (buf.len() * 2) as u32;
        // SAFETY: buffer size in bytes passed in `len`.
        let st = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                RUN_KEY,
                VALUE,
                RRF_RT_REG_SZ,
                None,
                Some(buf.as_mut_ptr().cast()),
                Some(&mut len),
            )
        };
        (st == ERROR_SUCCESS).then(|| {
            String::from_utf16_lossy(&buf[..buf.iter().position(|&c| c == 0).unwrap_or(0)])
        })
    }

    pub fn write(cmd: &str) -> Result<(), String> {
        let data: Vec<u16> = cmd.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: NUL-terminated strings; data length in bytes includes the terminator.
        let st = unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                RUN_KEY,
                VALUE,
                REG_SZ.0,
                Some(data.as_ptr().cast()),
                (data.len() * 2) as u32,
            )
        };
        if st == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(format!("Could not add Omni-HW to startup (error {})", st.0))
        }
    }

    pub fn remove() -> Result<(), String> {
        // SAFETY: NUL-terminated strings.
        let st = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, VALUE) };
        if st == ERROR_SUCCESS || st == ERROR_FILE_NOT_FOUND {
            Ok(())
        } else {
            Err(format!(
                "Could not remove Omni-HW from startup (error {})",
                st.0
            ))
        }
    }
}

/// XDG autostart entry running `cmd`.
#[cfg_attr(windows, allow(dead_code))]
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

#[cfg(not(windows))]
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
            command_line(r"C:\Apps\omnihw.exe", false),
            r#""C:\Apps\omnihw.exe" --autostart"#
        );
        assert_eq!(
            command_line(r"C:\Apps\omnihw.exe", true),
            r#""C:\Apps\omnihw.exe" --autostart --minimized"#
        );
    }
}
