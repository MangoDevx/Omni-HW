//! Minimal WMI access for `root\wmi` `MSPower_DeviceEnable`: the "Allow the computer to turn
//! off this device to save power" checkbox in Device Manager. Only one query and one update
//! are needed, so this uses the `windows` crate directly instead of a full WMI crate.

use windows::Win32::Foundation::{VARIANT_FALSE, VARIANT_TRUE};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
    CoSetProxyBlanket, CoUninitialize, EOAC_NONE, RPC_C_AUTHN_LEVEL_CALL,
    RPC_C_IMP_LEVEL_IMPERSONATE,
};
use windows::Win32::System::Variant::{VARIANT, VT_BOOL, VariantClear, VariantToBoolean};
use windows::Win32::System::Wmi::{
    IWbemClassObject, IWbemLocator, IWbemServices, WBEM_FLAG_FORWARD_ONLY,
    WBEM_FLAG_RETURN_IMMEDIATELY, WBEM_FLAG_UPDATE_ONLY, WBEM_GENERIC_FLAG_TYPE, WBEM_INFINITE,
    WbemLocator,
};
use windows::core::{BSTR, PCWSTR, w};

/// RPC_C_AUTHN_WINNT / RPC_C_AUTHZ_NONE (avoids pulling in the Rpc feature for two numbers).
const AUTHN_WINNT: u32 = 10;
const AUTHZ_NONE: u32 = 0;

/// Initializes COM on this thread for its lifetime.
struct Com(bool);

impl Com {
    fn init() -> Com {
        // SAFETY: plain COM initialization; balanced in Drop when it succeeded.
        let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        Com(hr.is_ok())
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        if self.0 {
            // SAFETY: matches the successful CoInitializeEx above.
            unsafe { CoUninitialize() };
        }
    }
}

fn connect(namespace: &str) -> windows::core::Result<IWbemServices> {
    // SAFETY: standard WMI connection sequence; all pointers are owned COM objects.
    unsafe {
        let locator: IWbemLocator = CoCreateInstance(&WbemLocator, None, CLSCTX_INPROC_SERVER)?;
        let services = locator.ConnectServer(
            &BSTR::from(namespace),
            &BSTR::new(),
            &BSTR::new(),
            &BSTR::new(),
            0,
            &BSTR::new(),
            None,
        )?;
        CoSetProxyBlanket(
            &services,
            AUTHN_WINNT,
            AUTHZ_NONE,
            PCWSTR::null(),
            RPC_C_AUTHN_LEVEL_CALL,
            RPC_C_IMP_LEVEL_IMPERSONATE,
            None,
            EOAC_NONE,
        )?;
        Ok(services)
    }
}

fn get_string(obj: &IWbemClassObject, name: PCWSTR) -> Option<String> {
    let mut v = VARIANT::default();
    // SAFETY: valid out VARIANT, cleared below.
    unsafe {
        obj.Get(name, 0, &mut v, None, None).ok()?;
        let mut buf = [0u16; 1024];
        let r = windows::Win32::System::Variant::VariantToString(&v, &mut buf);
        let _ = VariantClear(&mut v);
        r.ok()?;
        let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..end]))
    }
}

fn get_bool(obj: &IWbemClassObject, name: PCWSTR) -> Option<bool> {
    let mut v = VARIANT::default();
    // SAFETY: valid out VARIANT, cleared below.
    unsafe {
        obj.Get(name, 0, &mut v, None, None).ok()?;
        let r = VariantToBoolean(&v);
        let _ = VariantClear(&mut v);
        r.ok().map(|b| b.as_bool())
    }
}

/// Instance name ("PCI\VEN_...\..._0") -> enabled, for every device with the checkbox.
pub fn device_power_enable() -> Vec<(String, bool)> {
    let _com = Com::init();
    let Ok(svc) = connect(r"root\wmi") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    // SAFETY: COM calls on valid interfaces; objects are released when dropped.
    let Ok(e) = (unsafe {
        svc.ExecQuery(
            &BSTR::from("WQL"),
            &BSTR::from("SELECT InstanceName, Enable FROM MSPower_DeviceEnable"),
            WBEM_FLAG_FORWARD_ONLY | WBEM_FLAG_RETURN_IMMEDIATELY,
            None,
        )
    }) else {
        return out;
    };
    loop {
        let mut objs = [None];
        let mut n = 0;
        // SAFETY: one-element output array and count.
        let hr = unsafe { e.Next(WBEM_INFINITE, &mut objs, &mut n) };
        if hr.is_err() || n == 0 {
            break;
        }
        let Some(obj) = objs[0].take() else { break };
        if let (Some(name), Some(on)) = (
            get_string(&obj, w!("InstanceName")),
            get_bool(&obj, w!("Enable")),
        ) {
            out.push((name, on));
        }
    }
    out
}

/// Set `Enable` on the `MSPower_DeviceEnable` instance whose name is `instance_name`.
/// Needs administrator rights.
pub fn set_device_power_enable(instance_name: &str, enable: bool) -> windows::core::Result<()> {
    let _com = Com::init();
    let svc = connect(r"root\wmi")?;
    // WQL string literal: escape backslashes and quotes.
    let lit = instance_name.replace('\\', "\\\\").replace('"', "\\\"");
    let query = format!("SELECT * FROM MSPower_DeviceEnable WHERE InstanceName = \"{lit}\"");
    // SAFETY: COM calls on valid interfaces; the VARIANT holds a plain bool.
    unsafe {
        let e = svc.ExecQuery(
            &BSTR::from("WQL"),
            &BSTR::from(query),
            WBEM_FLAG_FORWARD_ONLY | WBEM_FLAG_RETURN_IMMEDIATELY,
            None,
        )?;
        let mut objs = [None];
        let mut n = 0;
        e.Next(WBEM_INFINITE, &mut objs, &mut n).ok()?;
        let obj = objs[0]
            .take()
            .ok_or_else(|| windows::core::Error::from(windows::Win32::Foundation::E_FAIL))?;
        let mut v = VARIANT::default();
        (*v.Anonymous.Anonymous).vt = VT_BOOL;
        (*v.Anonymous.Anonymous).Anonymous.boolVal =
            if enable { VARIANT_TRUE } else { VARIANT_FALSE };
        obj.Put(w!("Enable"), 0, &v, 0)?;
        svc.PutInstance(
            &obj,
            WBEM_GENERIC_FLAG_TYPE(WBEM_FLAG_UPDATE_ONLY.0),
            None,
            None,
        )
    }
}
