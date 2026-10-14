//! Active monitors: connection type, current refresh rate and the best one Windows offers at
//! the current resolution (QueryDisplayConfig + EnumDisplaySettings).

use super::utf16_z;
use crate::checkup::{Connection, Monitor};
use windows::Win32::Devices::Display::{
    DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
    DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_SOURCE_DEVICE_NAME,
    DISPLAYCONFIG_TARGET_DEVICE_NAME, DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes,
    QDC_ONLY_ACTIVE_PATHS, QueryDisplayConfig,
};
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::Graphics::Gdi::{
    DEVMODEW, ENUM_CURRENT_SETTINGS, ENUM_DISPLAY_SETTINGS_MODE, EnumDisplaySettingsW,
};
use windows::core::PCWSTR;

fn paths() -> Option<Vec<DISPLAYCONFIG_PATH_INFO>> {
    for _ in 0..3 {
        let (mut np, mut nm) = (0u32, 0u32);
        // SAFETY: valid out pointers.
        if unsafe { GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut np, &mut nm) }
            != ERROR_SUCCESS
        {
            return None;
        }
        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); np as usize];
        let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); nm as usize];
        // SAFETY: arrays sized from GetDisplayConfigBufferSizes; counts passed in/out.
        let r = unsafe {
            QueryDisplayConfig(
                QDC_ONLY_ACTIVE_PATHS,
                &mut np,
                paths.as_mut_ptr(),
                &mut nm,
                modes.as_mut_ptr(),
                None,
            )
        };
        if r == ERROR_SUCCESS {
            paths.truncate(np as usize);
            return Some(paths);
        }
        // The configuration changed between the two calls (ERROR_INSUFFICIENT_BUFFER); retry.
    }
    None
}

fn devmode(name: &[u16], mode: ENUM_DISPLAY_SETTINGS_MODE) -> Option<DEVMODEW> {
    let mut dm = DEVMODEW {
        dmSize: size_of::<DEVMODEW>() as u16,
        ..Default::default()
    };
    // SAFETY: NUL-terminated device name; dmSize set.
    unsafe { EnumDisplaySettingsW(PCWSTR(name.as_ptr()), mode, &mut dm) }
        .as_bool()
        .then_some(dm)
}

pub fn monitors() -> Vec<Monitor> {
    let Some(paths) = paths() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for p in paths {
        let mut src = DISPLAYCONFIG_SOURCE_DEVICE_NAME::default();
        src.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
        src.header.size = size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32;
        src.header.adapterId = p.sourceInfo.adapterId;
        src.header.id = p.sourceInfo.id;
        // SAFETY: header describes the full struct.
        if unsafe { DisplayConfigGetDeviceInfo(&mut src.header) } != 0 {
            continue;
        }
        let mut tgt = DISPLAYCONFIG_TARGET_DEVICE_NAME::default();
        tgt.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME;
        tgt.header.size = size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32;
        tgt.header.adapterId = p.targetInfo.adapterId;
        tgt.header.id = p.targetInfo.id;
        // SAFETY: as above.
        let named = unsafe { DisplayConfigGetDeviceInfo(&mut tgt.header) } == 0;

        let gdi = src.viewGdiDeviceName;
        let Some(cur) = devmode(&gdi, ENUM_CURRENT_SETTINGS) else {
            continue;
        };
        let (w, h) = (cur.dmPelsWidth, cur.dmPelsHeight);
        let mut max_hz = cur.dmDisplayFrequency;
        for i in 0..4096 {
            let Some(m) = devmode(&gdi, ENUM_DISPLAY_SETTINGS_MODE(i)) else {
                break;
            };
            if m.dmPelsWidth == w && m.dmPelsHeight == h {
                max_hz = max_hz.max(m.dmDisplayFrequency);
            }
        }
        let rr = p.targetInfo.refreshRate;
        let current_hz = if rr.Denominator > 0 {
            rr.Numerator as f64 / rr.Denominator as f64
        } else {
            cur.dmDisplayFrequency as f64
        };
        out.push(Monitor {
            name: if named {
                utf16_z(&tgt.monitorFriendlyDeviceName)
            } else {
                String::new()
            },
            source: utf16_z(&gdi),
            connection: Connection::from_output_technology(p.targetInfo.outputTechnology.0),
            width: w,
            height: h,
            current_hz,
            max_hz,
        });
    }
    out
}
