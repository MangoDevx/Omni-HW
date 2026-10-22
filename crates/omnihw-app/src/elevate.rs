//! Root access. Omni-HW runs as the user; what only root can read (memory modules from the
//! SMBIOS tables, NVMe and SATA drive health) comes from a short-lived copy of Omni-HW started
//! through `pkexec` with `--root-read`. It prints the raw data (see
//! [`omnihw_core::linux::rootdata`]) and exits without opening a window. pkexec asks for the
//! password through the desktop's polkit agent. Nothing runs as root unless the user asks
//! (a button, or the "read at start" setting).

use omnihw_core::linux::{Root, rootdata};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

const ROOT_READ_FLAG: &str = "--root-read";

/// Whether this process runs as root (asked once; it can't change while running).
pub fn elevated() -> bool {
    static ELEVATED: OnceLock<bool> = OnceLock::new();
    *ELEVATED.get_or_init(omnihw_core::platform::is_elevated)
}

/// If this process was started as the root helper, print the data and return the exit code.
pub fn run_helper() -> Option<i32> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [flag] if flag == ROOT_READ_FLAG => {
            print!("{}", rootdata::collect(&Root::system()));
            Some(0)
        }
        [flag, ..] if flag == ROOT_READ_FLAG => Some(2),
        _ => None,
    }
}

/// The root-only data as [`rootdata::collect`] text: read directly when already root, else
/// through the pkexec helper (which shows the password prompt). Blocks, so call it off the UI
/// thread.
pub fn read_as_root() -> Result<String, String> {
    if elevated() {
        return Ok(rootdata::collect(&Root::system()));
    }
    let exe =
        std::env::current_exe().map_err(|e| format!("Can't find Omni-HW's executable: {e}"))?;
    let out = Command::new("pkexec")
        .arg(exe)
        .arg(ROOT_READ_FLAG)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => {
                "pkexec isn't installed (it comes with polkit; install it with your package manager)"
                    .to_string()
            }
            _ => format!("Couldn't run pkexec: {e}"),
        })?;
    pkexec_result(out.status.code(), &out.stdout, &out.stderr)
}

/// Interpret pkexec's exit code: 126 when the prompt was dismissed or not allowed, 127 when
/// authentication failed or pkexec couldn't start the helper.
fn pkexec_result(code: Option<i32>, stdout: &[u8], stderr: &[u8]) -> Result<String, String> {
    match code {
        Some(0) => Ok(String::from_utf8_lossy(stdout).into_owned()),
        Some(126) => Err("Root access wasn't granted".into()),
        Some(127) => {
            let e = String::from_utf8_lossy(stderr);
            Err(match e.trim() {
                "" => "Authentication failed".to_string(),
                e => format!("Authentication failed ({e})"),
            })
        }
        Some(c) => Err(format!("The root helper failed (exit code {c})")),
        None => Err("The root helper was stopped".into()),
    }
}

/// Restart straight into the UEFI firmware setup (`systemctl reboot --firmware-setup`; polkit
/// may ask for the password). Only offered on UEFI systems, and only ever runs on a click after
/// the user confirmed.
pub fn restart_to_firmware() -> Result<(), String> {
    let out = Command::new("systemctl")
        .args(["reboot", "--firmware-setup"])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("Couldn't run systemctl: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        let e = String::from_utf8_lossy(&out.stderr);
        Err(format!(
            "Couldn't restart into the firmware setup: {}",
            e.trim()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::pkexec_result;

    #[test]
    fn pkexec_codes() {
        assert_eq!(
            pkexec_result(Some(0), b"smbios 00\n", b""),
            Ok("smbios 00\n".into())
        );
        assert_eq!(
            pkexec_result(Some(126), b"", b""),
            Err("Root access wasn't granted".into())
        );
        assert!(pkexec_result(Some(127), b"", b"Not authorized").is_err());
        assert!(pkexec_result(Some(2), b"", b"").is_err());
        assert!(pkexec_result(None, b"", b"").is_err());
    }
}
