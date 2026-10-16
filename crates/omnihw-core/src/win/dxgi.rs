//! DXGI adapter enumeration through the `windows` crate's COM bindings.

use super::wide::from_wide;
use crate::gpu::{Display, GpuAdapter};
use windows::Wdk::Graphics::Direct3D::{
    D3DKMT_ADAPTERTYPE, D3DKMT_CLOSEADAPTER, D3DKMT_OPENADAPTERFROMLUID, D3DKMT_QUERYADAPTERINFO,
    D3DKMTCloseAdapter, D3DKMTOpenAdapterFromLuid, D3DKMTQueryAdapterInfo, KMTQAITYPE_ADAPTERTYPE,
};
use windows::Win32::Foundation::{LUID, STATUS_SUCCESS};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE, IDXGIAdapter1, IDXGIDevice, IDXGIFactory1,
};
use windows::core::Interface;

/// `D3DKMT_ADAPTERTYPE` bits; the crate exposes the bitfield only as a raw `u32`.
const ADAPTERTYPE_RENDER_SUPPORTED: u32 = 1 << 0;
const ADAPTERTYPE_SOFTWARE_DEVICE: u32 = 1 << 2;
const ADAPTERTYPE_INDIRECT_DISPLAY: u32 = 1 << 6;

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
fn kmt_adapter_type(luid: LUID) -> Option<u32> {
    let mut open = D3DKMT_OPENADAPTERFROMLUID {
        AdapterLuid: luid,
        hAdapter: 0,
    };
    // SAFETY: valid in/out struct (a local).
    if unsafe { D3DKMTOpenAdapterFromLuid(&mut open) } != STATUS_SUCCESS {
        return None;
    }
    let mut flags = D3DKMT_ADAPTERTYPE::default();
    let mut q = D3DKMT_QUERYADAPTERINFO {
        hAdapter: open.hAdapter,
        Type: KMTQAITYPE_ADAPTERTYPE,
        pPrivateDriverData: (&mut flags as *mut D3DKMT_ADAPTERTYPE).cast(),
        PrivateDriverDataSize: size_of::<D3DKMT_ADAPTERTYPE>() as u32,
    };
    // SAFETY: the private data points to a D3DKMT_ADAPTERTYPE of the declared size, which
    // outlives the call; the adapter was opened above.
    let st = unsafe { D3DKMTQueryAdapterInfo(&mut q) };
    // SAFETY: closes the adapter opened above, exactly once (no early return in between).
    let _ = unsafe {
        D3DKMTCloseAdapter(&D3DKMT_CLOSEADAPTER {
            hAdapter: open.hAdapter,
        })
    };
    // SAFETY: every bit pattern is a valid u32.
    (st == STATUS_SUCCESS).then_some(unsafe { flags.Anonymous.Value })
}

/// Desktop-attached outputs (connected displays) driven by this adapter.
fn outputs(adapter: &IDXGIAdapter1) -> Vec<Display> {
    let mut out = Vec::new();
    for i in 0..16 {
        // SAFETY: valid adapter interface; errors (DXGI_ERROR_NOT_FOUND) end the list.
        let Ok(output) = (unsafe { adapter.EnumOutputs(i) }) else {
            break;
        };
        // SAFETY: valid output interface.
        let Ok(d) = (unsafe { output.GetDesc() }) else {
            continue;
        };
        if !d.AttachedToDesktop.as_bool() {
            continue;
        }
        let r = d.DesktopCoordinates;
        out.push(Display {
            name: from_wide(&d.DeviceName),
            width: (r.right - r.left).unsigned_abs(),
            height: (r.bottom - r.top).unsigned_abs(),
        });
    }
    out
}

/// Hardware adapters, in DXGI order. Software rasterizers and indirect-display
/// (virtual monitor) adapters, which mirror their render GPU's description, are skipped.
pub fn adapters() -> Vec<GpuAdapter> {
    let mut out = Vec::new();
    // SAFETY: plain factory creation; no pointers involved.
    let Ok(factory) = (unsafe { CreateDXGIFactory1::<IDXGIFactory1>() }) else {
        return out;
    };
    for i in 0..16 {
        // SAFETY: valid factory; DXGI_ERROR_NOT_FOUND (or any error) ends the list.
        let Ok(adapter) = (unsafe { factory.EnumAdapters1(i) }) else {
            break;
        };
        // SAFETY: valid adapter interface.
        let Ok(d) = (unsafe { adapter.GetDesc1() }) else {
            continue;
        };
        if d.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
            continue;
        }
        if let Some(t) = kmt_adapter_type(d.AdapterLuid)
            && (t & (ADAPTERTYPE_INDIRECT_DISPLAY | ADAPTERTYPE_SOFTWARE_DEVICE) != 0
                || t & ADAPTERTYPE_RENDER_SUPPORTED == 0)
        {
            continue;
        }
        // Passing IDXGIDevice to CheckInterfaceSupport returns the UMD driver version.
        // SAFETY: valid adapter and IID.
        let driver_version = match unsafe { adapter.CheckInterfaceSupport(&IDXGIDevice::IID) } {
            Ok(umd) => version_string(umd),
            Err(_) => String::new(),
        };
        let displays = outputs(&adapter);
        let luid = d.AdapterLuid;
        out.push(GpuAdapter {
            displays,
            name: from_wide(&d.Description).trim().to_string(),
            vendor_id: d.VendorId,
            device_id: d.DeviceId,
            subsys_id: d.SubSysId,
            revision: d.Revision,
            dedicated_vram: d.DedicatedVideoMemory as u64,
            shared_memory: d.SharedSystemMemory as u64,
            luid: ((luid.HighPart as u32 as u64) << 32) | luid.LowPart as u64,
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
