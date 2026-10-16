//! Battery design and full-charge capacity via SetupAPI + battery IOCTLs (no admin needed).

use super::device::Handle;
use super::wide::from_wide;
use crate::battery::{BatteryHealth, parse_battery_information};
use std::mem::offset_of;
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    DIGCF_DEVICEINTERFACE, DIGCF_PRESENT, GUID_DEVCLASS_BATTERY, SP_DEVICE_INTERFACE_DATA,
    SP_DEVICE_INTERFACE_DETAIL_DATA_W, SetupDiEnumDeviceInterfaces, SetupDiGetClassDevsW,
    SetupDiGetDeviceInterfaceDetailW,
};
use windows::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE};
use windows::Win32::System::Power::{
    BATTERY_QUERY_INFORMATION, BatteryInformation, IOCTL_BATTERY_QUERY_INFORMATION,
    IOCTL_BATTERY_QUERY_TAG,
};
use windows::core::{Owned, PCWSTR};

/// Device interface paths of present batteries.
fn battery_paths() -> Vec<String> {
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
    // SAFETY: the set was just created and is destroyed exactly once, on drop.
    let set = unsafe { Owned::new(set) };
    for index in 0..8 {
        let mut data = SP_DEVICE_INTERFACE_DATA {
            cbSize: size_of::<SP_DEVICE_INTERFACE_DATA>() as u32,
            ..Default::default()
        };
        // SAFETY: valid set; cbSize of the out struct is set as required.
        if unsafe {
            SetupDiEnumDeviceInterfaces(*set, None, &GUID_DEVCLASS_BATTERY, index, &mut data)
        }
        .is_err()
        {
            break;
        }
        let mut required = 0u32;
        // SAFETY: size query with no buffer; the expected failure is ignored.
        let _ = unsafe {
            SetupDiGetDeviceInterfaceDetailW(*set, &data, None, 0, Some(&mut required), None)
        };
        const PATH_AT: usize = offset_of!(SP_DEVICE_INTERFACE_DETAIL_DATA_W, DevicePath);
        if (required as usize) < size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() {
            continue;
        }
        // u64 storage keeps the detail struct 8-byte aligned.
        let mut buf = vec![0u64; (required as usize).div_ceil(8)];
        let detail = buf.as_mut_ptr().cast::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>();
        // SAFETY: the aligned buffer is at least `required` bytes, which covers the fixed
        // header checked above.
        unsafe { (*detail).cbSize = size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32 };
        // SAFETY: the buffer holds `required` bytes, the size passed in.
        if unsafe {
            SetupDiGetDeviceInterfaceDetailW(*set, &data, Some(detail), required, None, None)
        }
        .is_err()
        {
            continue;
        }
        // DevicePath is a NUL-terminated UTF-16 string running to the end of the buffer.
        // SAFETY: PATH_AT < required <= buffer size, so the u16 slice stays inside `buf`
        // (whose u64 storage is suitably aligned) and was initialized (zeroed, then filled).
        let path = unsafe {
            std::slice::from_raw_parts(
                detail.cast::<u8>().add(PATH_AT).cast::<u16>(),
                (required as usize - PATH_AT) / 2,
            )
        };
        out.push(from_wide(path));
    }
    out
}

fn query(path: &str) -> Option<BatteryHealth> {
    let h = Handle::open(path, GENERIC_READ.0 | GENERIC_WRITE.0)?;
    let timeout = 0u32;
    let mut tag = [0u8; 4];
    h.ioctl(IOCTL_BATTERY_QUERY_TAG, &timeout, &mut tag)?;
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
    let n = h.ioctl(IOCTL_BATTERY_QUERY_INFORMATION, &q, &mut info)?;
    parse_battery_information(&info[..n])
}

/// Wear of the first battery that reports capacities; None on desktops.
pub fn battery_health() -> Option<BatteryHealth> {
    battery_paths().iter().find_map(|p| query(p))
}
