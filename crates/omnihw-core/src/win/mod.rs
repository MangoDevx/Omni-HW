//! Windows data sources. Every function has a non-Windows stub returning `None`
//! so the crate (and its parser tests) build anywhere.

#[cfg(windows)]
mod device;
#[cfg(windows)]
pub(crate) mod disktest;
#[cfg(windows)]
mod dxgi;
#[cfg(windows)]
mod eventlog;
#[cfg(windows)]
pub(crate) mod memtest;
#[cfg(windows)]
pub mod network;
#[cfg(windows)]
pub mod nvml;
#[cfg(windows)]
pub mod pawnio;
#[cfg(windows)]
mod pdh;
pub mod perf;
pub mod power;
#[cfg(windows)]
pub(crate) mod registry;
pub mod settings;
#[cfg(windows)]
pub mod storage;
#[cfg(windows)]
mod system;
#[cfg(windows)]
pub(crate) mod wide;
#[cfg(windows)]
mod wmi;

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
}

impl OsInfo {
    /// "Windows 11 Pro 24H2 (build 26100.2033)"
    pub fn summary(&self) -> String {
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

#[cfg(windows)]
use system as imp;

#[cfg(not(windows))]
mod imp {
    use super::MemoryStatus;
    pub fn gpu_adapters() -> Vec<crate::gpu::GpuAdapter> {
        Vec::new()
    }
    pub fn drives() -> Vec<crate::storage::Drive> {
        Vec::new()
    }
    pub fn has_smart_capable_drive() -> bool {
        false
    }
    pub fn is_elevated() -> bool {
        false
    }
    pub fn os_info() -> Option<super::OsInfo> {
        None
    }
    pub fn uptime_secs() -> Option<u64> {
        None
    }
    pub fn battery() -> Option<super::Battery> {
        None
    }
    pub fn microcode_revision() -> Option<u32> {
        None
    }
    pub fn power_plan() -> Option<super::PowerPlan> {
        None
    }
    pub fn topology_buffer() -> Option<Vec<u8>> {
        None
    }
    pub fn smbios_buffer() -> Option<Vec<u8>> {
        None
    }
    pub fn nominal_mhz() -> Option<u32> {
        None
    }
    pub fn memory_status() -> Option<MemoryStatus> {
        None
    }
}

/// Physical drives with volumes (and NVMe health when permitted).
pub use imp::drives;
/// Hardware display adapters (DXGI), with NVML details on NVIDIA cards.
pub use imp::gpu_adapters;
/// Whether a drive could report ATA SMART (needs administrator rights).
pub use imp::has_smart_capable_drive;
/// Whether this process runs elevated (administrator).
pub use imp::is_elevated;
pub use imp::memory_status;
/// Nominal (base) clock from `CallNtPowerInformation`, max over all logical processors.
pub use imp::nominal_mhz;
/// Raw `RSMB` firmware table (RawSMBIOSData).
pub use imp::smbios_buffer;
/// Raw `GetLogicalProcessorInformationEx(RelationAll)` buffer.
pub use imp::topology_buffer;
pub use imp::{battery, microcode_revision, os_info, power_plan, uptime_secs};

#[cfg(windows)]
mod battery_wear;
/// Data collectors for the setup checks of the Health Check.
pub mod checkup;
/// Battery design vs full-charge capacity (None on desktops).
#[cfg(windows)]
pub use battery_wear::battery_health;
#[cfg(not(windows))]
pub fn battery_health() -> Option<crate::battery::BatteryHealth> {
    None
}
