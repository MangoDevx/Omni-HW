//! DXGI adapter enumeration through hand-written COM vtable calls.

use super::ffi::*;
use crate::gpu::{Display, GpuAdapter};
use std::ffi::c_void;
use std::ptr;

const IID_IDXGI_FACTORY1: GUID = GUID {
    data1: 0x770a_ae78,
    data2: 0xf26f,
    data3: 0x4dba,
    data4: [0xa8, 0x29, 0x25, 0x3c, 0x83, 0xd1, 0xb3, 0x87],
};
/// Passing IDXGIDevice to CheckInterfaceSupport returns the UMD driver version.
const IID_IDXGI_DEVICE: GUID = GUID {
    data1: 0x54ec_77fa,
    data2: 0x1377,
    data3: 0x44e6,
    data4: [0x8c, 0x32, 0x88, 0xfd, 0x5f, 0x44, 0xc8, 0x4c],
};
const DXGI_ERROR_NOT_FOUND: i32 = 0x887A_0002_u32 as i32;
const DXGI_ADAPTER_FLAG_SOFTWARE: u32 = 2;

// Vtable slots (IUnknown 0-2, IDXGIObject 3-6).
const SLOT_RELEASE: usize = 2;
const SLOT_FACTORY1_ENUM_ADAPTERS1: usize = 12;
const SLOT_ADAPTER_ENUM_OUTPUTS: usize = 7;
const SLOT_ADAPTER_CHECK_INTERFACE_SUPPORT: usize = 9;
const SLOT_OUTPUT_GET_DESC: usize = 7;
const SLOT_ADAPTER1_GET_DESC1: usize = 10;

#[repr(C)]
struct DxgiAdapterDesc1 {
    description: [u16; 128],
    vendor_id: u32,
    device_id: u32,
    sub_sys_id: u32,
    revision: u32,
    dedicated_video_memory: usize,
    dedicated_system_memory: usize,
    shared_system_memory: usize,
    luid_low: u32,
    luid_high: i32,
    flags: u32,
}

/// Owned COM pointer, released on drop.
struct Com(*mut c_void);

impl Com {
    /// # Safety
    /// `slot` must be a valid vtable index whose function has signature `F`.
    unsafe fn method<F: Copy>(&self, slot: usize) -> F {
        // SAFETY: COM objects start with a pointer to their vtable.
        unsafe {
            let vtbl = *(self.0 as *const *const *const c_void);
            std::mem::transmute_copy(&*vtbl.add(slot))
        }
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        // SAFETY: IUnknown::Release on a pointer we own.
        unsafe {
            let release: unsafe extern "system" fn(*mut c_void) -> u32 = self.method(SLOT_RELEASE);
            release(self.0);
        }
    }
}

fn version_string(v: i64) -> String {
    let v = v as u64;
    format!(
        "{}.{}.{}.{}",
        v >> 48,
        (v >> 32) & 0xFFFF,
        (v >> 16) & 0xFFFF,
        v & 0xFFFF
    )
}

/// Kernel adapter type flags (`D3DKMT_ADAPTERTYPE`) for a LUID.
fn kmt_adapter_type(luid_low: u32, luid_high: i32) -> Option<u32> {
    let mut open = D3DKMT_OPENADAPTERFROMLUID {
        luid_low,
        luid_high,
        adapter: 0,
    };
    // SAFETY: valid in/out struct.
    if unsafe { D3DKMTOpenAdapterFromLuid(&mut open) } != 0 {
        return None;
    }
    let mut flags: u32 = 0;
    let mut q = D3DKMT_QUERYADAPTERINFO {
        adapter: open.adapter,
        kind: KMTQAITYPE_ADAPTERTYPE,
        data: (&mut flags as *mut u32).cast(),
        data_size: 4,
    };
    // SAFETY: data points to a u32 of the declared size; adapter closed once.
    let st = unsafe { D3DKMTQueryAdapterInfo(&mut q) };
    unsafe { D3DKMTCloseAdapter(&mut open.adapter) };
    (st == 0).then_some(flags)
}

#[repr(C)]
struct DxgiOutputDesc {
    device_name: [u16; 32],
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
    attached_to_desktop: i32,
    rotation: u32,
    monitor: *mut c_void,
}

/// Desktop-attached outputs (connected displays) driven by this adapter.
fn outputs(adapter: &Com) -> Vec<Display> {
    type EnumOutputs = unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> i32;
    type GetDesc = unsafe extern "system" fn(*mut c_void, *mut DxgiOutputDesc) -> i32;
    let mut out = Vec::new();
    // SAFETY: slot 7 of IDXGIAdapter is EnumOutputs.
    let enum_outputs: EnumOutputs = unsafe { adapter.method(SLOT_ADAPTER_ENUM_OUTPUTS) };
    for i in 0..16 {
        let mut o = ptr::null_mut();
        // SAFETY: valid adapter and out pointer.
        if unsafe { enum_outputs(adapter.0, i, &mut o) } < 0 || o.is_null() {
            break;
        }
        let output = Com(o);
        // SAFETY: zeroed POD out-struct; slot 7 of IDXGIOutput is GetDesc.
        let mut d: DxgiOutputDesc = unsafe { std::mem::zeroed() };
        let get_desc: GetDesc = unsafe { output.method(SLOT_OUTPUT_GET_DESC) };
        if unsafe { get_desc(output.0, &mut d) } < 0 || d.attached_to_desktop == 0 {
            continue;
        }
        let len = d.device_name.iter().position(|&c| c == 0).unwrap_or(32);
        out.push(Display {
            name: String::from_utf16_lossy(&d.device_name[..len]),
            width: (d.right - d.left).unsigned_abs(),
            height: (d.bottom - d.top).unsigned_abs(),
        });
    }
    out
}

/// Hardware adapters, in DXGI order. Software rasterizers and indirect-display
/// (virtual monitor) adapters, which mirror their render GPU's description, are skipped.
pub fn adapters() -> Vec<GpuAdapter> {
    let mut out = Vec::new();
    let mut f = ptr::null_mut();
    // SAFETY: valid IID and out pointer.
    if unsafe { CreateDXGIFactory1(&IID_IDXGI_FACTORY1, &mut f) } < 0 || f.is_null() {
        return out;
    }
    let factory = Com(f);
    type EnumAdapters1 = unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> i32;
    type GetDesc1 = unsafe extern "system" fn(*mut c_void, *mut DxgiAdapterDesc1) -> i32;
    type CheckSupport = unsafe extern "system" fn(*mut c_void, *const GUID, *mut i64) -> i32;
    // SAFETY: slot 12 of IDXGIFactory1 is EnumAdapters1.
    let enum_adapters: EnumAdapters1 = unsafe { factory.method(SLOT_FACTORY1_ENUM_ADAPTERS1) };
    for i in 0..16 {
        let mut a = ptr::null_mut();
        // SAFETY: valid factory and out pointer.
        let hr = unsafe { enum_adapters(factory.0, i, &mut a) };
        if hr == DXGI_ERROR_NOT_FOUND || hr < 0 || a.is_null() {
            break;
        }
        let adapter = Com(a);
        // SAFETY: zeroed POD out-struct; slot 10 of IDXGIAdapter1 is GetDesc1.
        let mut d: DxgiAdapterDesc1 = unsafe { std::mem::zeroed() };
        let get_desc: GetDesc1 = unsafe { adapter.method(SLOT_ADAPTER1_GET_DESC1) };
        if unsafe { get_desc(adapter.0, &mut d) } < 0 || d.flags & DXGI_ADAPTER_FLAG_SOFTWARE != 0 {
            continue;
        }
        if let Some(t) = kmt_adapter_type(d.luid_low, d.luid_high)
            && (t & (ADAPTERTYPE_INDIRECT_DISPLAY | ADAPTERTYPE_SOFTWARE_DEVICE) != 0
                || t & ADAPTERTYPE_RENDER_SUPPORTED == 0)
        {
            continue;
        }
        let mut umd: i64 = 0;
        // SAFETY: slot 9 of IDXGIAdapter is CheckInterfaceSupport.
        let check: CheckSupport = unsafe { adapter.method(SLOT_ADAPTER_CHECK_INTERFACE_SUPPORT) };
        let driver_version = if unsafe { check(adapter.0, &IID_IDXGI_DEVICE, &mut umd) } >= 0 {
            version_string(umd)
        } else {
            String::new()
        };
        let len = d.description.iter().position(|&c| c == 0).unwrap_or(128);
        let displays = outputs(&adapter);
        out.push(GpuAdapter {
            displays,
            name: String::from_utf16_lossy(&d.description[..len])
                .trim()
                .to_string(),
            vendor_id: d.vendor_id,
            device_id: d.device_id,
            subsys_id: d.sub_sys_id,
            revision: d.revision,
            dedicated_vram: d.dedicated_video_memory as u64,
            shared_memory: d.shared_system_memory as u64,
            luid: ((d.luid_high as u32 as u64) << 32) | d.luid_low as u64,
            driver_version,
            nvml: None,
            driver_date: None,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn version() {
        let v: i64 = (32 << 48) | (16 << 16) | 1742;
        assert_eq!(super::version_string(v), "32.0.16.1742");
    }
}
