//! Platform data sources behind one API, implemented by the readers in [`crate::linux`].

pub mod events_viewer;
pub mod nvml;
pub mod power;
pub mod settings;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OsInfo {
    /// e.g. "Windows 11 Pro"
    pub product: String,
    /// e.g. "24H2"
    pub display_version: String,
    pub build: u32,
    /// Update build revision.
    pub ubr: u32,
    pub computer_name: String,
    /// Kernel release on Linux, e.g. "6.8.0-45-generic"; empty on Windows.
    pub kernel: String,
}

impl OsInfo {
    /// "Windows 11 Pro 24H2 (build 26100.2033)", or on Linux
    /// "Ubuntu 24.04.1 LTS (kernel 6.8.0-45-generic)".
    pub fn summary(&self) -> String {
        if !self.kernel.is_empty() {
            return format!("{} (kernel {})", self.product, self.kernel);
        }
        let mut s = self.product.clone();
        if !self.display_version.is_empty() {
            s.push(' ');
            s.push_str(&self.display_version);
        }
        format!("{s} (build {}.{})", self.build, self.ubr)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Battery {
    pub percent: u8,
    pub on_ac: bool,
    pub charging: bool,
    pub seconds_left: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanKind {
    Balanced,
    HighPerformance,
    UltimatePerformance,
    PowerSaver,
    Custom,
}

impl PlanKind {
    /// Map a power scheme GUID (lower-case, hyphenated) to the built-in plan it is.
    pub fn from_guid(guid: &str) -> PlanKind {
        match guid {
            "381b4222-f694-41f0-9685-ff5bb260df2e" => PlanKind::Balanced,
            "8c5e7fda-e8bf-4a96-9a85-a6e23a8c635c" => PlanKind::HighPerformance,
            "e9a42b02-d5df-448d-aa00-03f14749eb61" => PlanKind::UltimatePerformance,
            "a1841308-3541-4fab-bc81-f71556f20b4a" => PlanKind::PowerSaver,
            _ => PlanKind::Custom,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PowerPlan {
    pub name: String,
    pub guid: String,
    pub kind: PlanKind,
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
/// Whether a drive could report ATA SMART (needs root).
pub use imp::has_smart_capable_drive;
/// Whether this process runs as root.
pub use imp::is_elevated;
pub use imp::memory_status;
/// Nominal (base) clock in MHz.
pub use imp::nominal_mhz;
/// Raw SMBIOS table wrapped as RawSMBIOSData (root only).
pub use imp::smbios_buffer;
pub use imp::{battery, microcode_revision, os_info, power_plan, uptime_secs};

/// Data collectors for the setup checks of the Health Check.
pub mod checkup;
