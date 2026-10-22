//! Platform data sources behind one API, implemented by the readers in [`crate::linux`].

pub mod events_viewer;
pub mod nvml;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OsInfo {
    /// The distribution, e.g. "Ubuntu 24.04.1 LTS".
    pub product: String,
    pub computer_name: String,
    /// Kernel release, e.g. "6.8.0-45-generic".
    pub kernel: String,
}

impl OsInfo {
    /// "Ubuntu 24.04.1 LTS (kernel 6.8.0-45-generic)".
    pub fn summary(&self) -> String {
        if self.kernel.is_empty() {
            return self.product.clone();
        }
        format!("{} (kernel {})", self.product, self.kernel)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Battery {
    pub percent: u8,
    pub on_ac: bool,
    pub charging: bool,
    pub seconds_left: Option<u32>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MemoryStatus {
    pub load_percent: u32,
    pub total_bytes: u64,
    pub available_bytes: u64,
}

use crate::linux::imp;

/// Battery design vs full-charge capacity (None on desktops).
pub use imp::battery_health;
/// Physical drives with volumes (and NVMe health when permitted).
pub use imp::drives;
/// Display adapters (DRM), with NVML details on NVIDIA cards.
pub use imp::gpu_adapters;
/// Whether this process runs as root.
pub use imp::is_elevated;
pub use imp::memory_status;
/// Nominal (base) clock in MHz.
pub use imp::nominal_mhz;
/// Raw SMBIOS table wrapped as RawSMBIOSData (root only).
pub use imp::smbios_buffer;
pub use imp::{battery, microcode_revision, os_info, uptime_secs};

/// Data collectors for the setup checks of the Health Check.
pub mod checkup;
