//! Minimal WMI access for `root\wmi` `MSPower_DeviceEnable`: the "Allow the computer to turn
//! off this device to save power" checkbox in Device Manager. Only one query and one update
//! are needed, so this uses the `windows` crate directly instead of a full WMI crate.

use super::wide::from_wide;
use windows::Win32::Foundation::{E_FAIL, VARIANT_FALSE, VARIANT_TRUE};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
    CoSetProxyBlanket, CoUninitialize, EOAC_NONE, RPC_C_AUTHN_LEVEL_CALL,
    RPC_C_IMP_LEVEL_IMPERSONATE,
};
use windows::Win32::System::Variant::{
    VARIANT, VT_BOOL, VariantClear, VariantToBoolean, VariantToString,
};
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
    // SAFETY: standard WMI connection sequence on a COM-initialized thread; the BSTR
    // arguments outlive the calls and the returned interfaces are owned (released on drop).
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

/// An owned VARIANT, cleared with `VariantClear` on drop.
struct Variant(VARIANT);

impl Variant {
    /// Property `name` of a WMI object.
    fn get(obj: &IWbemClassObject, name: PCWSTR) -> Option<Variant> {
        let mut v = Variant(VARIANT::default());
        // SAFETY: valid object and NUL-terminated name; the out VARIANT is owned (and cleared)
        // by `v` whether or not the call succeeds.
        unsafe { obj.Get(name, 0, &mut v.0, None, None) }.ok()?;
        Some(v)
    }

    /// A VT_BOOL VARIANT.
    fn bool(on: bool) -> Variant {
        let mut v = VARIANT::default();
        // SAFETY: writes the type tag and the matching union member of a fresh VARIANT; the
        // ManuallyDrop wrappers are plain-data views, so nothing needs dropping.
        unsafe {
            (*v.Anonymous.Anonymous).vt = VT_BOOL;
            (*v.Anonymous.Anonymous).Anonymous.boolVal =
                if on { VARIANT_TRUE } else { VARIANT_FALSE };
        }
        Variant(v)
    }
}

impl Drop for Variant {
    fn drop(&mut self) {
        // SAFETY: the VARIANT is initialized (empty or filled by WMI) and cleared once.
        let _ = unsafe { VariantClear(&mut self.0) };
    }
}

fn get_string(obj: &IWbemClassObject, name: PCWSTR) -> Option<String> {
    let v = Variant::get(obj, name)?;
    let mut buf = [0u16; 1024];
    // SAFETY: initialized VARIANT; the output length is passed with the slice.
    unsafe { VariantToString(&v.0, &mut buf) }.ok()?;
    Some(from_wide(&buf))
}

fn get_bool(obj: &IWbemClassObject, name: PCWSTR) -> Option<bool> {
    let v = Variant::get(obj, name)?;
    // SAFETY: initialized VARIANT.
    unsafe { VariantToBoolean(&v.0) }.ok().map(|b| b.as_bool())
}

/// Instance name ("PCI\VEN_...\..._0") -> enabled, for every device with the checkbox.
pub fn device_power_enable() -> Vec<(String, bool)> {
    let _com = Com::init();
    let Ok(svc) = connect(r"root\wmi") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    // SAFETY: COM call on a valid interface; the BSTRs outlive the call and the returned
    // enumerator is released on drop.
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
        // SAFETY: valid enumerator; one-element output array and count.
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
    // SAFETY: COM call on a valid interface; the BSTRs outlive the call.
    let e = unsafe {
        svc.ExecQuery(
            &BSTR::from("WQL"),
            &BSTR::from(query),
            WBEM_FLAG_FORWARD_ONLY | WBEM_FLAG_RETURN_IMMEDIATELY,
            None,
        )
    }?;
    let mut objs = [None];
    let mut n = 0;
    // SAFETY: valid enumerator; one-element output array and count.
    unsafe { e.Next(WBEM_INFINITE, &mut objs, &mut n) }.ok()?;
    let obj = objs[0]
        .take()
        .ok_or_else(|| windows::core::Error::from(E_FAIL))?;
    let v = Variant::bool(enable);
    // SAFETY: valid object, NUL-terminated property name and initialized VARIANT (copied by
    // Put); the services interface outlives the call.
    unsafe {
        obj.Put(w!("Enable"), 0, &v.0, 0)?;
        svc.PutInstance(
            &obj,
            WBEM_GENERIC_FLAG_TYPE(WBEM_FLAG_UPDATE_ONLY.0),
            None,
            None,
        )
    }
}
