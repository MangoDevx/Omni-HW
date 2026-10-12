//! Windows data sources. Every function has a non-Windows stub returning `None`
//! so the crate (and its parser tests) build anywhere.

#[cfg(windows)]
mod dxgi;
#[cfg(windows)]
pub(crate) mod ffi;
#[cfg(windows)]
pub mod nvml;
#[cfg(windows)]
mod pdh;
pub mod perf;
#[cfg(windows)]
pub(crate) mod registry;
pub mod settings;
#[cfg(windows)]
pub mod storage;

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
mod imp {
    use super::ffi::*;
    use super::{Battery, MemoryStatus, OsInfo, PlanKind, PowerPlan};
    use crate::gpu::GpuAdapter;

    pub fn drives() -> Vec<crate::storage::Drive> {
        super::storage::drives()
    }

    pub fn has_smart_capable_drive() -> bool {
        super::storage::has_smart_capable_drive()
    }

    pub fn is_elevated() -> bool {
        const TOKEN_QUERY: u32 = 0x8;
        const TOKEN_ELEVATION: i32 = 20;
        let mut token = ptr::null_mut();
        // SAFETY: pseudo-handle for the current process; token closed below.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return false;
        }
        let mut elevated: u32 = 0;
        let mut len = 0u32;
        // SAFETY: 4-byte TOKEN_ELEVATION out buffer.
        let ok = unsafe {
            GetTokenInformation(
                token,
                TOKEN_ELEVATION,
                (&mut elevated as *mut u32).cast(),
                4,
                &mut len,
            )
        };
        // SAFETY: token opened above.
        unsafe { CloseHandle(token) };
        ok != 0 && elevated != 0
    }

    fn reg_string(subkey: &str, value: &str) -> Option<String> {
        let (k, v) = (wide(subkey), wide(value));
        let mut buf = [0u16; 256];
        let mut len = (buf.len() * 2) as u32;
        // SAFETY: buffer size in bytes passed in `len`.
        let st = unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                k.as_ptr(),
                v.as_ptr(),
                RRF_RT_REG_SZ,
                ptr::null_mut(),
                buf.as_mut_ptr().cast(),
                &mut len,
            )
        };
        (st == 0).then(|| {
            String::from_utf16_lossy(&buf[..buf.iter().position(|&c| c == 0).unwrap_or(0)])
        })
    }

    fn reg_dword(subkey: &str, value: &str) -> Option<u32> {
        let (k, v) = (wide(subkey), wide(value));
        let mut d: u32 = 0;
        let mut len = 4u32;
        // SAFETY: 4-byte buffer.
        let st = unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                k.as_ptr(),
                v.as_ptr(),
                RRF_RT_REG_DWORD,
                ptr::null_mut(),
                (&mut d as *mut u32).cast(),
                &mut len,
            )
        };
        (st == 0).then_some(d)
    }

    pub fn os_info() -> Option<OsInfo> {
        const KEY: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";
        // SAFETY: zeroed POD with size set as required.
        let mut v: OSVERSIONINFOW = unsafe { std::mem::zeroed() };
        v.size = size_of::<OSVERSIONINFOW>() as u32;
        let build = if unsafe { RtlGetVersion(&mut v) } == 0 {
            v.build
        } else {
            0
        };
        let product = reg_string(KEY, "ProductName").unwrap_or_else(|| "Windows".into());
        let mut name = [0u16; 256];
        let mut len = name.len() as u32;
        // ComputerNamePhysicalDnsHostname = 5. SAFETY: length passed in characters.
        let computer_name = if unsafe { GetComputerNameExW(5, name.as_mut_ptr(), &mut len) } != 0 {
            String::from_utf16_lossy(&name[..len as usize])
        } else {
            String::new()
        };
        Some(OsInfo {
            product: crate::windows_product_name(&product, build),
            display_version: reg_string(KEY, "DisplayVersion").unwrap_or_default(),
            build,
            ubr: reg_dword(KEY, "UBR").unwrap_or(0),
            computer_name,
        })
    }

    pub fn uptime_secs() -> Option<u64> {
        // SAFETY: trivially safe.
        Some(unsafe { GetTickCount64() } / 1000)
    }

    pub fn microcode_revision() -> Option<u32> {
        let (k, v) = (
            wide(r"HARDWARE\DESCRIPTION\System\CentralProcessor\0"),
            wide("Update Revision"),
        );
        let mut buf = [0u8; 16];
        let mut len = buf.len() as u32;
        // SAFETY: buffer size passed in `len`.
        let st = unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                k.as_ptr(),
                v.as_ptr(),
                RRF_RT_REG_BINARY,
                ptr::null_mut(),
                buf.as_mut_ptr().cast(),
                &mut len,
            )
        };
        if st != 0 {
            return None;
        }
        crate::parse_microcode(&buf[..len as usize])
    }

    pub fn power_plan() -> Option<PowerPlan> {
        let mut g: *mut GUID = ptr::null_mut();
        // SAFETY: out pointer; the GUID is freed with LocalFree below.
        if unsafe { PowerGetActiveScheme(0, &mut g) } != 0 || g.is_null() {
            return None;
        }
        // SAFETY: g points to a GUID allocated by PowerGetActiveScheme.
        let guid = unsafe { &*g };
        let text = format!(
            "{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{}",
            guid.data1,
            guid.data2,
            guid.data3,
            guid.data4[0],
            guid.data4[1],
            guid.data4[2..]
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        );
        let mut name = [0u16; 128];
        let mut len = (name.len() * 2) as u32;
        // SAFETY: buffer size in bytes passed in `len`.
        let ok = unsafe {
            PowerReadFriendlyName(
                0,
                g,
                ptr::null(),
                ptr::null(),
                name.as_mut_ptr().cast(),
                &mut len,
            )
        } == 0;
        // SAFETY: allocated by PowerGetActiveScheme.
        unsafe { LocalFree(g.cast()) };
        let name = if ok {
            String::from_utf16_lossy(&name[..name.iter().position(|&c| c == 0).unwrap_or(0)])
        } else {
            String::new()
        };
        Some(PowerPlan {
            name,
            kind: PlanKind::from_guid(&text),
            guid: text,
        })
    }

    pub fn battery() -> Option<Battery> {
        let mut s = SYSTEM_POWER_STATUS::default();
        // SAFETY: valid out pointer.
        if unsafe { GetSystemPowerStatus(&mut s) } == 0
            || s.battery_flag & 128 != 0
            || s.battery_flag == 255
        {
            return None;
        }
        Some(Battery {
            percent: s.battery_life_percent.min(100),
            on_ac: s.ac_line_status == 1,
            charging: s.battery_flag & 8 != 0,
            seconds_left: (s.battery_life_time != u32::MAX).then_some(s.battery_life_time),
        })
    }

    pub fn gpu_adapters() -> Vec<GpuAdapter> {
        let mut a = super::dxgi::adapters();
        super::nvml::attach_details(&mut a);
        super::settings::attach_driver_dates(&mut a);
        a
    }
    use std::ptr;

    pub fn topology_buffer() -> Option<Vec<u8>> {
        let mut len: u32 = 0;
        // SAFETY: size query with null buffer.
        unsafe { GetLogicalProcessorInformationEx(RELATION_ALL, ptr::null_mut(), &mut len) };
        loop {
            let mut buf = vec![0u8; len as usize];
            // SAFETY: buffer is `len` bytes.
            let ok = unsafe {
                GetLogicalProcessorInformationEx(RELATION_ALL, buf.as_mut_ptr(), &mut len)
            };
            if ok != 0 {
                buf.truncate(len as usize);
                return Some(buf);
            }
            // SAFETY: trivially safe.
            if unsafe { GetLastError() } != ERROR_INSUFFICIENT_BUFFER {
                return None;
            }
        }
    }

    pub fn smbios_buffer() -> Option<Vec<u8>> {
        // SAFETY: size query with null buffer.
        let size = unsafe { GetSystemFirmwareTable(RSMB, 0, ptr::null_mut(), 0) };
        if size == 0 {
            return None;
        }
        let mut buf = vec![0u8; size as usize];
        // SAFETY: buffer is `size` bytes.
        let got = unsafe { GetSystemFirmwareTable(RSMB, 0, buf.as_mut_ptr(), size) };
        if got == 0 || got > size {
            return None;
        }
        buf.truncate(got as usize);
        Some(buf)
    }

    pub fn nominal_mhz() -> Option<u32> {
        // SAFETY: trivially safe.
        let n = unsafe { GetActiveProcessorCount(ALL_PROCESSOR_GROUPS) }.max(1) as usize;
        let mut info = vec![PROCESSOR_POWER_INFORMATION::default(); n];
        let bytes = (n * size_of::<PROCESSOR_POWER_INFORMATION>()) as u32;
        // SAFETY: output buffer holds `n` entries.
        let st = unsafe {
            CallNtPowerInformation(
                PROCESSOR_INFORMATION,
                ptr::null_mut(),
                0,
                info.as_mut_ptr().cast(),
                bytes,
            )
        };
        if st != STATUS_SUCCESS {
            return None;
        }
        info.iter().map(|p| p.MaxMhz).max().filter(|&m| m > 0)
    }

    pub fn memory_status() -> Option<MemoryStatus> {
        let mut m = MEMORYSTATUSEX {
            dwLength: size_of::<MEMORYSTATUSEX>() as u32,
            ..Default::default()
        };
        // SAFETY: dwLength initialized as required.
        if unsafe { GlobalMemoryStatusEx(&mut m) } == 0 {
            return None;
        }
        Some(MemoryStatus {
            load_percent: m.dwMemoryLoad,
            total_bytes: m.ullTotalPhys,
            available_bytes: m.ullAvailPhys,
        })
    }
}

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
/// Battery design vs full-charge capacity (None on desktops).
#[cfg(windows)]
pub use battery_wear::battery_health;
#[cfg(not(windows))]
pub fn battery_health() -> Option<crate::battery::BatteryHealth> {
    None
}
