//! Battery design and full-charge capacity via SetupAPI + battery IOCTLs (no admin needed).

#![allow(non_snake_case, clippy::upper_case_acronyms)]

use super::ffi::*;
use crate::battery::{BatteryHealth, parse_battery_information};
use std::ffi::c_void;
use std::ptr;

/// GUID_DEVCLASS_BATTERY {72631e54-78a4-11d0-bcf7-00aa00b7b32a}, also the battery interface class.
const GUID_DEVCLASS_BATTERY: GUID = GUID {
    data1: 0x7263_1e54,
    data2: 0x78a4,
    data3: 0x11d0,
    data4: [0xbc, 0xf7, 0x00, 0xaa, 0x00, 0xb7, 0xb3, 0x2a],
};
const DIGCF_PRESENT: u32 = 0x2;
const DIGCF_DEVICEINTERFACE: u32 = 0x10;
const IOCTL_BATTERY_QUERY_TAG: u32 = 0x0029_4040;
const IOCTL_BATTERY_QUERY_INFORMATION: u32 = 0x0029_4044;
const BATTERY_INFORMATION_LEVEL: u32 = 0;
const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;

#[repr(C)]
struct SP_DEVICE_INTERFACE_DATA {
    cbSize: u32,
    InterfaceClassGuid: GUID,
    Flags: u32,
    Reserved: usize,
}

#[link(name = "setupapi")]
unsafe extern "system" {
    fn SetupDiGetClassDevsW(
        class: *const GUID,
        enumerator: *const u16,
        parent: isize,
        flags: u32,
    ) -> isize;
    fn SetupDiEnumDeviceInterfaces(
        set: isize,
        devinfo: *mut c_void,
        class: *const GUID,
        index: u32,
        data: *mut SP_DEVICE_INTERFACE_DATA,
    ) -> BOOL;
    fn SetupDiGetDeviceInterfaceDetailW(
        set: isize,
        data: *const SP_DEVICE_INTERFACE_DATA,
        detail: *mut u8,
        size: u32,
        required: *mut u32,
        devinfo: *mut c_void,
    ) -> BOOL;
    fn SetupDiDestroyDeviceInfoList(set: isize) -> BOOL;
}

/// Device paths of present batteries.
fn battery_paths() -> Vec<Vec<u16>> {
    let mut out = Vec::new();
    // SAFETY: valid class GUID; no enumerator or parent window.
    let set = unsafe {
        SetupDiGetClassDevsW(
            &GUID_DEVCLASS_BATTERY,
            ptr::null(),
            0,
            DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
        )
    };
    if set == -1 {
        return out;
    }
    for index in 0..8 {
        // SAFETY: zeroed POD with cbSize set as required.
        let mut data: SP_DEVICE_INTERFACE_DATA = unsafe { std::mem::zeroed() };
        data.cbSize = size_of::<SP_DEVICE_INTERFACE_DATA>() as u32;
        // SAFETY: valid set and out struct.
        if unsafe {
            SetupDiEnumDeviceInterfaces(
                set,
                ptr::null_mut(),
                &GUID_DEVCLASS_BATTERY,
                index,
                &mut data,
            )
        } == 0
        {
            break;
        }
        let mut required = 0u32;
        // SAFETY: size query with null buffer.
        unsafe {
            SetupDiGetDeviceInterfaceDetailW(
                set,
                &data,
                ptr::null_mut(),
                0,
                &mut required,
                ptr::null_mut(),
            )
        };
        if required < 8 {
            continue;
        }
        // u64 storage keeps the detail struct 8-byte aligned.
        let mut buf = vec![0u64; (required as usize).div_ceil(8)];
        let detail = buf.as_mut_ptr() as *mut u8;
        // SP_DEVICE_INTERFACE_DETAIL_DATA_W.cbSize is 8 on x64 (u32 + one WCHAR, padded).
        // SAFETY: buffer is at least `required` bytes.
        unsafe { *(detail as *mut u32) = 8 };
        // SAFETY: buffer size passed explicitly.
        if unsafe {
            SetupDiGetDeviceInterfaceDetailW(
                set,
                &data,
                detail,
                required,
                ptr::null_mut(),
                ptr::null_mut(),
            )
        } == 0
        {
            continue;
        }
        // DevicePath starts at offset 4 as a NUL-terminated UTF-16 string.
        // SAFETY: within the buffer filled above.
        let path = unsafe {
            std::slice::from_raw_parts((detail as *const u16).add(2), (required as usize - 4) / 2)
        };
        let len = path.iter().position(|&c| c == 0).unwrap_or(path.len());
        let mut p = path[..len].to_vec();
        p.push(0);
        out.push(p);
    }
    // SAFETY: set from SetupDiGetClassDevsW, destroyed once.
    unsafe { SetupDiDestroyDeviceInfoList(set) };
    out
}

fn ioctl(h: HANDLE, code: u32, input: &mut [u8], out: &mut [u8]) -> Option<usize> {
    let mut got = 0;
    // SAFETY: buffers valid for their lengths.
    let ok = unsafe {
        DeviceIoControl(
            h,
            code,
            input.as_mut_ptr().cast(),
            input.len() as u32,
            out.as_mut_ptr().cast(),
            out.len() as u32,
            &mut got,
            ptr::null_mut(),
        )
    };
    (ok != 0).then_some(got as usize)
}

fn query(path: &[u16]) -> Option<BatteryHealth> {
    // SAFETY: NUL-terminated device path.
    let h = unsafe {
        CreateFileW(
            path.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            ptr::null_mut(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            ptr::null_mut(),
        )
    };
    if h == INVALID_HANDLE_VALUE || h.is_null() {
        return None;
    }
    let result = (|| {
        let mut timeout = 0u32.to_le_bytes();
        let mut tag = [0u8; 4];
        ioctl(h, IOCTL_BATTERY_QUERY_TAG, &mut timeout, &mut tag)?;
        if u32::from_le_bytes(tag) == 0 {
            return None;
        }
        // BATTERY_QUERY_INFORMATION: BatteryTag, InformationLevel, AtRate.
        let mut q = [0u8; 12];
        q[0..4].copy_from_slice(&tag);
        q[4..8].copy_from_slice(&BATTERY_INFORMATION_LEVEL.to_le_bytes());
        let mut info = [0u8; 36];
        let n = ioctl(h, IOCTL_BATTERY_QUERY_INFORMATION, &mut q, &mut info)?;
        parse_battery_information(&info[..n])
    })();
    // SAFETY: handle opened above.
    unsafe { CloseHandle(h) };
    result
}

/// Wear of the first battery that reports capacities; None on desktops.
pub fn battery_health() -> Option<BatteryHealth> {
    battery_paths().iter().find_map(|p| query(p))
}
