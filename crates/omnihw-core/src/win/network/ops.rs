//! Applying [`NetAction`]s. Enable, disable and rename go through WMI `MSFT_NetAdapter`, the MTU
//! through `MSFT_NetIPInterface`, and release / renew through
//! `Win32_NetworkAdapterConfiguration`. DHCP, static IPv4 and DNS run `netsh interface ipv4`
//! on the interface index (WMI is unreliable on adapters that aren't connected). Advanced
//! driver properties are written straight to the adapter's driver key in the registry (found by
//! its `NetCfgInstanceId`), then the adapter is disabled and enabled so the driver reloads them.
//! Flushing the DNS cache and the Winsock / TCP/IP resets run `ipconfig` / `netsh`. Tools run
//! without a console window, with arguments passed as separate argv entries (never a shell).
//! Everything but the DNS flush needs administrator rights.

use crate::network::{NetAction, prefix_mask};
use crate::win::registry::{self, Hive, NET_CLASS};
use crate::win::wmi::{self, Value, Wmi};
use std::net::Ipv4Addr;
use std::process::Output;
use windows::Win32::Foundation::{ERROR_ACCESS_DENIED, NO_ERROR};
use windows::Win32::NetworkManagement::IpHelper::{
    FreeMibTable, GetIpForwardTable2, GetUnicastIpAddressTable, MIB_IPFORWARD_TABLE2,
    MIB_UNICASTIPADDRESS_TABLE,
};
use windows::Win32::Networking::WinSock::{
    AF_INET, IpPrefixOriginManual, MIB_IPPROTO_NETMGMT, SOCKADDR_INET,
};
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

/// Win32_NetworkAdapterConfiguration release / renew results: 0 = done, 1 = done after a restart.
fn config_result(code: u32) -> Result<(), String> {
    match code {
        0 | 1 => Ok(()),
        64 => Err("not supported on this adapter".into()),
        67 => Err("an error occurred while processing the request".into()),
        82 => Err("the DHCP lease could not be renewed (no DHCP server answered)".into()),
        83 => Err("the DHCP lease could not be released".into()),
        84 => Err("the adapter isn't connected".into()),
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

/// Run a system tool from System32 without a console window and return its output.
fn run_tool_output(exe: &str, args: &[impl AsRef<std::ffi::OsStr>]) -> Result<Output, String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
    let path = std::path::Path::new(&root).join("System32").join(exe);
    std::process::Command::new(&path)
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("couldn't run {exe}: {e}"))
}

/// "exe failed: <first line of its output>".
fn tool_failure(exe: &str, out: &Output) -> String {
    let text = String::from_utf8_lossy(&out.stdout);
    match text.lines().map(str::trim).find(|l| !l.is_empty()) {
        Some(l) => format!("{exe} failed: {l}"),
        None => format!("{exe} failed"),
    }
}

/// Run a system tool; a non-zero exit is an error carrying the tool's first output line.
fn run_tool(exe: &str, args: &[impl AsRef<std::ffi::OsStr>]) -> Result<(), String> {
    let out = run_tool_output(exe, args)?;
    if out.status.success() {
        Ok(())
    } else {
        Err(tool_failure(exe, &out))
    }
}

/// `netsh interface ipv4 set address` for a static address; `gateway=none` drops a stale gateway.
fn static_args(if_index: u32, ip: Ipv4Addr, prefix: u8, gateway: Option<Ipv4Addr>) -> Vec<String> {
    vec![
        "interface".into(),
        "ipv4".into(),
        "set".into(),
        "address".into(),
        format!("name={if_index}"),
        "source=static".into(),
        format!("address={ip}"),
        format!("mask={}", prefix_mask(prefix)),
        match gateway {
            Some(g) => format!("gateway={g}"),
            None => "gateway=none".into(),
        },
    ]
}

/// `netsh interface ipv4 set address ... source=dhcp`.
fn dhcp_address_args(if_index: u32) -> Vec<String> {
    vec![
        "interface".into(),
        "ipv4".into(),
        "set".into(),
        "address".into(),
        format!("name={if_index}"),
        "source=dhcp".into(),
    ]
}

/// The `netsh` calls that set the DNS servers: automatic when empty, else the first server
/// with `set` and each following one with `add` at its 1-based position. `validate=no` stops
/// netsh from contacting the servers.
fn dns_args(if_index: u32, servers: &[Ipv4Addr]) -> Vec<Vec<String>> {
    let name = format!("name={if_index}");
    let Some((first, rest)) = servers.split_first() else {
        return vec![vec![
            "interface".into(),
            "ipv4".into(),
            "set".into(),
            "dnsservers".into(),
            name,
            "source=dhcp".into(),
        ]];
    };
    let mut calls = vec![vec![
        "interface".into(),
        "ipv4".into(),
        "set".into(),
        "dnsservers".into(),
        name.clone(),
        "source=static".into(),
        format!("address={first}"),
        "register=primary".into(),
        "validate=no".into(),
    ]];
    for (i, server) in rest.iter().enumerate() {
        calls.push(vec![
            "interface".into(),
            "ipv4".into(),
            "add".into(),
            "dnsservers".into(),
            name.clone(),
            format!("address={server}"),
            format!("index={}", i + 2),
            "validate=no".into(),
        ]);
    }
    calls
}

/// `netsh interface ipv4 delete address`; `gateway=all` also drops the gateways set with it.
fn delete_address_args(if_index: u32, ip: Ipv4Addr) -> Vec<String> {
    vec![
        "interface".into(),
        "ipv4".into(),
        "delete".into(),
        "address".into(),
        format!("name={if_index}"),
        format!("address={ip}"),
        "gateway=all".into(),
    ]
}

/// `netsh interface ipv4 delete route` for a manual default route (persistent store).
fn delete_gateway_args(if_index: u32, gateway: Ipv4Addr) -> Vec<String> {
    vec![
        "interface".into(),
        "ipv4".into(),
        "delete".into(),
        "route".into(),
        "prefix=0.0.0.0/0".into(),
        format!("interface={if_index}"),
        format!("nexthop={gateway}"),
        "store=persistent".into(),
    ]
}

fn v4(a: &SOCKADDR_INET) -> Ipv4Addr {
    // SAFETY: the tables below are requested with AF_INET, so the IPv4 member is the active one.
    let raw = unsafe { a.Ipv4.sin_addr.S_un.S_addr };
    Ipv4Addr::from(raw.to_ne_bytes())
}

/// Manually configured IPv4 addresses and default-route next hops of an interface.
fn manual_config(if_index: u32) -> (Vec<Ipv4Addr>, Vec<Ipv4Addr>) {
    let mut addrs = Vec::new();
    let mut table: *mut MIB_UNICASTIPADDRESS_TABLE = std::ptr::null_mut();
    // SAFETY: valid out pointer; on success the table is freed below with FreeMibTable.
    if unsafe { GetUnicastIpAddressTable(AF_INET, &mut table) } == NO_ERROR && !table.is_null() {
        // SAFETY: the table holds `NumEntries` rows and stays allocated until FreeMibTable.
        let rows = unsafe {
            std::slice::from_raw_parts((*table).Table.as_ptr(), (*table).NumEntries as usize)
        };
        for r in rows {
            if r.InterfaceIndex == if_index && r.PrefixOrigin == IpPrefixOriginManual {
                addrs.push(v4(&r.Address));
            }
        }
        // SAFETY: allocated by GetUnicastIpAddressTable, freed exactly once; `rows` is not used after.
        unsafe { FreeMibTable(table.cast()) };
    }
    let mut gateways = Vec::new();
    let mut table: *mut MIB_IPFORWARD_TABLE2 = std::ptr::null_mut();
    // SAFETY: valid out pointer; on success the table is freed below with FreeMibTable.
    if unsafe { GetIpForwardTable2(AF_INET, &mut table) } == NO_ERROR && !table.is_null() {
        // SAFETY: the table holds `NumEntries` rows and stays allocated until FreeMibTable.
        let rows = unsafe {
            std::slice::from_raw_parts((*table).Table.as_ptr(), (*table).NumEntries as usize)
        };
        for r in rows {
            if r.InterfaceIndex == if_index
                && r.DestinationPrefix.PrefixLength == 0
                && r.Protocol == MIB_IPPROTO_NETMGMT
            {
                gateways.push(v4(&r.NextHop));
            }
        }
        // SAFETY: allocated by GetIpForwardTable2, freed exactly once; `rows` is not used after.
        unsafe { FreeMibTable(table.cast()) };
    }
    (addrs, gateways)
}

/// Run a netsh delete; "Element not found." (already gone) counts as success.
fn run_delete(args: &[String]) -> Result<(), String> {
    let out = run_tool_output("netsh.exe", args)?;
    let gone = String::from_utf8_lossy(&out.stdout)
        .to_lowercase()
        .contains("element not found");
    if out.status.success() || gone {
        Ok(())
    } else {
        Err(tool_failure("netsh.exe", &out))
    }
}

/// The class subkey of the adapter whose `NetCfgInstanceId` equals `guid` (case-insensitive),
/// from (subkey, NetCfgInstanceId) pairs.
fn driver_key(
    entries: impl IntoIterator<Item = (String, Option<String>)>,
    guid: &str,
) -> Option<String> {
    entries
        .into_iter()
        .find(|(_, id)| {
            id.as_deref()
                .is_some_and(|id| id.eq_ignore_ascii_case(guid))
        })
        .map(|(sub, _)| sub)
}

/// Call an MSFT_NetAdapter method ("Enable" / "Disable") on the adapter with this GUID.
fn adapter_method(guid: &str, method: &str) -> Result<(), String> {
    let w = Wmi::connect(STD_CIMV2).map_err(wmi_err)?;
    let a = adapter(&w, guid)?;
    method_result(
        w.call(&a, "MSFT_NetAdapter", method, &[])
            .map_err(wmi_err)?,
    )
}

/// Write an advanced driver property (REG_SZ) in the adapter's driver key.
fn set_driver_value(guid: &str, keyword: &str, value: &str) -> Result<(), String> {
    let entries = registry::subkeys(Hive::LocalMachine, NET_CLASS)
        .into_iter()
        .map(|sub| {
            let id = registry::string(
                Hive::LocalMachine,
                &format!(r"{NET_CLASS}\{sub}"),
                "NetCfgInstanceId",
            );
            (sub, id)
        });
    let sub = driver_key(entries, guid).ok_or("The adapter's driver settings were not found")?;
    registry::set_string(
        Hive::LocalMachine,
        &format!(r"{NET_CLASS}\{sub}"),
        keyword,
        value,
    )
    .map_err(|e| {
        if e == ERROR_ACCESS_DENIED {
            "administrator rights are required".to_string()
        } else {
            windows::core::Error::from(e.to_hresult()).message()
        }
    })
}

/// Apply a validated action. Blocks.
pub fn apply(action: &NetAction) -> Result<(), String> {
    match action {
        NetAction::Enable { guid, .. } => adapter_method(guid, "Enable"),
        NetAction::Disable { guid, .. } => adapter_method(guid, "Disable"),
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
            // A disconnected adapter keeps DHCP "enabled" while still holding a manual address
            // and gateway, so `source=dhcp` would do nothing: remove those first.
            let (addrs, gateways) = manual_config(*if_index);
            for ip in addrs {
                run_delete(&delete_address_args(*if_index, ip))?;
            }
            for gw in gateways {
                run_delete(&delete_gateway_args(*if_index, gw))?;
            }
            let out = run_tool_output("netsh.exe", &dhcp_address_args(*if_index))?;
            // netsh fails with "DHCP is already enabled on this interface." when it already is.
            let already = String::from_utf8_lossy(&out.stdout)
                .to_lowercase()
                .contains("already enabled");
            if !out.status.success() && !already {
                return Err(tool_failure("netsh.exe", &out));
            }
            // DNS servers come from DHCP again.
            for call in dns_args(*if_index, &[]) {
                run_tool("netsh.exe", &call)?;
            }
            Ok(())
        }
        NetAction::StaticIp {
            if_index,
            ip,
            prefix,
            gateway,
            ..
        } => run_tool("netsh.exe", &static_args(*if_index, *ip, *prefix, *gateway)),
        NetAction::Dns {
            if_index, servers, ..
        } => {
            for call in dns_args(*if_index, servers) {
                run_tool("netsh.exe", &call)?;
            }
            Ok(())
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
            {
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
            }
            // The driver reads its settings when it loads: write the value, then restart the
            // adapter (what Device Manager does).
            set_driver_value(guid, keyword, value)?;
            adapter_method(guid, "Disable")?;
            adapter_method(guid, "Enable")
        }
        NetAction::FlushDns => run_tool("ipconfig.exe", &["/flushdns"]),
        NetAction::ResetWinsock => run_tool("netsh.exe", &["winsock", "reset"]),
        NetAction::ResetTcpIp => run_tool("netsh.exe", &["int", "ip", "reset"]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(a: u8, b: u8, c: u8, d: u8) -> Ipv4Addr {
        Ipv4Addr::new(a, b, c, d)
    }

    #[test]
    fn static_with_and_without_gateway() {
        assert_eq!(
            static_args(7, ip(192, 168, 1, 50), 24, Some(ip(192, 168, 1, 1))),
            [
                "interface",
                "ipv4",
                "set",
                "address",
                "name=7",
                "source=static",
                "address=192.168.1.50",
                "mask=255.255.255.0",
                "gateway=192.168.1.1"
            ]
        );
        let a = static_args(7, ip(10, 0, 0, 5), 20, None);
        assert_eq!(a[7], "mask=255.255.240.0");
        assert_eq!(a[8], "gateway=none");
    }

    #[test]
    fn delete_args() {
        assert_eq!(
            delete_address_args(15, ip(192, 168, 1, 50)),
            [
                "interface",
                "ipv4",
                "delete",
                "address",
                "name=15",
                "address=192.168.1.50",
                "gateway=all"
            ]
        );
        assert_eq!(
            delete_gateway_args(15, ip(192, 168, 1, 1)),
            [
                "interface",
                "ipv4",
                "delete",
                "route",
                "prefix=0.0.0.0/0",
                "interface=15",
                "nexthop=192.168.1.1",
                "store=persistent"
            ]
        );
    }

    #[test]
    fn dhcp_args_use_the_index() {
        let a = dhcp_address_args(12);
        assert_eq!(a[4], "name=12");
        assert_eq!(a[5], "source=dhcp");
    }

    #[test]
    fn dns_none_is_automatic() {
        assert_eq!(
            dns_args(3, &[]),
            [[
                "interface",
                "ipv4",
                "set",
                "dnsservers",
                "name=3",
                "source=dhcp"
            ]]
        );
    }

    #[test]
    fn dns_one_server() {
        assert_eq!(
            dns_args(3, &[ip(1, 1, 1, 1)]),
            [[
                "interface",
                "ipv4",
                "set",
                "dnsservers",
                "name=3",
                "source=static",
                "address=1.1.1.1",
                "register=primary",
                "validate=no"
            ]]
        );
    }

    #[test]
    fn dns_three_servers_are_indexed_from_two() {
        let calls = dns_args(3, &[ip(1, 1, 1, 1), ip(8, 8, 8, 8), ip(9, 9, 9, 9)]);
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[0][2], "set");
        assert_eq!(
            calls[1],
            [
                "interface",
                "ipv4",
                "add",
                "dnsservers",
                "name=3",
                "address=8.8.8.8",
                "index=2",
                "validate=no"
            ]
        );
        assert_eq!(calls[2][5], "address=9.9.9.9");
        assert_eq!(calls[2][6], "index=3");
    }

    #[test]
    fn driver_key_matches_guid_ignoring_case() {
        let entries = vec![
            ("0000".to_string(), None),
            ("0001".to_string(), Some("{AAAA-1}".to_string())),
            ("0002".to_string(), Some("{abcd-2}".to_string())),
        ];
        assert_eq!(
            driver_key(entries.clone(), "{ABCD-2}").as_deref(),
            Some("0002")
        );
        assert_eq!(driver_key(entries, "{ffff-3}"), None);
    }
}
