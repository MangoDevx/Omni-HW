//! Battery design and full-charge capacity via SetupAPI + battery IOCTLs (no admin needed).

use crate::battery::{BatteryHealth, parse_battery_information};
use std::mem::offset_of;
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    DIGCF_DEVICEINTERFACE, DIGCF_PRESENT, GUID_DEVCLASS_BATTERY, SP_DEVICE_INTERFACE_DATA,
    SP_DEVICE_INTERFACE_DETAIL_DATA_W, SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces,
    SetupDiGetClassDevsW, SetupDiGetDeviceInterfaceDetailW,
};
use windows::Win32::Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::IO::DeviceIoControl;
use windows::Win32::System::Power::{
    BATTERY_QUERY_INFORMATION, BatteryInformation, IOCTL_BATTERY_QUERY_INFORMATION,
    IOCTL_BATTERY_QUERY_TAG,
};
use windows::core::PCWSTR;

/// Device paths of present batteries (NUL-terminated UTF-16).
fn battery_paths() -> Vec<Vec<u16>> {
    let mut out = Vec::new();
    // SAFETY: valid class GUID; no enumerator or parent window.
    let Ok(set) = (unsafe {
        SetupDiGetClassDevsW(
            Some(&GUID_DEVCLASS_BATTERY),
            PCWSTR::null(),
            None,
            DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
        )
    }) else {
        return out;
    };
    for index in 0..8 {
        let mut data = SP_DEVICE_INTERFACE_DATA {
            cbSize: size_of::<SP_DEVICE_INTERFACE_DATA>() as u32,
            ..Default::default()
        };
        // SAFETY: valid set and out struct.
        if unsafe {
            SetupDiEnumDeviceInterfaces(set, None, &GUID_DEVCLASS_BATTERY, index, &mut data)
        }
        .is_err()
        {
            break;
        }
        let mut required = 0u32;
        // SAFETY: size query with no buffer; the expected failure is ignored.
        let _ = unsafe {
            SetupDiGetDeviceInterfaceDetailW(set, &data, None, 0, Some(&mut required), None)
        };
        const PATH_AT: usize = offset_of!(SP_DEVICE_INTERFACE_DETAIL_DATA_W, DevicePath);
        if (required as usize) < size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() {
            continue;
        }
        // u64 storage keeps the detail struct 8-byte aligned.
        let mut buf = vec![0u64; (required as usize).div_ceil(8)];
        let detail = buf.as_mut_ptr() as *mut SP_DEVICE_INTERFACE_DETAIL_DATA_W;
        // SAFETY: buffer is at least `required` bytes, which covers the fixed header.
        unsafe { (*detail).cbSize = size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32 };
        // SAFETY: buffer size passed explicitly.
        if unsafe {
            SetupDiGetDeviceInterfaceDetailW(set, &data, Some(detail), required, None, None)
        }
        .is_err()
        {
            continue;
        }
        // DevicePath is a NUL-terminated UTF-16 string running to the end of the buffer.
        // SAFETY: within the buffer filled above.
        let path = unsafe {
            std::slice::from_raw_parts(
                detail.cast::<u8>().add(PATH_AT).cast::<u16>(),
                (required as usize - PATH_AT) / 2,
            )
        };
        let len = path.iter().position(|&c| c == 0).unwrap_or(path.len());
        let mut p = path[..len].to_vec();
        p.push(0);
        out.push(p);
    }
    // SAFETY: set from SetupDiGetClassDevsW, destroyed once.
    let _ = unsafe { SetupDiDestroyDeviceInfoList(set) };
    out
}

fn ioctl<I: ?Sized>(h: HANDLE, code: u32, input: &I, out: &mut [u8]) -> Option<usize> {
    let mut got = 0;
    // SAFETY: buffers valid for their lengths.
    unsafe {
        DeviceIoControl(
            h,
            code,
            Some((input as *const I).cast()),
            size_of_val(input) as u32,
            Some(out.as_mut_ptr().cast()),
            out.len() as u32,
            Some(&mut got),
            None,
        )
    }
    .ok()?;
    Some(got as usize)
}

fn query(path: &[u16]) -> Option<BatteryHealth> {
    // SAFETY: NUL-terminated device path.
    let h = unsafe {
        CreateFileW(
            PCWSTR(path.as_ptr()),
            (GENERIC_READ | GENERIC_WRITE).0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
    }
    .ok()?;
    let result = (|| {
        let timeout = 0u32;
        let mut tag = [0u8; 4];
        ioctl(h, IOCTL_BATTERY_QUERY_TAG, &timeout, &mut tag)?;
        let tag = u32::from_le_bytes(tag);
        if tag == 0 {
            return None;
        }
        let q = BATTERY_QUERY_INFORMATION {
            BatteryTag: tag,
            InformationLevel: BatteryInformation,
            AtRate: 0,
        };
        // Sized for BATTERY_INFORMATION, which `parse_battery_information` decodes.
        let mut info = [0u8; 36];
        let n = ioctl(h, IOCTL_BATTERY_QUERY_INFORMATION, &q, &mut info)?;
        parse_battery_information(&info[..n])
    })();
    // SAFETY: handle opened above.
    let _ = unsafe { CloseHandle(h) };
    result
}

/// Wear of the first battery that reports capacities; None on desktops.
pub fn battery_health() -> Option<BatteryHealth> {
    battery_paths().iter().find_map(|p| query(p))
}
