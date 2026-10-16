//! Windows implementations of the system-level queries re-exported from [`crate::win`]: OS
//! version, elevation, memory, battery, power plan, topology and SMBIOS buffers.

use super::registry::{self, Hive};
use super::wide::{from_wide, guid_text};
use super::{Battery, MemoryStatus, OsInfo, PlanKind, PowerPlan};
use crate::gpu::GpuAdapter;
use windows::Wdk::System::SystemServices::RtlGetVersion;
use windows::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, HANDLE, STATUS_SUCCESS};
use windows::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
use windows::Win32::System::Power::{
    CallNtPowerInformation, GetSystemPowerStatus, PROCESSOR_POWER_INFORMATION,
    ProcessorInformation, SYSTEM_POWER_STATUS,
};
use windows::Win32::System::Registry::RRF_RT_REG_BINARY;
use windows::Win32::System::SystemInformation::{
    ComputerNamePhysicalDnsHostname, GetComputerNameExW, GetLogicalProcessorInformationEx,
    GetSystemFirmwareTable, GetTickCount64, GlobalMemoryStatusEx, MEMORYSTATUSEX, OSVERSIONINFOW,
    RSMB, RelationAll,
};
use windows::Win32::System::Threading::{
    ALL_PROCESSOR_GROUPS, GetActiveProcessorCount, GetCurrentProcess, OpenProcessToken,
};
use windows::core::{Owned, PWSTR};

pub fn drives() -> Vec<crate::storage::Drive> {
    super::storage::drives()
}

pub fn has_smart_capable_drive() -> bool {
    super::storage::has_smart_capable_drive()
}

pub fn is_elevated() -> bool {
    let mut token = HANDLE::default();
    // SAFETY: GetCurrentProcess returns a pseudo-handle that needs no closing; valid out
    // pointer to a local.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }.is_err() {
        return false;
    }
    // SAFETY: OpenProcessToken succeeded, so `token` is ours to close (on drop).
    let token = unsafe { Owned::new(token) };
    let mut elevation = TOKEN_ELEVATION::default();
    let mut len = 0u32;
    // SAFETY: the out buffer is a TOKEN_ELEVATION of the declared size; valid token handle.
    let ok = unsafe {
        GetTokenInformation(
            *token,
            TokenElevation,
            Some((&raw mut elevation).cast()),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        )
    }
    .is_ok();
    ok && elevation.TokenIsElevated != 0
}

pub fn os_info() -> Option<OsInfo> {
    const KEY: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";
    let mut v = OSVERSIONINFOW {
        dwOSVersionInfoSize: size_of::<OSVERSIONINFOW>() as u32,
        ..Default::default()
    };
    // SAFETY: the struct's size field is set as RtlGetVersion requires.
    let build = if unsafe { RtlGetVersion(&mut v) } == STATUS_SUCCESS {
        v.dwBuildNumber
    } else {
        0
    };
    let product = registry::string(Hive::LocalMachine, KEY, "ProductName")
        .unwrap_or_else(|| "Windows".into());
    let mut name = [0u16; 256];
    let mut len = name.len() as u32;
    // SAFETY: the buffer length is passed in characters in `len`.
    let computer_name = if unsafe {
        GetComputerNameExW(
            ComputerNamePhysicalDnsHostname,
            Some(PWSTR(name.as_mut_ptr())),
            &mut len,
        )
    }
    .is_ok()
    {
        from_wide(&name[..(len as usize).min(name.len())])
    } else {
        String::new()
    };
    Some(OsInfo {
        product: crate::windows_product_name(&product, build),
        display_version: registry::string(Hive::LocalMachine, KEY, "DisplayVersion")
            .unwrap_or_default(),
        build,
        ubr: registry::dword(Hive::LocalMachine, KEY, "UBR").unwrap_or(0),
        computer_name,
    })
}

pub fn uptime_secs() -> Option<u64> {
    // SAFETY: no arguments; always succeeds.
    Some(unsafe { GetTickCount64() } / 1000)
}

pub fn microcode_revision() -> Option<u32> {
    let mut buf = [0u8; 16];
    let len = registry::get(
        Hive::LocalMachine,
        r"HARDWARE\DESCRIPTION\System\CentralProcessor\0",
        "Update Revision",
        RRF_RT_REG_BINARY,
        &mut buf,
    )?;
    crate::parse_microcode(&buf[..len])
}

pub fn power_plan() -> Option<PowerPlan> {
    let g = super::power::active_scheme()?;
    let guid = guid_text(&g);
    Some(PowerPlan {
        name: super::power::scheme_name(&g),
        kind: PlanKind::from_guid(&guid),
        guid,
    })
}

pub fn battery() -> Option<Battery> {
    let mut s = SYSTEM_POWER_STATUS::default();
    // SAFETY: valid out pointer to a local.
    if unsafe { GetSystemPowerStatus(&mut s) }.is_err()
        || s.BatteryFlag & 128 != 0
        || s.BatteryFlag == 255
    {
        return None;
    }
    Some(Battery {
        percent: s.BatteryLifePercent.min(100),
        on_ac: s.ACLineStatus == 1,
        charging: s.BatteryFlag & 8 != 0,
        seconds_left: (s.BatteryLifeTime != u32::MAX).then_some(s.BatteryLifeTime),
    })
}

pub fn gpu_adapters() -> Vec<GpuAdapter> {
    let mut a = super::dxgi::adapters();
    super::nvml::attach_details(&mut a);
    super::settings::attach_driver_dates(&mut a);
    a
}

pub fn topology_buffer() -> Option<Vec<u8>> {
    let mut len: u32 = 0;
    // SAFETY: size query with no buffer; the expected ERROR_INSUFFICIENT_BUFFER is ignored.
    let _ = unsafe { GetLogicalProcessorInformationEx(RelationAll, None, &mut len) };
    // The topology can change (CPU hot-add) between calls; retry until the buffer fits.
    loop {
        let mut buf = vec![0u8; len as usize];
        // SAFETY: the buffer holds `len` bytes, the size passed in.
        let r = unsafe {
            GetLogicalProcessorInformationEx(RelationAll, Some(buf.as_mut_ptr().cast()), &mut len)
        };
        match r {
            Ok(()) => {
                buf.truncate(len as usize);
                return Some(buf);
            }
            Err(e) if e.code() == ERROR_INSUFFICIENT_BUFFER.to_hresult() => {}
            Err(_) => return None,
        }
    }
}

pub fn smbios_buffer() -> Option<Vec<u8>> {
    // SAFETY: size query with no buffer.
    let size = unsafe { GetSystemFirmwareTable(RSMB, 0, None) };
    if size == 0 {
        return None;
    }
    let mut buf = vec![0u8; size as usize];
    // SAFETY: the buffer length is passed with the slice.
    let got = unsafe { GetSystemFirmwareTable(RSMB, 0, Some(&mut buf)) };
    if got == 0 || got > size {
        return None;
    }
    buf.truncate(got as usize);
    Some(buf)
}

pub fn nominal_mhz() -> Option<u32> {
    // SAFETY: no pointers involved.
    let n = unsafe { GetActiveProcessorCount(ALL_PROCESSOR_GROUPS) }.max(1) as usize;
    let mut info = vec![PROCESSOR_POWER_INFORMATION::default(); n];
    let bytes = (n * size_of::<PROCESSOR_POWER_INFORMATION>()) as u32;
    // SAFETY: the output buffer holds `n` entries, `bytes` bytes in total.
    let st = unsafe {
        CallNtPowerInformation(
            ProcessorInformation,
            None,
            0,
            Some(info.as_mut_ptr().cast()),
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
    // SAFETY: dwLength is initialized as GlobalMemoryStatusEx requires.
    unsafe { GlobalMemoryStatusEx(&mut m) }.ok()?;
    Some(MemoryStatus {
        load_percent: m.dwMemoryLoad,
        total_bytes: m.ullTotalPhys,
        available_bytes: m.ullAvailPhys,
    })
}
