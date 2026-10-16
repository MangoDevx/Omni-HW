//! Applying [`NetAction`]s. Adapter changes go through WMI: `MSFT_NetAdapter` (enable, disable,
//! rename), `MSFT_NetIPInterface` (MTU), `MSFT_NetAdapterAdvancedPropertySettingData` (driver
//! properties) and `Win32_NetworkAdapterConfiguration` (DHCP, static IPv4, DNS, release/renew).
//! Flushing the DNS cache and the Winsock / TCP/IP resets have no API and run `ipconfig` /
//! `netsh` without a console window. Everything but the DNS flush needs administrator rights.

use crate::network::{NetAction, prefix_mask};
use crate::win::wmi::{self, Value, Wmi};
use windows::Win32::System::Wmi::IWbemClassObject;

const STD_CIMV2: &str = r"root\StandardCimv2";

fn wmi_err(e: windows::core::Error) -> String {
    // WBEM_E_ACCESS_DENIED / E_ACCESSDENIED
    if matches!(e.code().0 as u32, 0x8004_1003 | 0x8007_0005) {
        "administrator rights are required".into()
    } else {
        let m = e.message();
        if m.is_empty() {
            format!("WMI error {:#010x}", e.code().0)
        } else {
            m
        }
    }
}

fn one(w: &Wmi, wql: &str, what: &str) -> Result<IWbemClassObject, String> {
    w.query(wql)
        .map_err(wmi_err)?
        .into_iter()
        .next()
        .ok_or_else(|| format!("{what} was not found (it may have been removed or disabled)"))
}

/// The MSFT_NetAdapter for a validated "{GUID}".
fn adapter(w: &Wmi, guid: &str) -> Result<IWbemClassObject, String> {
    one(
        w,
        &format!("SELECT * FROM MSFT_NetAdapter WHERE InterfaceGuid = '{guid}'"),
        "The network adapter",
    )
}

/// The Win32_NetworkAdapterConfiguration for an interface index.
fn config(w: &Wmi, if_index: u32) -> Result<IWbemClassObject, String> {
    one(
        w,
        &format!(
            "SELECT * FROM Win32_NetworkAdapterConfiguration WHERE InterfaceIndex = {if_index}"
        ),
        "The network adapter",
    )
}

/// Win32_NetworkAdapterConfiguration method results: 0 = done, 1 = done after a restart.
fn config_result(code: u32) -> Result<(), String> {
    match code {
        0 | 1 => Ok(()),
        64 => Err("not supported on this adapter".into()),
        66 => Err("invalid subnet mask".into()),
        67 => Err("an error occurred while processing the request".into()),
        70 => Err("invalid IP address".into()),
        71 => Err("invalid gateway address".into()),
        81 => Err("DHCP could not be enabled".into()),
        82 => Err("the DHCP lease could not be renewed (no DHCP server answered)".into()),
        83 => Err("the DHCP lease could not be released".into()),
        84 => Err("IP is not enabled on this adapter".into()),
        91 => Err("administrator rights are required".into()),
        c => Err(format!("Windows returned error {c}")),
    }
}

fn method_result(code: u32) -> Result<(), String> {
    match code {
        0 => Ok(()),
        5 => Err("administrator rights are required".into()),
        c => Err(format!("Windows returned error {c}")),
    }
}

/// Run a system tool from System32 without a console window.
fn run_tool(exe: &str, args: &[&str]) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
    let path = std::path::Path::new(&root).join("System32").join(exe);
    let out = std::process::Command::new(&path)
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("couldn't run {exe}: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        let text = String::from_utf8_lossy(&out.stdout);
        let line = text.lines().map(str::trim).find(|l| !l.is_empty());
        Err(match line {
            Some(l) => format!("{exe} failed: {l}"),
            None => format!("{exe} failed"),
        })
    }
}

/// Apply a validated action. Blocks.
pub fn apply(action: &NetAction) -> Result<(), String> {
    match action {
        NetAction::Enable { guid, .. } | NetAction::Disable { guid, .. } => {
            let method = if matches!(action, NetAction::Enable { .. }) {
                "Enable"
            } else {
                "Disable"
            };
            let w = Wmi::connect(STD_CIMV2).map_err(wmi_err)?;
            let a = adapter(&w, guid)?;
            method_result(
                w.call(&a, "MSFT_NetAdapter", method, &[])
                    .map_err(wmi_err)?,
            )
        }
        NetAction::Rename { guid, new_name, .. } => {
            let w = Wmi::connect(STD_CIMV2).map_err(wmi_err)?;
            let a = adapter(&w, guid)?;
            method_result(
                w.call(
                    &a,
                    "MSFT_NetAdapter",
                    "Rename",
                    &[("NewName", Value::Str(new_name))],
                )
                .map_err(wmi_err)?,
            )
        }
        NetAction::Dhcp { if_index, .. } => {
            let w = Wmi::connect(r"root\cimv2").map_err(wmi_err)?;
            let c = config(&w, *if_index)?;
            let cls = "Win32_NetworkAdapterConfiguration";
            config_result(w.call(&c, cls, "EnableDHCP", &[]).map_err(wmi_err)?)?;
            // No server list: DNS servers come from DHCP again.
            config_result(
                w.call(&c, cls, "SetDNSServerSearchOrder", &[])
                    .map_err(wmi_err)?,
            )
        }
        NetAction::StaticIp {
            if_index,
            ip,
            prefix,
            gateway,
            ..
        } => {
            let w = Wmi::connect(r"root\cimv2").map_err(wmi_err)?;
            let c = config(&w, *if_index)?;
            let cls = "Win32_NetworkAdapterConfiguration";
            let ips = [ip.to_string()];
            let masks = [prefix_mask(*prefix).to_string()];
            config_result(
                w.call(
                    &c,
                    cls,
                    "EnableStatic",
                    &[
                        ("IPAddress", Value::Strs(&ips)),
                        ("SubnetMask", Value::Strs(&masks)),
                    ],
                )
                .map_err(wmi_err)?,
            )?;
            let gws: Vec<String> = gateway.iter().map(|g| g.to_string()).collect();
            if gws.is_empty() {
                return Ok(());
            }
            config_result(
                w.call(
                    &c,
                    cls,
                    "SetGateways",
                    &[("DefaultIPGateway", Value::Strs(&gws))],
                )
                .map_err(wmi_err)?,
            )
        }
        NetAction::Dns {
            if_index, servers, ..
        } => {
            let w = Wmi::connect(r"root\cimv2").map_err(wmi_err)?;
            let c = config(&w, *if_index)?;
            let list: Vec<String> = servers.iter().map(|s| s.to_string()).collect();
            let args: &[(&str, Value)] = if list.is_empty() {
                &[]
            } else {
                &[("DNSServerSearchOrder", Value::Strs(&list))]
            };
            config_result(
                w.call(
                    &c,
                    "Win32_NetworkAdapterConfiguration",
                    "SetDNSServerSearchOrder",
                    args,
                )
                .map_err(wmi_err)?,
            )
        }
        NetAction::Release { if_index, .. } | NetAction::Renew { if_index, .. } => {
            let method = if matches!(action, NetAction::Release { .. }) {
                "ReleaseDHCPLease"
            } else {
                "RenewDHCPLease"
            };
            let w = Wmi::connect(r"root\cimv2").map_err(wmi_err)?;
            let c = config(&w, *if_index)?;
            config_result(
                w.call(&c, "Win32_NetworkAdapterConfiguration", method, &[])
                    .map_err(wmi_err)?,
            )
        }
        NetAction::Mtu { if_index, mtu, .. } => {
            let w = Wmi::connect(STD_CIMV2).map_err(wmi_err)?;
            let rows = w
                .query(&format!(
                    "SELECT * FROM MSFT_NetIPInterface WHERE InterfaceIndex = {if_index}"
                ))
                .map_err(wmi_err)?;
            if rows.is_empty() {
                return Err("The network interface was not found".into());
            }
            let value = i32::try_from(*mtu).map_err(|_| "invalid MTU".to_string())?;
            for r in &rows {
                // IPv6 needs at least 1280.
                if wmi::uint(r, "AddressFamily") == Some(23) && *mtu < 1280 {
                    continue;
                }
                w.put(r, &[("NlMtu", Value::I32(value))]).map_err(wmi_err)?;
            }
            Ok(())
        }
        NetAction::Advanced {
            guid,
            keyword,
            value,
            ..
        } => {
            let w = Wmi::connect(STD_CIMV2).map_err(wmi_err)?;
            // The keyword is validated to [A-Za-z0-9*_], so it is safe inside the WQL string.
            let rows = w
                .query(&format!(
                    "SELECT * FROM MSFT_NetAdapterAdvancedPropertySettingData WHERE RegistryKeyword = '{keyword}'"
                ))
                .map_err(wmi_err)?;
            let prefix = format!("{}::", guid.to_uppercase());
            let row = rows
                .iter()
                .find(|r| {
                    wmi::string(r, "InstanceID")
                        .is_some_and(|id| id.to_uppercase().starts_with(&prefix))
                })
                .ok_or("This adapter doesn't have that setting")?;
            if !wmi::strings(row, "ValidRegistryValues").contains(value) {
                return Err(format!("\"{value}\" is not an allowed value for {keyword}"));
            }
            w.put(
                row,
                &[("RegistryValue", Value::Strs(std::slice::from_ref(value)))],
            )
            .map_err(wmi_err)
        }
        NetAction::FlushDns => run_tool("ipconfig.exe", &["/flushdns"]),
        NetAction::ResetWinsock => run_tool("netsh.exe", &["winsock", "reset"]),
        NetAction::ResetTcpIp => run_tool("netsh.exe", &["int", "ip", "reset"]),
    }
}
