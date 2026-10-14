//! Windows activation (local licensing state, no network), installed graphics driver packages
//! and startup apps.

use crate::checkup::{
    Activation, activation_from_genuine_state, gpu_package_vendor, startup_approved_disabled,
};
use crate::win::registry::{self, Hive};
use std::path::PathBuf;
use windows::Win32::Security::Authentication::Identity::{SL_GENUINE_STATE, SLIsGenuineLocal};
use windows::core::GUID;

/// Licensing application id of Windows itself.
const WINDOWS_APP_ID: GUID = GUID::from_u128(0x55c92734_d682_4d71_983e_d6ec3f16059f);

pub fn activation() -> Option<Activation> {
    let mut state = SL_GENUINE_STATE::default();
    // SAFETY: valid GUID and out pointer; reads the local license store only.
    unsafe { SLIsGenuineLocal(&WINDOWS_APP_ID, &mut state, None) }.ok()?;
    activation_from_genuine_state(state.0)
}

const UNINSTALL: [&str; 2] = [
    r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
    r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall",
];

pub fn gpu_driver_packages() -> Vec<String> {
    let mut out = Vec::new();
    for root in UNINSTALL {
        for k in registry::subkeys(Hive::LocalMachine, root) {
            let Some(name) =
                registry::string(Hive::LocalMachine, &format!(r"{root}\{k}"), "DisplayName")
            else {
                continue;
            };
            if gpu_package_vendor(&name).is_some() && !out.contains(&name) {
                out.push(name);
            }
        }
    }
    out
}

const APPROVED: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved";

/// Entries of a Run key that aren't disabled in Task Manager.
fn run_key(hive: Hive, key: &str, approved: &str, out: &mut Vec<String>) {
    for name in registry::value_names(hive, key) {
        if name.is_empty() {
            continue;
        }
        let disabled = registry::binary(hive, &format!(r"{APPROVED}\{approved}"), &name)
            .is_some_and(|v| startup_approved_disabled(&v));
        if !disabled {
            out.push(name);
        }
    }
}

/// Shortcuts in a Startup folder that aren't disabled in Task Manager.
fn startup_folder(dir: Option<PathBuf>, hive: Hive, out: &mut Vec<String>) {
    let Some(entries) = dir.and_then(|d| std::fs::read_dir(d).ok()) else {
        return;
    };
    for e in entries.flatten() {
        let file = e.file_name().to_string_lossy().to_string();
        if file.eq_ignore_ascii_case("desktop.ini") || !e.path().is_file() {
            continue;
        }
        let disabled = registry::binary(hive, &format!(r"{APPROVED}\StartupFolder"), &file)
            .is_some_and(|v| startup_approved_disabled(&v));
        if !disabled {
            out.push(
                e.path()
                    .file_stem()
                    .map_or(file.clone(), |s| s.to_string_lossy().to_string()),
            );
        }
    }
}

pub fn startup_apps() -> Option<Vec<String>> {
    const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const RUN32: &str = r"Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Run";
    let mut out = Vec::new();
    run_key(Hive::CurrentUser, RUN, "Run", &mut out);
    run_key(Hive::LocalMachine, RUN, "Run", &mut out);
    run_key(Hive::LocalMachine, RUN32, "Run32", &mut out);
    let folder = |var: &str| {
        std::env::var_os(var)
            .map(|p| PathBuf::from(p).join(r"Microsoft\Windows\Start Menu\Programs\Startup"))
    };
    startup_folder(folder("APPDATA"), Hive::CurrentUser, &mut out);
    startup_folder(folder("ProgramData"), Hive::LocalMachine, &mut out);
    Some(out)
}
