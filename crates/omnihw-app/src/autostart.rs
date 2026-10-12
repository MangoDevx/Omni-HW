//! Launch at Windows sign-in via `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`.
//!
//! The Run entry starts Omni-HW with `--autostart` (never shows the UAC prompt) and, when
//! requested, `--minimized` (window starts minimized).

// Wired into app.rs/main.rs after merge; remove this allow then.

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
    use std::ffi::c_void;

    const HKEY_CURRENT_USER: isize = 0x8000_0001u32 as i32 as isize;
    const REG_SZ: u32 = 1;
    const RRF_RT_REG_SZ: u32 = 0x2;
    const ERROR_FILE_NOT_FOUND: i32 = 2;
    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const VALUE: &str = "Omni-HW";

    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn RegSetKeyValueW(
            key: isize,
            subkey: *const u16,
            value: *const u16,
            kind: u32,
            data: *const c_void,
            len: u32,
        ) -> i32;
        fn RegDeleteKeyValueW(key: isize, subkey: *const u16, value: *const u16) -> i32;
        fn RegGetValueW(
            key: isize,
            subkey: *const u16,
            value: *const u16,
            flags: u32,
            kind: *mut u32,
            data: *mut c_void,
            len: *mut u32,
        ) -> i32;
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub fn current() -> Option<String> {
        let (k, v) = (wide(RUN_KEY), wide(VALUE));
        let mut buf = [0u16; 1024];
        let mut len = (buf.len() * 2) as u32;
        // SAFETY: buffer size in bytes passed in `len`.
        let st = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                k.as_ptr(),
                v.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                buf.as_mut_ptr().cast(),
                &mut len,
            )
        };
        (st == 0).then(|| {
            String::from_utf16_lossy(&buf[..buf.iter().position(|&c| c == 0).unwrap_or(0)])
        })
    }

    pub fn write(cmd: &str) -> Result<(), String> {
        let (k, v, data) = (wide(RUN_KEY), wide(VALUE), wide(cmd));
        // SAFETY: NUL-terminated strings; data length in bytes includes the terminator.
        let st = unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                k.as_ptr(),
                v.as_ptr(),
                REG_SZ,
                data.as_ptr().cast(),
                (data.len() * 2) as u32,
            )
        };
        if st == 0 {
            Ok(())
        } else {
            Err(format!("Could not add Omni-HW to startup (error {st})"))
        }
    }

    pub fn remove() -> Result<(), String> {
        let (k, v) = (wide(RUN_KEY), wide(VALUE));
        // SAFETY: NUL-terminated strings.
        let st = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, k.as_ptr(), v.as_ptr()) };
        if st == 0 || st == ERROR_FILE_NOT_FOUND {
            Ok(())
        } else {
            Err(format!(
                "Could not remove Omni-HW from startup (error {st})"
            ))
        }
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn current() -> Option<String> {
        None
    }
    pub fn write(_cmd: &str) -> Result<(), String> {
        Err("Launch at startup is only supported on Windows".into())
    }
    pub fn remove() -> Result<(), String> {
        Ok(())
    }
}

/// Add or remove the startup entry for the running executable.
pub fn set(enabled: bool, minimized: bool) -> Result<(), String> {
    if !enabled {
        return imp::remove();
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
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
