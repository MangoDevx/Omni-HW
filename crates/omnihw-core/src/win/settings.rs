//! Windows configuration that affects performance or stability: pending restarts, VBS /
//! Memory Integrity, hardware GPU scheduling, Game Mode, pagefile, and GPU driver dates.

/// Windows settings relevant to the Health Check. `None` means "couldn't tell".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WindowsSettings {
    /// Why Windows wants a restart (empty when none is pending).
    pub pending_reboot: Vec<&'static str>,
    /// Memory Integrity (HVCI) is running right now.
    pub hvci_running: Option<bool>,
    /// Virtualization-based security is turned on in policy/registry.
    pub vbs_configured: Option<bool>,
    /// Hardware-accelerated GPU scheduling; `None` when the GPU/driver doesn't offer it.
    pub hags: Option<bool>,
    pub game_mode: Option<bool>,
    /// No pagefile on any drive.
    pub pagefile_disabled: Option<bool>,
}

/// Parse a driver INF date ("M-D-YYYY", as stored in `DriverDate`) into (year, month, day).
pub fn parse_driver_date(s: &str) -> Option<(i64, u32, u32)> {
    let mut it = s.trim().split(['-', '/']);
    let m: u32 = it.next()?.parse().ok()?;
    let d: u32 = it.next()?.parse().ok()?;
    let y: i64 = it.next()?.parse().ok()?;
    ((1..=12).contains(&m) && (1..=31).contains(&d) && y > 1990).then_some((y, m, d))
}

/// `PagingFiles` lists one entry per pagefile ("C:\pagefile.sys 0 0", or "?:\pagefile.sys"
/// for system-managed); no entries means the pagefile is disabled.
pub fn pagefile_disabled(paging_files: &[String]) -> bool {
    paging_files.iter().all(|s| s.trim().is_empty())
}

/// Whether a display adapter's `MatchingDeviceId` (e.g. "pci\ven_10de&dev_2204&subsys_...")
/// belongs to the given PCI vendor/device.
pub fn matches_device(matching_id: &str, vendor: u32, device: u32) -> bool {
    matching_id
        .to_ascii_lowercase()
        .contains(&format!("ven_{vendor:04x}&dev_{device:04x}"))
}

#[cfg(windows)]
pub use imp::{attach_driver_dates, windows_settings};

#[cfg(not(windows))]
pub fn windows_settings() -> WindowsSettings {
    WindowsSettings::default()
}

#[cfg(windows)]
mod imp {
    use super::*;
    use crate::gpu::GpuAdapter;
    use crate::win::registry::{self, Hive};
    use windows::Wdk::System::SystemInformation::{
        NtQuerySystemInformation, SystemCodeIntegrityInformation,
    };
    use windows::Win32::Foundation::STATUS_SUCCESS;
    use windows::Win32::System::WindowsProgramming::{
        CODEINTEGRITY_OPTION_HVCI_KMCI_ENABLED, SYSTEM_CODEINTEGRITY_INFORMATION,
    };

    const DISPLAY_CLASS: &str =
        r"SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}";

    fn hvci_running() -> Option<bool> {
        let mut info = SYSTEM_CODEINTEGRITY_INFORMATION {
            Length: size_of::<SYSTEM_CODEINTEGRITY_INFORMATION>() as u32,
            CodeIntegrityOptions: 0,
        };
        let mut ret = 0;
        // SAFETY: Length set as required; buffer size matches.
        let st = unsafe {
            NtQuerySystemInformation(
                SystemCodeIntegrityInformation,
                (&mut info as *mut SYSTEM_CODEINTEGRITY_INFORMATION).cast(),
                info.Length,
                &mut ret,
            )
        };
        (st == STATUS_SUCCESS)
            .then_some(info.CodeIntegrityOptions & CODEINTEGRITY_OPTION_HVCI_KMCI_ENABLED != 0)
    }

    pub fn windows_settings() -> WindowsSettings {
        let mut pending_reboot = Vec::new();
        if registry::key_exists(
            Hive::LocalMachine,
            r"SOFTWARE\Microsoft\Windows\CurrentVersion\WindowsUpdate\Auto Update\RebootRequired",
        ) {
            pending_reboot.push("Windows Update");
        }
        if registry::key_exists(
            Hive::LocalMachine,
            r"SOFTWARE\Microsoft\Windows\CurrentVersion\Component Based Servicing\RebootPending",
        ) {
            pending_reboot.push("Windows component servicing");
        }
        let vbs_configured = registry::dword(
            Hive::LocalMachine,
            r"SYSTEM\CurrentControlSet\Control\DeviceGuard",
            "EnableVirtualizationBasedSecurity",
        )
        .map(|v| v != 0);
        let hags = registry::dword(
            Hive::LocalMachine,
            r"SYSTEM\CurrentControlSet\Control\GraphicsDrivers",
            "HwSchMode",
        )
        .map(|v| v == 2);
        // Game Mode defaults to on when the value is absent.
        let game_mode = Some(
            registry::dword(
                Hive::CurrentUser,
                r"Software\Microsoft\GameBar",
                "AutoGameModeEnabled",
            )
            .or_else(|| {
                registry::dword(
                    Hive::CurrentUser,
                    r"Software\Microsoft\GameBar",
                    "AllowAutoGameMode",
                )
            })
            .is_none_or(|v| v != 0),
        );
        let pagefile_disabled = registry::multi_string(
            Hive::LocalMachine,
            r"SYSTEM\CurrentControlSet\Control\Session Manager\Memory Management",
            "PagingFiles",
        )
        .map(|v| pagefile_disabled(&v));
        WindowsSettings {
            pending_reboot,
            hvci_running: hvci_running(),
            vbs_configured,
            hags,
            game_mode,
            pagefile_disabled,
        }
    }

    /// Fill `driver_date` from the display class registry keys (matched by PCI vendor/device).
    pub fn attach_driver_dates(adapters: &mut [GpuAdapter]) {
        for i in 0..64 {
            let key = format!(r"{DISPLAY_CLASS}\{i:04}");
            let Some(id) = registry::string(Hive::LocalMachine, &key, "MatchingDeviceId") else {
                continue;
            };
            let Some(date) = registry::string(Hive::LocalMachine, &key, "DriverDate")
                .and_then(|d| parse_driver_date(&d))
            else {
                continue;
            };
            for a in adapters.iter_mut().filter(|a| a.driver_date.is_none()) {
                if matches_device(&id, a.vendor_id, a.device_id) {
                    a.driver_date = Some(date);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn driver_dates() {
        assert_eq!(parse_driver_date("10-3-2025"), Some((2025, 10, 3)));
        assert_eq!(parse_driver_date("1/21/2024"), Some((2024, 1, 21)));
        assert_eq!(parse_driver_date("13-1-2024"), None);
        assert_eq!(parse_driver_date("garbage"), None);
    }

    #[test]
    fn pagefile() {
        assert!(pagefile_disabled(&[]));
        assert!(pagefile_disabled(&[" ".into()]));
        assert!(!pagefile_disabled(&[r"?:\pagefile.sys".into()]));
        assert!(!pagefile_disabled(&[r"C:\pagefile.sys 16384 32768".into()]));
    }

    #[test]
    fn device_matching() {
        assert!(matches_device(
            r"pci\ven_10de&dev_2204&subsys_161319da",
            0x10DE,
            0x2204
        ));
        assert!(matches_device(r"PCI\VEN_10DE&DEV_2204", 0x10DE, 0x2204));
        assert!(!matches_device(r"pci\ven_10de&dev_2206", 0x10DE, 0x2204));
    }
}
