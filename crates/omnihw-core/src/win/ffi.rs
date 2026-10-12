//! Hand-written Win32 declarations (kernel32, powrprof, pdh). x86-64 layouts only.

#![allow(non_snake_case, non_camel_case_types, clippy::upper_case_acronyms)]

use std::ffi::c_void;

pub type BOOL = i32;
pub type PDH_STATUS = u32;
pub type PDH_HQUERY = *mut c_void;
pub type PDH_HCOUNTER = *mut c_void;

pub const RELATION_ALL: i32 = 0xFFFF;
pub const ALL_PROCESSOR_GROUPS: u16 = 0xFFFF;
pub const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
/// 'RSMB' as a big-endian DWORD.
pub const RSMB: u32 = u32::from_be_bytes(*b"RSMB");

pub const PROCESSOR_INFORMATION: i32 = 11;
pub const STATUS_SUCCESS: i32 = 0;

pub const PDH_FMT_DOUBLE: u32 = 0x0000_0200;
pub const PDH_FMT_NOCAP100: u32 = 0x0000_8000;
pub const PDH_MORE_DATA: u32 = 0x8000_07D2;
pub const PDH_CSTATUS_VALID_DATA: u32 = 0x0000_0000;
pub const PDH_CSTATUS_NEW_DATA: u32 = 0x0000_0001;

#[repr(C)]
#[derive(Default)]
pub struct MEMORYSTATUSEX {
    pub dwLength: u32,
    pub dwMemoryLoad: u32,
    pub ullTotalPhys: u64,
    pub ullAvailPhys: u64,
    pub ullTotalPageFile: u64,
    pub ullAvailPageFile: u64,
    pub ullTotalVirtual: u64,
    pub ullAvailVirtual: u64,
    pub ullAvailExtendedVirtual: u64,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct PROCESSOR_POWER_INFORMATION {
    pub Number: u32,
    pub MaxMhz: u32,
    pub CurrentMhz: u32,
    pub MhzLimit: u32,
    pub MaxIdleState: u32,
    pub CurrentIdleState: u32,
}

/// `PDH_FMT_COUNTERVALUE` with the union narrowed to the `double` member.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct PDH_FMT_COUNTERVALUE {
    pub CStatus: u32,
    pub doubleValue: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PDH_FMT_COUNTERVALUE_ITEM_W {
    pub szName: *const u16,
    pub FmtValue: PDH_FMT_COUNTERVALUE,
}

#[link(name = "kernel32")]
unsafe extern "system" {
    pub fn GetLogicalProcessorInformationEx(
        relationship: i32,
        buffer: *mut u8,
        returned_length: *mut u32,
    ) -> BOOL;
    pub fn GetSystemFirmwareTable(provider: u32, table_id: u32, buffer: *mut u8, size: u32) -> u32;
    pub fn GlobalMemoryStatusEx(buffer: *mut MEMORYSTATUSEX) -> BOOL;
    pub fn GetActiveProcessorCount(group: u16) -> u32;
    pub fn GetLastError() -> u32;
}

#[link(name = "powrprof")]
unsafe extern "system" {
    pub fn CallNtPowerInformation(
        level: i32,
        input: *mut c_void,
        input_len: u32,
        output: *mut c_void,
        output_len: u32,
    ) -> i32;
}

#[link(name = "pdh")]
unsafe extern "system" {
    pub fn PdhOpenQueryW(
        data_source: *const u16,
        user_data: usize,
        query: *mut PDH_HQUERY,
    ) -> PDH_STATUS;
    pub fn PdhAddEnglishCounterW(
        query: PDH_HQUERY,
        path: *const u16,
        user_data: usize,
        counter: *mut PDH_HCOUNTER,
    ) -> PDH_STATUS;
    pub fn PdhCollectQueryData(query: PDH_HQUERY) -> PDH_STATUS;
    pub fn PdhGetFormattedCounterArrayW(
        counter: PDH_HCOUNTER,
        format: u32,
        buffer_size: *mut u32,
        item_count: *mut u32,
        items: *mut PDH_FMT_COUNTERVALUE_ITEM_W,
    ) -> PDH_STATUS;
    pub fn PdhCloseQuery(query: PDH_HQUERY) -> PDH_STATUS;
}

/// NUL-terminated UTF-16 for passing to `W` APIs.
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Read a NUL-terminated UTF-16 string.
///
/// # Safety
/// `p` must be null or point to a valid NUL-terminated UTF-16 string.
pub unsafe fn from_wide(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut len = 0;
    // SAFETY: caller guarantees NUL termination.
    unsafe {
        while *p.add(len) != 0 {
            len += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(p, len))
    }
}

pub type HMODULE = *mut c_void;
pub const LOAD_LIBRARY_SEARCH_SYSTEM32: u32 = 0x0000_0800;

#[link(name = "kernel32")]
unsafe extern "system" {
    pub fn LoadLibraryExW(name: *const u16, file: *mut c_void, flags: u32) -> HMODULE;
    pub fn GetProcAddress(module: HMODULE, name: *const u8) -> *mut c_void;
    pub fn FreeLibrary(module: HMODULE) -> BOOL;
}

#[repr(C)]
pub struct GUID {
    pub data1: u32,
    pub data2: u16,
    pub data3: u16,
    pub data4: [u8; 8],
}

#[link(name = "dxgi")]
unsafe extern "system" {
    pub fn CreateDXGIFactory1(riid: *const GUID, factory: *mut *mut c_void) -> i32;
}

#[repr(C)]
pub struct D3DKMT_OPENADAPTERFROMLUID {
    pub luid_low: u32,
    pub luid_high: i32,
    pub adapter: u32,
}

#[repr(C)]
pub struct D3DKMT_QUERYADAPTERINFO {
    pub adapter: u32,
    pub kind: u32,
    pub data: *mut c_void,
    pub data_size: u32,
}

pub const KMTQAITYPE_ADAPTERTYPE: u32 = 15;
/// `D3DKMT_ADAPTERTYPE` bits.
pub const ADAPTERTYPE_RENDER_SUPPORTED: u32 = 1 << 0;
pub const ADAPTERTYPE_SOFTWARE_DEVICE: u32 = 1 << 2;
pub const ADAPTERTYPE_INDIRECT_DISPLAY: u32 = 1 << 6;

#[link(name = "gdi32")]
unsafe extern "system" {
    pub fn D3DKMTOpenAdapterFromLuid(args: *mut D3DKMT_OPENADAPTERFROMLUID) -> i32;
    pub fn D3DKMTQueryAdapterInfo(args: *mut D3DKMT_QUERYADAPTERINFO) -> i32;
    pub fn D3DKMTCloseAdapter(args: *mut u32) -> i32;
}

pub type HANDLE = *mut c_void;
pub const INVALID_HANDLE_VALUE: HANDLE = -1isize as HANDLE;
pub const GENERIC_READ: u32 = 0x8000_0000;
pub const GENERIC_WRITE: u32 = 0x4000_0000;
pub const FILE_SHARE_READ: u32 = 0x1;
pub const FILE_SHARE_WRITE: u32 = 0x2;
pub const OPEN_EXISTING: u32 = 3;

pub const IOCTL_STORAGE_QUERY_PROPERTY: u32 = 0x002D_1400;
pub const IOCTL_STORAGE_GET_DEVICE_NUMBER: u32 = 0x002D_1080;
pub const IOCTL_DISK_GET_DRIVE_GEOMETRY_EX: u32 = 0x0007_00A0;

#[link(name = "kernel32")]
unsafe extern "system" {
    pub fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        security: *mut c_void,
        disposition: u32,
        flags: u32,
        template: HANDLE,
    ) -> HANDLE;
    pub fn CloseHandle(h: HANDLE) -> BOOL;
    pub fn DeviceIoControl(
        h: HANDLE,
        code: u32,
        input: *mut c_void,
        input_len: u32,
        output: *mut c_void,
        output_len: u32,
        returned: *mut u32,
        overlapped: *mut c_void,
    ) -> BOOL;
    pub fn GetLogicalDrives() -> u32;
    pub fn GetDiskFreeSpaceExW(
        root: *const u16,
        avail: *mut u64,
        total: *mut u64,
        free: *mut u64,
    ) -> BOOL;
    pub fn GetVolumeInformationW(
        root: *const u16,
        name: *mut u16,
        name_len: u32,
        serial: *mut u32,
        max_component: *mut u32,
        flags: *mut u32,
        fs_name: *mut u16,
        fs_len: u32,
    ) -> BOOL;
    pub fn GetTickCount64() -> u64;
    pub fn GetSystemPowerStatus(status: *mut SYSTEM_POWER_STATUS) -> BOOL;
    pub fn GetComputerNameExW(kind: i32, buf: *mut u16, len: *mut u32) -> BOOL;
}

#[repr(C)]
#[derive(Default)]
pub struct SYSTEM_POWER_STATUS {
    pub ac_line_status: u8,
    pub battery_flag: u8,
    pub battery_life_percent: u8,
    pub system_status_flag: u8,
    pub battery_life_time: u32,
    pub battery_full_life_time: u32,
}

#[repr(C)]
pub struct OSVERSIONINFOW {
    pub size: u32,
    pub major: u32,
    pub minor: u32,
    pub build: u32,
    pub platform: u32,
    pub csd: [u16; 128],
}

#[link(name = "ntdll")]
unsafe extern "system" {
    pub fn RtlGetVersion(info: *mut OSVERSIONINFOW) -> i32;
}

pub const HKEY_LOCAL_MACHINE: isize = 0x8000_0002u32 as i32 as isize;
pub const RRF_RT_REG_SZ: u32 = 0x2;
pub const RRF_RT_REG_DWORD: u32 = 0x10;

#[link(name = "advapi32")]
unsafe extern "system" {
    pub fn RegGetValueW(
        key: isize,
        subkey: *const u16,
        value: *const u16,
        flags: u32,
        kind: *mut u32,
        data: *mut c_void,
        len: *mut u32,
    ) -> i32;
}

pub const RRF_RT_REG_BINARY: u32 = 0x8;

#[link(name = "powrprof")]
unsafe extern "system" {
    pub fn PowerGetActiveScheme(user_root: isize, active: *mut *mut GUID) -> u32;
    pub fn PowerReadFriendlyName(
        root: isize,
        scheme: *const GUID,
        subgroup: *const GUID,
        setting: *const GUID,
        buffer: *mut u8,
        size: *mut u32,
    ) -> u32;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    pub fn LocalFree(mem: *mut c_void) -> *mut c_void;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    pub fn GetCurrentProcess() -> HANDLE;
}

#[link(name = "advapi32")]
unsafe extern "system" {
    pub fn OpenProcessToken(process: HANDLE, access: u32, token: *mut HANDLE) -> BOOL;
    pub fn GetTokenInformation(
        token: HANDLE,
        class: i32,
        info: *mut c_void,
        len: u32,
        returned: *mut u32,
    ) -> BOOL;
}
