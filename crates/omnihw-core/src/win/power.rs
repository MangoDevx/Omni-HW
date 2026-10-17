//! Power settings via powrprof (plans, values, capabilities, wake-armed devices), the System
//! event log (wake history), the network class registry and WMI (adapter power saving),
//! SetupAPI (connected USB devices) and, when elevated, `powercfg` for power requests and wake
//! timers (their kernel interface is undocumented).

#[cfg(not(windows))]
mod imp {
    use crate::power::{Action, ApplyError, PowerInfo};
    pub fn gather_settings(_p: &mut PowerInfo) {}
    pub fn gather_devices(_p: &mut PowerInfo) {}
    pub fn apply(_a: &Action) -> Result<(), ApplyError> {
        Err(ApplyError::Failed("Not supported on this platform".into()))
    }
    pub fn query_admin_raw() -> Option<(String, String)> {
        None
    }
}

#[cfg(windows)]
mod imp {
    use crate::power::{
        self, Action, AdapterKind, AdapterPower, ApplyError, Capabilities, EeeSetting, PowerInfo,
        Scheme, Setting, Source, UsbSensitive,
    };
    use crate::win::PlanKind;
    use crate::win::registry::{self, Hive, NET_CLASS};
    use crate::win::wide::{from_wide, from_wide_bytes, guid_text};
    use std::ptr;
    use windows::Win32::Devices::DeviceAndDriverInstallation::{
        DICS_FLAG_CONFIGSPECIFIC, DICS_PROPCHANGE, DIF_PROPERTYCHANGE, DIGCF_ALLCLASSES,
        DIGCF_PRESENT, HDEVINFO, SETUP_DI_REGISTRY_PROPERTY, SP_CLASSINSTALL_HEADER,
        SP_DEVINFO_DATA, SP_PROPCHANGE_PARAMS, SPDRP_CLASS, SPDRP_DEVICEDESC, SPDRP_FRIENDLYNAME,
        SetupDiCallClassInstaller, SetupDiCreateDeviceInfoList, SetupDiEnumDeviceInfo,
        SetupDiGetClassDevsW, SetupDiGetDeviceInstanceIdW, SetupDiGetDeviceRegistryPropertyW,
        SetupDiOpenDeviceInfoW, SetupDiSetClassInstallParamsW,
    };
    use windows::Win32::Foundation::{ERROR_ACCESS_DENIED, ERROR_SUCCESS, HLOCAL, WIN32_ERROR};
    use windows::Win32::System::Power::{
        ACCESS_SCHEME, DEVICEPOWER_CLEAR_WAKEENABLED, DEVICEPOWER_FILTER_DEVICES_PRESENT,
        DEVICEPOWER_FILTER_WAKEENABLED, DevicePowerClose, DevicePowerEnumDevices, DevicePowerOpen,
        DevicePowerSetDeviceState, GetPwrCapabilities, PowerEnumerate, PowerGetActiveScheme,
        PowerReadACValueIndex, PowerReadDCValueIndex, PowerReadFriendlyName, PowerSetActiveScheme,
        PowerWriteACValueIndex, PowerWriteDCValueIndex, SYSTEM_POWER_CAPABILITIES,
    };
    use windows::core::{GUID, HSTRING, Owned, PCWSTR};

    const POWER_KEY: &str = r"SYSTEM\CurrentControlSet\Control\Session Manager\Power";
    /// NCF_PHYSICAL: a real (not virtual) network adapter.
    const NCF_PHYSICAL: u32 = 0x4;
    /// Wake history kept: 14 days.
    const WAKE_HISTORY_MS: u64 = 14 * 86_400_000;

    fn denied(e: WIN32_ERROR) -> ApplyError {
        if e == ERROR_ACCESS_DENIED {
            ApplyError::AccessDenied
        } else {
            ApplyError::Failed(windows::core::Error::from(e.to_hresult()).message())
        }
    }

    /// Friendly name of a power scheme ("Balanced"); empty if it can't be read.
    pub(crate) fn scheme_name(g: &GUID) -> String {
        let mut name = [0u16; 256];
        let mut len = (name.len() * 2) as u32;
        // SAFETY: valid GUID pointer; the buffer size in bytes is passed in `len`.
        let ok = unsafe {
            PowerReadFriendlyName(
                None,
                Some(g),
                None,
                None,
                Some(name.as_mut_ptr().cast()),
                &mut len,
            )
        } == ERROR_SUCCESS;
        if ok { from_wide(&name) } else { String::new() }
    }

    /// GUID of the active power scheme.
    pub(crate) fn active_scheme() -> Option<GUID> {
        let mut g: *mut GUID = ptr::null_mut();
        // SAFETY: valid out pointer to a local.
        if unsafe { PowerGetActiveScheme(None, &mut g) } != ERROR_SUCCESS || g.is_null() {
            return None;
        }
        // SAFETY: on success the GUID was LocalAlloc'd for us; freed with LocalFree on drop.
        let _alloc = unsafe { Owned::new(HLOCAL(g.cast())) };
        // SAFETY: `g` points to an initialized GUID that lives until `_alloc` drops.
        Some(unsafe { g.read() })
    }

    fn schemes() -> Vec<GUID> {
        let mut out = Vec::new();
        for i in 0..64 {
            let mut g = GUID::zeroed();
            let mut size = size_of::<GUID>() as u32;
            // SAFETY: buffer is one GUID; size in bytes.
            let st = unsafe {
                PowerEnumerate(
                    None,
                    None,
                    None,
                    ACCESS_SCHEME,
                    i,
                    Some((&raw mut g).cast()),
                    &mut size,
                )
            };
            if st != ERROR_SUCCESS {
                break;
            }
            out.push(g);
        }
        out
    }

    fn read(scheme: &GUID, s: Setting) -> power::AcDc {
        let (sub, set) = (GUID::from_u128(s.subgroup()), GUID::from_u128(s.guid()));
        let (mut ac, mut dc) = (0u32, 0u32);
        // SAFETY: GUID pointers and out values are valid locals for the calls.
        let ac_ok =
            unsafe { PowerReadACValueIndex(None, Some(scheme), Some(&sub), Some(&set), &mut ac) }
                == ERROR_SUCCESS;
        // SAFETY: as above.
        let dc_ok =
            unsafe { PowerReadDCValueIndex(None, Some(scheme), Some(&sub), Some(&set), &mut dc) }
                == ERROR_SUCCESS.0;
        power::AcDc {
            ac: ac_ok.then_some(ac),
            dc: dc_ok.then_some(dc),
        }
    }

    fn capabilities() -> Capabilities {
        let mut c = SYSTEM_POWER_CAPABILITIES::default();
        // SAFETY: valid out struct (a local).
        if !unsafe { GetPwrCapabilities(&mut c) } {
            return Capabilities::default();
        }
        Capabilities {
            lid: c.LidPresent,
            sleep_button: c.SleepButtonPresent,
            battery: c.SystemBatteriesPresent && !c.BatteriesAreShortTerm,
            s3: c.SystemS3,
            hibernate: c.SystemS4,
            hiberfile: c.HiberFilePresent,
            modern_standby: c.AoAc,
        }
    }

    pub fn gather_settings(p: &mut PowerInfo) {
        p.schemes = schemes()
            .iter()
            .map(|g| {
                let guid = guid_text(g);
                Scheme {
                    name: scheme_name(g),
                    kind: PlanKind::from_guid(&guid),
                    guid,
                }
            })
            .collect();
        if let Some(a) = active_scheme() {
            p.active = Some(guid_text(&a));
            p.values = Setting::ALL.iter().map(|&s| (s, read(&a, s))).collect();
        }
        p.caps = capabilities();
        p.fast_startup =
            registry::dword(Hive::LocalMachine, POWER_KEY, "HiberbootEnabled").map(|v| v != 0);
    }

    // ---- Devices ----------------------------------------------------------------------------

    /// A present device: (instance ID, setup class, friendly name or description).
    struct Dev {
        id: String,
        class: String,
        name: String,
    }

    fn prop(set: HDEVINFO, d: &SP_DEVINFO_DATA, p: SETUP_DI_REGISTRY_PROPERTY) -> Option<String> {
        let mut buf = [0u8; 512];
        // SAFETY: valid device info set and element; the buffer slice is passed with its length.
        unsafe { SetupDiGetDeviceRegistryPropertyW(set, d, p, None, Some(&mut buf), None) }.ok()?;
        Some(from_wide_bytes(&buf)).filter(|s| !s.is_empty())
    }

    fn present_devices() -> Vec<Dev> {
        // SAFETY: enumerates all present devices; no class, enumerator or window.
        let Ok(set) = (unsafe {
            SetupDiGetClassDevsW(None, PCWSTR::null(), None, DIGCF_PRESENT | DIGCF_ALLCLASSES)
        }) else {
            return Vec::new();
        };
        // SAFETY: the set was just created and is destroyed exactly once, on drop.
        let set = unsafe { Owned::new(set) };
        let mut out = Vec::new();
        for i in 0..4096 {
            let mut d = SP_DEVINFO_DATA {
                cbSize: size_of::<SP_DEVINFO_DATA>() as u32,
                ..Default::default()
            };
            // SAFETY: valid set; cbSize of the out struct is set as required.
            if unsafe { SetupDiEnumDeviceInfo(*set, i, &mut d) }.is_err() {
                break;
            }
            let mut id = [0u16; 512];
            // SAFETY: valid set and element; the buffer slice is passed with its length.
            if unsafe { SetupDiGetDeviceInstanceIdW(*set, &d, Some(&mut id), None) }.is_err() {
                continue;
            }
            let id = from_wide(&id);
            let upper = id.to_ascii_uppercase();
            if !(upper.starts_with(r"USB\")
                || upper.starts_with(r"HID\VID_")
                || upper.starts_with(r"PCI\"))
            {
                continue;
            }
            out.push(Dev {
                class: prop(*set, &d, SPDRP_CLASS).unwrap_or_default(),
                name: prop(*set, &d, SPDRP_FRIENDLYNAME)
                    .or_else(|| prop(*set, &d, SPDRP_DEVICEDESC))
                    .unwrap_or_default(),
                id,
            });
        }
        out
    }

    fn sensitive_usb(devs: &[Dev]) -> Vec<(String, UsbSensitive)> {
        let mut out: Vec<(String, UsbSensitive)> = Vec::new();
        for d in devs {
            let upper = d.id.to_ascii_uppercase();
            if upper.starts_with(r"PCI\") || upper.starts_with(r"USB\ROOT_HUB") {
                continue;
            }
            if let Some(k) = power::classify_usb(&d.class, &d.name, &d.id)
                && !out.iter().any(|(n, kk)| *n == d.name && *kk == k)
            {
                out.push((d.name.clone(), k));
            }
        }
        out
    }

    fn adapters(devs: &[Dev]) -> Vec<AdapterPower> {
        let wmi = crate::win::wmi::device_power_enable();
        let allow = |id: &str| {
            wmi.iter()
                .find(|(n, _)| n.strip_suffix("_0").unwrap_or(n).eq_ignore_ascii_case(id))
                .map(|(_, on)| *on)
        };
        let present = |id: &str| devs.iter().any(|d| d.id.eq_ignore_ascii_case(id));
        let mut out = Vec::new();
        for sub in registry::subkeys(Hive::LocalMachine, NET_CLASS) {
            if sub.len() != 4 || !sub.chars().all(|c| c.is_ascii_digit()) {
                continue;
            }
            let key = format!(r"{NET_CLASS}\{sub}");
            let chars = registry::dword(Hive::LocalMachine, &key, "Characteristics").unwrap_or(0);
            if chars & NCF_PHYSICAL == 0 {
                continue;
            }
            let Some(kind) = registry::dword(Hive::LocalMachine, &key, "*PhysicalMediaType")
                .and_then(AdapterKind::from_media)
                .filter(|k| *k != AdapterKind::Bluetooth)
            else {
                continue;
            };
            let Some(id) = registry::string(Hive::LocalMachine, &key, "DeviceInstanceID") else {
                continue;
            };
            if !present(&id) {
                continue;
            }
            let eee = power::EEE_VALUES.iter().find_map(|v| {
                let data = registry::string(Hive::LocalMachine, &key, v)?;
                // Only properties the driver exposes on its Advanced tab.
                registry::key_exists(Hive::LocalMachine, &format!(r"{key}\Ndi\Params\{v}")).then(
                    || EeeSetting {
                        value: v.to_string(),
                        enabled: data.trim() != "0",
                    },
                )
            });
            out.push(AdapterPower {
                name: registry::string(Hive::LocalMachine, &key, "DriverDesc").unwrap_or_default(),
                kind,
                allow_turn_off: allow(&id),
                instance_id: id,
                class_key: Some(sub),
                eee,
            });
        }
        for d in devs
            .iter()
            .filter(|d| d.class.eq_ignore_ascii_case("Bluetooth"))
        {
            out.push(AdapterPower {
                name: d.name.clone(),
                kind: AdapterKind::Bluetooth,
                instance_id: d.id.clone(),
                class_key: None,
                allow_turn_off: allow(&d.id),
                eee: None,
            });
        }
        out
    }

    /// The device power API session (`DevicePowerOpen`), closed on drop.
    struct DevicePowerSession;

    impl DevicePowerSession {
        fn open() -> Option<DevicePowerSession> {
            // SAFETY: no pointers; balanced by DevicePowerClose in Drop.
            unsafe { DevicePowerOpen(None) }.then_some(DevicePowerSession)
        }
    }

    impl Drop for DevicePowerSession {
        fn drop(&mut self) {
            // SAFETY: matches the successful DevicePowerOpen in `open`.
            unsafe { DevicePowerClose() };
        }
    }

    /// Devices allowed to wake the PC (`powercfg /devicequery wake_armed`).
    fn wake_armed() -> Vec<String> {
        let Some(_session) = DevicePowerSession::open() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for i in 0..256 {
            let mut buf = [0u16; 512];
            let mut size = (buf.len() * 2) as u32;
            // SAFETY: open session; the buffer size in bytes is passed in `size`.
            let ok = unsafe {
                DevicePowerEnumDevices(
                    i,
                    DEVICEPOWER_FILTER_DEVICES_PRESENT | DEVICEPOWER_FILTER_WAKEENABLED,
                    0,
                    Some(buf.as_mut_ptr().cast()),
                    &mut size,
                )
            };
            if !ok {
                break;
            }
            let name = from_wide(&buf);
            if !name.is_empty() && !out.contains(&name) {
                out.push(name);
            }
        }
        out
    }

    /// Recent Power-Troubleshooter "returned from a low power state" events, newest first.
    fn wake_history() -> Vec<power::WakeEvent> {
        let xpath = format!(
            "*[System[Provider[@Name='Microsoft-Windows-Power-Troubleshooter'] and EventID=1 and TimeCreated[timediff(@SystemTime) <= {WAKE_HISTORY_MS}]]]"
        );
        crate::win::eventlog::query_system(&xpath, 50, power::parse_wake_event).unwrap_or_default()
    }

    fn powercfg(arg: &str) -> Option<String> {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let root = std::env::var_os("SystemRoot")?;
        let exe = std::path::Path::new(&root)
            .join("System32")
            .join("powercfg.exe");
        let out = std::process::Command::new(exe)
            .arg(arg)
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// Raw `powercfg /requests` and `/waketimers` output (administrator only).
    pub fn query_admin_raw() -> Option<(String, String)> {
        if !crate::win::is_elevated() {
            return None;
        }
        Some((
            powercfg("/requests")?,
            powercfg("/waketimers").unwrap_or_default(),
        ))
    }

    pub fn gather_devices(p: &mut PowerInfo) {
        let devs = present_devices();
        p.sensitive_usb = sensitive_usb(&devs);
        p.adapters = adapters(&devs);
        p.wake_armed = wake_armed();
        p.wakes = wake_history();
        if let Some((r, t)) = query_admin_raw() {
            p.requests = Some(power::parse_requests(&r));
            p.wake_timers = Some(power::parse_wake_timers(&t));
        }
    }

    // ---- Writes -----------------------------------------------------------------------------

    fn write_value(setting: Setting, source: Source, value: u32) -> Result<(), ApplyError> {
        let scheme =
            active_scheme().ok_or_else(|| ApplyError::Failed("No active power plan".into()))?;
        let (sub, set) = (
            GUID::from_u128(setting.subgroup()),
            GUID::from_u128(setting.guid()),
        );
        if matches!(source, Source::Ac | Source::Both) {
            // SAFETY: valid GUID pointers to locals.
            let st =
                unsafe { PowerWriteACValueIndex(None, &scheme, Some(&sub), Some(&set), value) };
            if st != ERROR_SUCCESS {
                return Err(denied(st));
            }
        }
        if matches!(source, Source::Dc | Source::Both) {
            // SAFETY: valid GUID pointers to locals.
            let st =
                unsafe { PowerWriteDCValueIndex(None, &scheme, Some(&sub), Some(&set), value) };
            if st != ERROR_SUCCESS.0 {
                return Err(denied(WIN32_ERROR(st)));
            }
        }
        // Re-apply the plan so the change takes effect now.
        // SAFETY: valid GUID pointer.
        let st = unsafe { PowerSetActiveScheme(None, Some(&scheme)) };
        if st == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(denied(st))
        }
    }

    /// Restart a device so it picks up changed advanced properties (like Device Manager does).
    fn restart_device(instance_id: &str) -> Result<(), ApplyError> {
        let fail = |e: windows::core::Error| {
            if e.code() == ERROR_ACCESS_DENIED.to_hresult() {
                ApplyError::AccessDenied
            } else {
                ApplyError::Failed(e.message())
            }
        };
        // SAFETY: an empty set is created and destroyed (on drop) exactly once; the instance id
        // is a NUL-terminated HSTRING; every struct's size field is set as required and the
        // params size covers the full SP_PROPCHANGE_PARAMS its header starts.
        unsafe {
            let set = Owned::new(SetupDiCreateDeviceInfoList(None, None).map_err(fail)?);
            let mut d = SP_DEVINFO_DATA {
                cbSize: size_of::<SP_DEVINFO_DATA>() as u32,
                ..Default::default()
            };
            SetupDiOpenDeviceInfoW(*set, &HSTRING::from(instance_id), None, 0, Some(&mut d))
                .map_err(fail)?;
            let params = SP_PROPCHANGE_PARAMS {
                ClassInstallHeader: SP_CLASSINSTALL_HEADER {
                    cbSize: size_of::<SP_CLASSINSTALL_HEADER>() as u32,
                    InstallFunction: DIF_PROPERTYCHANGE,
                },
                StateChange: DICS_PROPCHANGE,
                Scope: DICS_FLAG_CONFIGSPECIFIC,
                HwProfile: 0,
            };
            SetupDiSetClassInstallParamsW(
                *set,
                Some(&d),
                Some(&params.ClassInstallHeader),
                size_of::<SP_PROPCHANGE_PARAMS>() as u32,
            )
            .map_err(fail)?;
            SetupDiCallClassInstaller(DIF_PROPERTYCHANGE, *set, Some(&d)).map_err(fail)
        }
    }

    pub fn apply(action: &Action) -> Result<(), ApplyError> {
        match action {
            Action::SetScheme(g) => {
                let g = GUID::try_from(g.as_str())
                    .map_err(|_| ApplyError::Failed("Invalid plan".into()))?;
                // SAFETY: valid GUID pointer.
                let st = unsafe { PowerSetActiveScheme(None, Some(&g)) };
                if st == ERROR_SUCCESS {
                    Ok(())
                } else {
                    Err(denied(st))
                }
            }
            Action::SetValue {
                setting,
                source,
                value,
            } => write_value(*setting, *source, *value),
            Action::FastStartup(on) => registry::set_dword(
                Hive::LocalMachine,
                POWER_KEY,
                "HiberbootEnabled",
                u32::from(*on),
            )
            .map_err(denied),
            Action::AllowTurnOff { instance_id, allow } => {
                let name = crate::win::wmi::device_power_enable()
                    .into_iter()
                    .map(|(n, _)| n)
                    .find(|n| {
                        n.strip_suffix("_0")
                            .unwrap_or(n)
                            .eq_ignore_ascii_case(instance_id)
                    })
                    .ok_or_else(|| {
                        ApplyError::Failed("The device has no power management setting".into())
                    })?;
                crate::win::wmi::set_device_power_enable(&name, *allow).map_err(|e| {
                    // WBEM_E_ACCESS_DENIED or E_ACCESSDENIED.
                    if matches!(e.code().0 as u32, 0x8004_1003 | 0x8007_0005) {
                        ApplyError::AccessDenied
                    } else {
                        ApplyError::Failed(e.message())
                    }
                })
            }
            Action::NicProperty {
                class_key,
                value,
                data,
            } => {
                if class_key.len() != 4 || !power::is_eee_value(value) {
                    return Err(ApplyError::Failed("Unsupported adapter property".into()));
                }
                let key = format!(r"{NET_CLASS}\{class_key}");
                let id = registry::string(Hive::LocalMachine, &key, "DeviceInstanceID")
                    .ok_or_else(|| ApplyError::Failed("Network adapter not found".into()))?;
                registry::set_string(Hive::LocalMachine, &key, value, data).map_err(denied)?;
                restart_device(&id)
            }
            Action::DisarmWake(name) => {
                let _session = DevicePowerSession::open()
                    .ok_or_else(|| ApplyError::Failed("Device power API unavailable".into()))?;
                // SAFETY: open session; NUL-terminated device name that outlives the call.
                let st = unsafe {
                    DevicePowerSetDeviceState(
                        &HSTRING::from(name.as_str()),
                        DEVICEPOWER_CLEAR_WAKEENABLED,
                        None,
                    )
                };
                if st == ERROR_SUCCESS.0 {
                    Ok(())
                } else {
                    Err(denied(WIN32_ERROR(st)))
                }
            }
        }
    }

    #[cfg(test)]
    mod tests {
        /// Prints what the collector sees on this PC: `cargo test -p omnihw-core -- --ignored --nocapture`.
        #[test]
        #[ignore]
        fn dump_power_info() {
            let t = std::time::Instant::now();
            let p = crate::power::PowerInfo::gather();
            println!("{p:#?}\ngathered in {:?}", t.elapsed());
        }
    }
}

#[cfg(windows)]
pub(crate) use imp::{active_scheme, scheme_name};
pub use imp::{apply, gather_devices, gather_settings, query_admin_raw};
