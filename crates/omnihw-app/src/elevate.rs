//! Ask for administrator rights at startup when SATA drives are present, because reading their
//! SMART health requires it. If the user declines the UAC prompt, Omni-HW continues without it.
//! `--no-elevate` skips the prompt (and is passed to the elevated copy so it can never loop).
//! Launches at Windows sign-in (`--autostart`) and the "Ask for administrator at start" setting
//! being off also skip it.

use std::sync::OnceLock;

const NO_ELEVATE: &str = "--no-elevate";

/// Whether this process runs as administrator (asked once; it can't change while running).
pub fn elevated() -> bool {
    static ELEVATED: OnceLock<bool> = OnceLock::new();
    *ELEVATED.get_or_init(omnihw_core::win::is_elevated)
}

/// Returns true if an elevated copy was launched and this process should exit.
pub fn relaunch_elevated_if_useful() -> bool {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args
        .iter()
        .any(|a| a == NO_ELEVATE || a == crate::autostart::AUTOSTART_FLAG)
        || !crate::settings::Settings::load().ask_admin_at_start
        || elevated()
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

/// Run this executable elevated with `params` (shows the UAC prompt), wait for it to finish and
/// return its exit code. Errors if the prompt was declined or the launch failed. Blocks, so call
/// it off the UI thread.
#[cfg(windows)]
pub fn run_as_admin_wait(params: &str) -> Result<u32, String> {
    use windows::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
    use windows::Win32::System::Threading::{GetExitCodeProcess, INFINITE, WaitForSingleObject};
    use windows::Win32::UI::Shell::{
        SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW,
    };
    use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;
    use windows::core::{HSTRING, w};
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let (file, params) = (
        HSTRING::from(&*exe.to_string_lossy()),
        HSTRING::from(params),
    );
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
        lpVerb: w!("runas"),
        lpFile: windows::core::PCWSTR(file.as_ptr()),
        lpParameters: windows::core::PCWSTR(params.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    // SAFETY: the strings outlive the call; shows the UAC prompt.
    unsafe { ShellExecuteExW(&mut info) }
        .map_err(|_| "Administrator permission was not granted".to_string())?;
    if info.hProcess.is_invalid() {
        return Err("Could not start the elevated helper".into());
    }
    /// Closes the helper's process handle on drop.
    struct Process(windows::Win32::Foundation::HANDLE);
    impl Drop for Process {
        fn drop(&mut self) {
            // SAFETY: a valid process handle from ShellExecuteExW, closed exactly once.
            let _ = unsafe { CloseHandle(self.0) };
        }
    }
    let process = Process(info.hProcess);
    let mut code = 1u32;
    // SAFETY: `process` holds a valid handle for the duration of both calls.
    let ok = unsafe {
        WaitForSingleObject(process.0, INFINITE) == WAIT_OBJECT_0
            && GetExitCodeProcess(process.0, &mut code).is_ok()
    };
    if !ok {
        return Err("The elevated helper did not finish".into());
    }
    Ok(code)
}

#[cfg(not(windows))]
pub fn run_as_admin_wait(_params: &str) -> Result<u32, String> {
    Err("Not supported on this platform".into())
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
