//! Omni-HW core: hardware detection and data model.
//!
//! Parsers (`cpuid`, `codename`, `topology`, `smbios`) are platform-agnostic;
//! Windows calls live in [`win`] with non-Windows stubs.

pub mod alerts;
pub mod battery;
pub mod bios_help;
pub mod bottleneck;
pub mod checkup;
pub mod codename;
pub mod cpuid;
pub mod diagnostics;
pub mod disktest;
pub mod gpu;
pub mod lansec;
mod le;
pub mod memtest;
pub mod network;
pub mod power;
pub mod recording;
pub mod report;
pub mod sensors;
pub mod smart;
pub mod smbios;
pub mod storage;
pub mod topology;
pub mod win;

use codename::Codename;
use cpuid::CpuInfo;
use gpu::GpuAdapter;
use smbios::Smbios;
use storage::Drive;
use topology::Topology;
use win::{Battery, MemoryStatus, OsInfo, PowerPlan};

/// Static system information, gathered once at startup.
#[derive(Debug, Clone, Default)]
pub struct SystemInfo {
    pub cpu: CpuInfo,
    pub codename: Option<Codename>,
    pub topology: Option<Topology>,
    pub smbios: Option<Smbios>,
    /// Nominal clock in MHz.
    pub base_mhz: Option<u32>,
    pub memory: Option<MemoryStatus>,
    pub gpus: Vec<GpuAdapter>,
    pub drives: Vec<Drive>,
    pub os: Option<OsInfo>,
    pub battery: Option<Battery>,
    /// Battery wear (design vs full-charge capacity), laptops only.
    pub battery_health: Option<battery::BatteryHealth>,
    /// CPU microcode revision loaded by Windows.
    pub microcode: Option<u32>,
    pub power_plan: Option<PowerPlan>,
    /// Windows settings that affect performance and stability.
    pub windows: win::settings::WindowsSettings,
    /// Firmware, device, event-log and setup data for the Health Check.
    pub checkup: checkup::Checkup,
    /// Non-fatal problems encountered while gathering.
    pub warnings: Vec<String>,
}

impl SystemInfo {
    pub fn gather() -> SystemInfo {
        // The Health Check data (event log, device tree, display modes) is the slowest part;
        // collect it alongside everything else.
        std::thread::scope(|s| {
            let checkup = s.spawn(win::checkup::gather);
            let mut info = SystemInfo::gather_hardware();
            info.checkup = checkup.join().unwrap_or_default();
            info
        })
    }

    fn gather_hardware() -> SystemInfo {
        let mut warnings = Vec::new();
        let cpu = cpuid::decode(&cpuid::NativeCpuid);
        let codename = codename::lookup(cpu.vendor, cpu.family, cpu.model);

        let topology = match win::topology_buffer().map(|b| Topology::parse(&b)) {
            Some(Ok(t)) => Some(t),
            Some(Err(e)) => {
                warnings.push(format!("Topology: {e}"));
                None
            }
            None => {
                warnings.push("Topology: GetLogicalProcessorInformationEx unavailable".into());
                None
            }
        };

        let smbios = match win::smbios_buffer().map(|b| Smbios::parse_raw(&b)) {
            Some(Ok(s)) => Some(s),
            Some(Err(e)) => {
                warnings.push(format!("SMBIOS: {e}"));
                None
            }
            None => {
                warnings.push("SMBIOS: firmware table unavailable".into());
                None
            }
        };

        SystemInfo {
            cpu,
            codename,
            topology,
            smbios,
            base_mhz: win::nominal_mhz(),
            memory: win::memory_status(),
            gpus: win::gpu_adapters(),
            drives: win::drives(),
            os: win::os_info(),
            battery: win::battery(),
            battery_health: win::battery_health(),
            microcode: win::microcode_revision(),
            power_plan: win::power_plan(),
            windows: win::settings::windows_settings(),
            checkup: checkup::Checkup::default(),
            warnings,
        }
    }
}

/// The registry `ProductName` still says "Windows 10" on Windows 11; fix it up by build number.
pub fn windows_product_name(product: &str, build: u32) -> String {
    if build >= 22000 && product.starts_with("Windows 10") {
        product.replacen("Windows 10", "Windows 11", 1)
    } else {
        product.to_string()
    }
}

/// "3d 4h 12m" style duration.
pub fn format_uptime(secs: u64) -> String {
    let (d, h, m) = (secs / 86400, secs / 3600 % 24, secs / 60 % 60);
    if d > 0 {
        format!("{d}d {h}h {m}m")
    } else if h > 0 {
        format!("{h}h {m}m")
    } else {
        format!("{m}m")
    }
}

/// Gregorian (year, month, day) for days since 1970-01-01 (Howard Hinnant's civil_from_days).
pub fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// The `Update Revision` registry value: 4 bytes on current Windows, 8 bytes (revision in
/// the high DWORD) on older builds.
pub fn parse_microcode(b: &[u8]) -> Option<u32> {
    let dw = |o: usize| le::u32_at(b, o);
    match b.len() {
        8 => dw(4).filter(|&v| v != 0).or_else(|| dw(0)),
        4 => dw(0),
        _ => None,
    }
    .filter(|&v| v != 0)
}

/// "48 KB", "2 MB", "1.25 MB"...
pub fn format_bytes(b: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    let (v, unit) = match b {
        _ if b >= GB => (b as f64 / GB as f64, "GB"),
        _ if b >= MB => (b as f64 / MB as f64, "MB"),
        _ if b >= KB => (b as f64 / KB as f64, "KB"),
        _ => return format!("{b} B"),
    };
    if v.fract() == 0.0 {
        format!("{v:.0} {unit}")
    } else {
        format!("{v:.2} {unit}").replace(".00", "")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn product_names() {
        assert_eq!(
            windows_product_name("Windows 10 Pro", 26300),
            "Windows 11 Pro"
        );
        assert_eq!(
            windows_product_name("Windows 10 Pro", 19045),
            "Windows 10 Pro"
        );
        assert_eq!(
            windows_product_name("Windows Server 2022", 20348),
            "Windows Server 2022"
        );
    }

    #[test]
    fn civil_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_373), (2025, 10, 12));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
    }

    #[test]
    fn microcode() {
        assert_eq!(parse_microcode(&[0x2B, 0x01, 0, 0]), Some(0x12B));
        assert_eq!(
            parse_microcode(&[0, 0, 0, 0, 0x29, 0x01, 0, 0]),
            Some(0x129)
        );
        assert_eq!(parse_microcode(&[0, 0, 0, 0]), None);
        assert_eq!(parse_microcode(&[1, 2]), None);
    }

    #[test]
    fn uptime() {
        assert_eq!(format_uptime(59), "0m");
        assert_eq!(format_uptime(3 * 3600 + 120), "3h 2m");
        assert_eq!(format_uptime(2 * 86400 + 5 * 3600), "2d 5h 0m");
    }

    #[test]
    fn bytes() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(48 * 1024), "48 KB");
        assert_eq!(format_bytes(1280 * 1024), "1.25 MB");
        assert_eq!(format_bytes(36 * 1024 * 1024), "36 MB");
        assert_eq!(format_bytes(32 * 1024 * 1024 * 1024), "32 GB");
    }
}
