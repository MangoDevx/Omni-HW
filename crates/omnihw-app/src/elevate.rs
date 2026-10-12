//! Ask for administrator rights at startup when SATA drives are present, because reading their
//! SMART health requires it. If the user declines the UAC prompt, Omni-HW continues without it.
//! `--no-elevate` skips the prompt (and is passed to the elevated copy so it can never loop).
//! Launches at Windows sign-in (`--autostart`) and the "Ask for administrator at start" setting
//! being off also skip it.

const NO_ELEVATE: &str = "--no-elevate";

/// Returns true if an elevated copy was launched and this process should exit.
pub fn relaunch_elevated_if_useful() -> bool {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args
        .iter()
        .any(|a| a == NO_ELEVATE || a == crate::autostart::AUTOSTART_FLAG)
        || !crate::settings::Settings::load().ask_admin_at_start
        || omnihw_core::win::is_elevated()
        || !omnihw_core::win::has_smart_capable_drive()
    {
        return false;
    }
    let mut params: Vec<String> = args.iter().map(|a| quote(a)).collect();
    params.push(NO_ELEVATE.into());
    run_as_admin(&params.join(" "))
}

/// Quote an argument for the Windows command line.
fn quote(a: &str) -> String {
    if a.is_empty() || a.contains([' ', '\t', '"']) {
        format!("\"{}\"", a.replace('"', "\\\""))
    } else {
        a.to_string()
    }
}

#[cfg(windows)]
fn run_as_admin(params: &str) -> bool {
    use std::ffi::c_void;
    #[link(name = "shell32")]
    unsafe extern "system" {
        fn ShellExecuteW(
            hwnd: *mut c_void,
            op: *const u16,
            file: *const u16,
            params: *const u16,
            dir: *const u16,
            show: i32,
        ) -> isize;
    }
    let wide = |s: &str| {
        s.encode_utf16()
            .chain(std::iter::once(0))
            .collect::<Vec<u16>>()
    };
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let (op, file, params) = (wide("runas"), wide(&exe.to_string_lossy()), wide(params));
    const SW_SHOWNORMAL: i32 = 1;
    // SAFETY: NUL-terminated strings that outlive the call. Shows the UAC prompt; returns a
    // value <= 32 if it failed or the user declined.
    let r = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            op.as_ptr(),
            file.as_ptr(),
            params.as_ptr(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    r > 32
}

#[cfg(not(windows))]
fn run_as_admin(_params: &str) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::quote;

    #[test]
    fn quoting() {
        assert_eq!(quote("--x"), "--x");
        assert_eq!(quote("a b"), "\"a b\"");
        assert_eq!(quote(""), "\"\"");
    }
}
