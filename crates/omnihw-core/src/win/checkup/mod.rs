//! Collectors for [`crate::checkup::Checkup`]. Called once from `SystemInfo::gather` on the
//! background thread; nothing here runs on the UI thread or makes network requests.

#[cfg(windows)]
mod devices;
#[cfg(windows)]
mod displays;
#[cfg(windows)]
mod events;
#[cfg(windows)]
mod firmware;
#[cfg(windows)]
mod network;
#[cfg(windows)]
mod software;

use crate::checkup::Checkup;

#[cfg(windows)]
pub fn gather() -> Checkup {
    let (devices, nvme_links) = devices::devices();
    Checkup {
        firmware: firmware::firmware(),
        monitors: displays::monitors(),
        devices,
        events: events::events(),
        nics: network::wired_links(),
        nvme_links,
        activation: software::activation(),
        gpu_driver_packages: software::gpu_driver_packages(),
        startup_apps: software::startup_apps(),
    }
}

#[cfg(not(windows))]
pub fn gather() -> Checkup {
    Checkup::default()
}

#[cfg(windows)]
fn utf16_z(b: &[u16]) -> String {
    String::from_utf16_lossy(&b[..b.iter().position(|&c| c == 0).unwrap_or(b.len())])
}
