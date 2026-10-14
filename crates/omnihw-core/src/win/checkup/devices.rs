//! Device Manager state through CfgMgr32: display adapters, devices with problem codes, and the
//! PCIe link of each NVMe drive's controller.

use crate::checkup::{Device, PcieLink, problem_is_benign};
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    CM_DEVNODE_STATUS_FLAGS, CM_GETIDLIST_FILTER_PRESENT, CM_Get_DevNode_PropertyW,
    CM_Get_DevNode_Status, CM_Get_Device_ID_List_SizeW, CM_Get_Device_ID_ListW, CM_Get_Parent,
    CM_LOCATE_DEVNODE_NORMAL, CM_Locate_DevNodeW, CM_PROB, CR_SUCCESS, DN_HAS_PROBLEM,
};
use windows::Win32::Devices::Properties::{
    DEVPKEY_Device_Class, DEVPKEY_Device_DeviceDesc, DEVPKEY_Device_FriendlyName,
    DEVPKEY_Device_HardwareIds, DEVPKEY_Device_Service, DEVPROPTYPE,
};
use windows::Win32::Foundation::DEVPROPKEY;
use windows::Win32::NetworkManagement::WiFi::{
    DEVPKEY_PciDevice_CurrentLinkSpeed, DEVPKEY_PciDevice_CurrentLinkWidth,
    DEVPKEY_PciDevice_MaxLinkSpeed, DEVPKEY_PciDevice_MaxLinkWidth,
};
use windows::core::PCWSTR;

fn prop(inst: u32, key: &DEVPROPKEY) -> Option<Vec<u8>> {
    let mut ty = DEVPROPTYPE::default();
    let mut size = 0u32;
    // SAFETY: size query without a buffer (fails with CR_BUFFER_SMALL).
    let _ = unsafe { CM_Get_DevNode_PropertyW(inst, key, &mut ty, None, &mut size, 0) };
    if size == 0 {
        return None;
    }
    let mut buf = vec![0u8; size as usize];
    // SAFETY: buffer of `size` bytes.
    let r = unsafe {
        CM_Get_DevNode_PropertyW(inst, key, &mut ty, Some(buf.as_mut_ptr()), &mut size, 0)
    };
    (r == CR_SUCCESS).then(|| {
        buf.truncate(size as usize);
        buf
    })
}

/// String property (the first entry of a string list).
fn prop_string(inst: u32, key: &DEVPROPKEY) -> String {
    prop(inst, key)
        .map(|b| {
            let w: Vec<u16> = b
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            super::utf16_z(&w)
        })
        .unwrap_or_default()
}

fn prop_u32(inst: u32, key: &DEVPROPKEY) -> Option<u32> {
    let b = prop(inst, key)?;
    Some(u32::from_le_bytes(b.get(..4)?.try_into().ok()?))
}

/// Instance ids of all present devices.
fn present_ids() -> Vec<Vec<u16>> {
    let mut len = 0u32;
    // SAFETY: valid out pointer; no filter string.
    if unsafe { CM_Get_Device_ID_List_SizeW(&mut len, PCWSTR::null(), CM_GETIDLIST_FILTER_PRESENT) }
        != CR_SUCCESS
    {
        return Vec::new();
    }
    let mut buf = vec![0u16; len as usize];
    // SAFETY: buffer length passed with the slice.
    if unsafe { CM_Get_Device_ID_ListW(PCWSTR::null(), &mut buf, CM_GETIDLIST_FILTER_PRESENT) }
        != CR_SUCCESS
    {
        return Vec::new();
    }
    buf.split(|&c| c == 0)
        .filter(|s| !s.is_empty())
        .map(|s| s.iter().copied().chain([0]).collect())
        .collect()
}

fn link(inst: u32) -> Option<((u32, u32), (u32, u32))> {
    Some((
        (
            prop_u32(inst, &DEVPKEY_PciDevice_CurrentLinkSpeed)?,
            prop_u32(inst, &DEVPKEY_PciDevice_CurrentLinkWidth)?,
        ),
        (
            prop_u32(inst, &DEVPKEY_PciDevice_MaxLinkSpeed)?,
            prop_u32(inst, &DEVPKEY_PciDevice_MaxLinkWidth)?,
        ),
    ))
}

/// Display adapters and problem devices, plus NVMe PCIe links.
pub fn devices() -> (Vec<Device>, Vec<PcieLink>) {
    let mut out = Vec::new();
    let mut links = Vec::new();
    for id in present_ids() {
        let mut inst = 0u32;
        // SAFETY: NUL-terminated instance id.
        if unsafe { CM_Locate_DevNodeW(&mut inst, PCWSTR(id.as_ptr()), CM_LOCATE_DEVNODE_NORMAL) }
            != CR_SUCCESS
        {
            continue;
        }
        let (mut status, mut problem) = (CM_DEVNODE_STATUS_FLAGS(0), CM_PROB(0));
        // SAFETY: valid out pointers.
        let problem = (unsafe { CM_Get_DevNode_Status(&mut status, &mut problem, inst, 0) }
            == CR_SUCCESS
            && status.0 & DN_HAS_PROBLEM.0 != 0)
            .then_some(problem.0)
            .filter(|&p| p != 0);
        let class = prop_string(inst, &DEVPKEY_Device_Class);
        let is_display = class.eq_ignore_ascii_case("Display");
        let is_disk = class.eq_ignore_ascii_case("DiskDrive");
        let name = || {
            let n = prop_string(inst, &DEVPKEY_Device_FriendlyName);
            if n.is_empty() {
                prop_string(inst, &DEVPKEY_Device_DeviceDesc)
            } else {
                n
            }
        };
        if is_disk {
            let mut parent = 0u32;
            // SAFETY: valid out pointer.
            if unsafe { CM_Get_Parent(&mut parent, inst, 0) } == CR_SUCCESS
                && prop_string(parent, &DEVPKEY_Device_HardwareIds)
                    .to_ascii_uppercase()
                    .starts_with("PCI\\")
                && let Some((current, max)) = link(parent)
            {
                links.push(PcieLink {
                    device: name(),
                    current,
                    max,
                });
            }
        }
        if is_display || problem.is_some_and(|p| !problem_is_benign(p)) {
            out.push(Device {
                name: name(),
                class,
                service: prop_string(inst, &DEVPKEY_Device_Service),
                hardware_id: prop_string(inst, &DEVPKEY_Device_HardwareIds),
                problem,
            });
        }
    }
    (out, links)
}
