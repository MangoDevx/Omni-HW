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
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows::core::{HSTRING, PCWSTR, w};
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let (file, params) = (
        HSTRING::from(&*exe.to_string_lossy()),
        HSTRING::from(params),
    );
    // SAFETY: NUL-terminated strings that outlive the call. Shows the UAC prompt; returns a
    // value <= 32 if it failed or the user declined.
    let r = unsafe {
        ShellExecuteW(
            None,
            w!("runas"),
            &file,
            &params,
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    r.0 as isize > 32
}

#[cfg(not(windows))]
fn run_as_admin(_params: &str) -> bool {
    false
}

/// Restart straight into the UEFI firmware setup (`shutdown /r /fw /t 0`). Needs administrator
/// rights, so it goes through UAC; only offered on UEFI systems. Only ever runs on a click
/// after the user confirmed.
#[cfg(windows)]
pub fn restart_to_firmware() -> Result<(), String> {
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;
    use windows::core::{HSTRING, PCWSTR, w};
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
    let exe = std::path::Path::new(&root).join(r"System32\shutdown.exe");
    let file = HSTRING::from(&*exe.to_string_lossy());
    // SAFETY: NUL-terminated strings that outlive the call. Shows the UAC prompt unless already
    // elevated; returns a value <= 32 if it failed or the user declined.
    let r = unsafe {
        ShellExecuteW(
            None,
            w!("runas"),
            &file,
            w!("/r /fw /t 0"),
            PCWSTR::null(),
            SW_HIDE,
        )
    };
    if r.0 as isize > 32 {
        Ok(())
    } else {
        Err("Couldn't restart: administrator rights were declined, or this PC doesn't support restarting into the firmware setup.".into())
    }
}

#[cfg(not(windows))]
pub fn restart_to_firmware() -> Result<(), String> {
    Err("Only supported on Windows".into())
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
