//! Minimal WMI access using the `windows` crate directly: `root\wmi` `MSPower_DeviceEnable`
//! (the "Allow the computer to turn off this device to save power" checkbox in Device Manager),
//! plus a small query / update / method-call wrapper ([`Wmi`]) for the network adapter classes.
//! Only these few operations are needed, so no full WMI crate is pulled in.

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
use windows::core::{BSTR, HSTRING, PCWSTR, w};

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

// ---- General queries, updates and method calls (network adapters) --------------------------

/// A WMI namespace connection, with COM initialized on this thread for its lifetime.
pub(crate) struct Wmi {
    // Declared before `_com` so it is released before COM is uninitialized.
    svc: IWbemServices,
    _com: Com,
}

/// A value to write into a WMI property or method parameter.
pub(crate) enum Value<'a> {
    Str(&'a str),
    Strs(&'a [String]),
    /// CIM uint32/sint32 travel as VT_I4.
    I32(i32),
}

impl Variant {
    fn from_value(v: &Value) -> windows::core::Result<Variant> {
        use windows::Win32::System::Variant::{InitVariantFromStringArray, VT_BSTR, VT_I4};
        let mut out = VARIANT::default();
        match v {
            Value::Str(s) => {
                // SAFETY: writes the type tag and the matching union member of a fresh VARIANT;
                // the BSTR is owned by the VARIANT and freed by VariantClear on drop.
                unsafe {
                    (*out.Anonymous.Anonymous).vt = VT_BSTR;
                    (*out.Anonymous.Anonymous).Anonymous.bstrVal =
                        std::mem::ManuallyDrop::new(BSTR::from(*s));
                }
            }
            Value::I32(n) => {
                // SAFETY: as above, for a plain integer member.
                unsafe {
                    (*out.Anonymous.Anonymous).vt = VT_I4;
                    (*out.Anonymous.Anonymous).Anonymous.lVal = *n;
                }
            }
            Value::Strs(list) => {
                let wide: Vec<HSTRING> = list.iter().map(|s| HSTRING::from(s.as_str())).collect();
                let ptrs: Vec<PCWSTR> = wide.iter().map(|h| PCWSTR(h.as_ptr())).collect();
                // SAFETY: the pointers stay valid (held by `wide`) for the call, which copies
                // the strings into a new SAFEARRAY owned by the returned VARIANT.
                out = unsafe { InitVariantFromStringArray(&ptrs) }?;
            }
        }
        Ok(Variant(out))
    }
}

impl Wmi {
    pub(crate) fn connect(namespace: &str) -> windows::core::Result<Wmi> {
        let com = Com::init();
        Ok(Wmi {
            svc: connect(namespace)?,
            _com: com,
        })
    }

    /// Every object a WQL query returns.
    pub(crate) fn query(&self, wql: &str) -> windows::core::Result<Vec<IWbemClassObject>> {
        // SAFETY: COM call on a valid interface; the BSTRs outlive the call and the returned
        // enumerator is released on drop.
        let e = unsafe {
            self.svc.ExecQuery(
                &BSTR::from("WQL"),
                &BSTR::from(wql),
                WBEM_FLAG_FORWARD_ONLY | WBEM_FLAG_RETURN_IMMEDIATELY,
                None,
            )
        }?;
        let mut out = Vec::new();
        loop {
            let mut objs = [None];
            let mut n = 0;
            // SAFETY: valid enumerator; one-element output array and count.
            let hr = unsafe { e.Next(WBEM_INFINITE, &mut objs, &mut n) };
            if hr.is_err() || n == 0 {
                break;
            }
            match objs[0].take() {
                Some(o) => out.push(o),
                None => break,
            }
        }
        Ok(out)
    }

    /// Set properties on an instance and write it back.
    pub(crate) fn put(
        &self,
        obj: &IWbemClassObject,
        props: &[(&str, Value)],
    ) -> windows::core::Result<()> {
        for (name, v) in props {
            let v = Variant::from_value(v)?;
            let name = HSTRING::from(*name);
            // SAFETY: valid object, NUL-terminated name and initialized VARIANT (copied by Put).
            unsafe { obj.Put(&name, 0, &v.0, 0) }?;
        }
        // SAFETY: valid services interface and instance.
        unsafe {
            self.svc.PutInstance(
                obj,
                WBEM_GENERIC_FLAG_TYPE(WBEM_FLAG_UPDATE_ONLY.0),
                None,
                None,
            )
        }
    }

    /// Call `method` of class `class` on instance `obj` with `args`; returns its ReturnValue.
    pub(crate) fn call(
        &self,
        obj: &IWbemClassObject,
        class: &str,
        method: &str,
        args: &[(&str, Value)],
    ) -> windows::core::Result<u32> {
        let path =
            get_string(obj, w!("__PATH")).ok_or_else(|| windows::core::Error::from(E_FAIL))?;
        let method_w = HSTRING::from(method);
        let mut class_obj = None;
        // SAFETY: valid services interface; the out pointer receives an owned interface.
        unsafe {
            self.svc.GetObject(
                &BSTR::from(class),
                WBEM_GENERIC_FLAG_TYPE(0),
                None,
                Some(&mut class_obj),
                None,
            )
        }?;
        let class_obj = class_obj.ok_or_else(|| windows::core::Error::from(E_FAIL))?;
        let mut in_sig: Option<IWbemClassObject> = None;
        let mut out_sig: Option<IWbemClassObject> = None;
        // SAFETY: valid class object and NUL-terminated method name; outputs are owned.
        unsafe { class_obj.GetMethod(&method_w, 0, &mut in_sig, &mut out_sig) }?;
        let input = match in_sig {
            Some(sig) if !args.is_empty() => {
                // SAFETY: valid signature object.
                let inst = unsafe { sig.SpawnInstance(0) }?;
                for (name, v) in args {
                    let v = Variant::from_value(v)?;
                    let name = HSTRING::from(*name);
                    // SAFETY: valid object, NUL-terminated name and initialized VARIANT.
                    unsafe { inst.Put(&name, 0, &v.0, 0) }?;
                }
                Some(inst)
            }
            _ => None,
        };
        let mut out = None;
        // SAFETY: valid services interface; the BSTRs and input object outlive the call.
        unsafe {
            self.svc.ExecMethod(
                &BSTR::from(path),
                &BSTR::from(method),
                WBEM_GENERIC_FLAG_TYPE(0),
                None,
                input.as_ref(),
                Some(&mut out),
                None,
            )
        }?;
        Ok(out
            .as_ref()
            .and_then(|o| get_u32(o, w!("ReturnValue")))
            .unwrap_or(0))
    }
}

/// A string property (numbers and booleans are converted to text); `None` when empty.
pub(crate) fn string(obj: &IWbemClassObject, name: &str) -> Option<String> {
    let name = HSTRING::from(name);
    get_string(obj, PCWSTR(name.as_ptr())).filter(|s| !s.is_empty())
}

pub(crate) fn boolean(obj: &IWbemClassObject, name: &str) -> Option<bool> {
    let name = HSTRING::from(name);
    get_bool(obj, PCWSTR(name.as_ptr()))
}

pub(crate) fn uint(obj: &IWbemClassObject, name: &str) -> Option<u32> {
    let name = HSTRING::from(name);
    get_u32(obj, PCWSTR(name.as_ptr()))
}

/// A uint64 property (WMI passes these as strings).
pub(crate) fn uint64(obj: &IWbemClassObject, name: &str) -> Option<u64> {
    string(obj, name)?.trim().parse().ok()
}

/// A string-array property (empty when missing or null).
pub(crate) fn strings(obj: &IWbemClassObject, name: &str) -> Vec<String> {
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::System::Variant::{VariantGetElementCount, VariantGetStringElem};
    let name = HSTRING::from(name);
    let Some(v) = Variant::get(obj, PCWSTR(name.as_ptr())) else {
        return Vec::new();
    };
    // SAFETY: initialized VARIANT; 0 for empty/null values.
    let n = unsafe { VariantGetElementCount(&v.0) };
    (0..n)
        .filter_map(|i| {
            // SAFETY: `i` is below the element count; the returned string is CoTaskMem
            // allocated and freed right after copying.
            unsafe {
                let p = VariantGetStringElem(&v.0, i).ok()?;
                let s = p.to_string().ok();
                CoTaskMemFree(Some(p.0 as *const _));
                s
            }
        })
        .collect()
}

fn get_u32(obj: &IWbemClassObject, name: PCWSTR) -> Option<u32> {
    use windows::Win32::System::Variant::{VT_EMPTY, VT_NULL, VariantToUInt32};
    let v = Variant::get(obj, name)?;
    // SAFETY: reads the type tag of an initialized VARIANT.
    let vt = unsafe { v.0.Anonymous.Anonymous.vt };
    if vt == VT_NULL || vt == VT_EMPTY {
        return None;
    }
    // SAFETY: initialized VARIANT.
    unsafe { VariantToUInt32(&v.0) }.ok()
}
