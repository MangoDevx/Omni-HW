//! Network adapters: addresses and link state from IP Helper (`GetAdaptersAddresses`,
//! `GetIfEntry2`), Wi-Fi details from the WLAN API, and driver details, disabled adapters and
//! advanced properties from WMI (`root\StandardCimv2` `MSFT_NetAdapter*`). Nothing here sends
//! anything over the network; see [`probe`] for the tests that do.

mod ops;
pub mod probe;
mod wlan;

pub use ops::apply;

use super::registry::{self, Hive};
use super::wmi::{self, Wmi};
use crate::network::{
    Adapter, AdvancedProperty, Driver, Duplex, IpEntry, Media, NetworkInfo, Status, format_mac,
    max_speed, normalize_mac,
};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use windows::Win32::Foundation::{ERROR_BUFFER_OVERFLOW, NO_ERROR};
use windows::Win32::NetworkManagement::IpHelper::{
    GAA_FLAG_INCLUDE_GATEWAYS, GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_MULTICAST,
    GetAdaptersAddresses, GetIfEntry2, IP_ADAPTER_ADDRESSES_LH, IP_ADAPTER_DHCP_ENABLED,
    MIB_IF_ROW2,
};
use windows::Win32::NetworkManagement::Ndis::{IfOperStatusUp, MediaConnectStateConnected};
use windows::Win32::Networking::WinSock::{
    AF_INET, AF_INET6, AF_UNSPEC, IpDadStateDuplicate, IpPrefixOriginDhcp, SOCKADDR_IN,
    SOCKADDR_IN6, SOCKET_ADDRESS,
};

/// `InterfaceAndOperStatusFlags` bit: a real hardware adapter.
const HARDWARE_INTERFACE: u8 = 1 << 0;
/// Interface types left out: loopback and the Teredo/ISATAP/6to4 tunnels Windows creates.
const SKIP_IF_TYPES: [u32; 2] = [24, 131];

pub fn gather(detailed: bool, previous: Option<&NetworkInfo>) -> NetworkInfo {
    let mut adapters = ip_adapters();
    for (guid, w) in wlan::connections() {
        if let Some(a) = adapters
            .iter_mut()
            .find(|a| a.guid.eq_ignore_ascii_case(&guid))
        {
            a.media = Media::WiFi;
            a.wifi = Some(w);
        }
    }
    let mut read_details = false;
    if detailed {
        read_details = wmi_details(&mut adapters).is_some();
    }
    if !read_details && let Some(prev) = previous {
        carry_over(&mut adapters, prev);
    }
    adapters.sort_by(|a, b| {
        b.physical
            .cmp(&a.physical)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    NetworkInfo {
        adapters,
        detailed: read_details || previous.is_some_and(|p| p.detailed),
    }
}

/// Keep the WMI-only details (and disabled adapters) of the last detailed read.
fn carry_over(adapters: &mut Vec<Adapter>, prev: &NetworkInfo) {
    if prev.detailed {
        // Adapters Network Connections hides (Wi-Fi Direct, WAN miniports) stay hidden.
        adapters.retain(|a| prev.adapters.iter().any(|p| p.guid == a.guid));
    }
    for p in &prev.adapters {
        match adapters.iter_mut().find(|a| a.guid == p.guid) {
            Some(a) => {
                a.driver = p.driver.clone();
                a.advanced = p.advanced.clone();
                a.max_speed_bps = p.max_speed_bps;
                a.physical |= p.physical;
                if a.status == Status::Up {
                    a.duplex = p.duplex;
                }
                if p.media != Media::Other {
                    a.media = p.media;
                }
            }
            None if p.status == Status::Disabled => adapters.push(p.clone()),
            None => {}
        }
    }
}

/// Owned buffer for `GetAdaptersAddresses` (u64 elements for alignment).
fn adapters_buffer() -> Option<Vec<u64>> {
    let flags = GAA_FLAG_INCLUDE_GATEWAYS | GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST;
    let mut size: u32 = 32 * 1024;
    for _ in 0..4 {
        let mut buf = vec![0u64; (size as usize).div_ceil(8)];
        // SAFETY: the buffer holds `size` writable bytes, suitably aligned for the structs.
        let r = unsafe {
            GetAdaptersAddresses(
                u32::from(AF_UNSPEC.0),
                flags,
                None,
                Some(buf.as_mut_ptr().cast()),
                &mut size,
            )
        };
        if r == NO_ERROR.0 {
            return Some(buf);
        }
        if r != ERROR_BUFFER_OVERFLOW.0 {
            return None;
        }
    }
    None
}

/// An address from a `SOCKET_ADDRESS`.
fn sock_addr(s: &SOCKET_ADDRESS) -> Option<IpAddr> {
    if s.lpSockaddr.is_null() {
        return None;
    }
    // SAFETY: a non-null sockaddr from GetAdaptersAddresses, valid while the buffer lives; the
    // family says which struct it is and `iSockaddrLength` covers it.
    unsafe {
        let family = (*s.lpSockaddr).sa_family;
        if family == AF_INET && s.iSockaddrLength as usize >= size_of::<SOCKADDR_IN>() {
            let a = &*(s.lpSockaddr as *const SOCKADDR_IN);
            Some(IpAddr::V4(Ipv4Addr::from(
                a.sin_addr.S_un.S_addr.to_ne_bytes(),
            )))
        } else if family == AF_INET6 && s.iSockaddrLength as usize >= size_of::<SOCKADDR_IN6>() {
            let a = &*(s.lpSockaddr as *const SOCKADDR_IN6);
            Some(IpAddr::V6(Ipv6Addr::from(a.sin6_addr.u.Byte)))
        } else {
            None
        }
    }
}

/// Walk a NULL-terminated linked list of IP Helper structs.
///
/// # Safety
/// `first` must be null or point to a valid list whose nodes stay alive for `'a`.
unsafe fn list<'a, T>(first: *mut T, next: impl Fn(&T) -> *mut T) -> Vec<&'a T> {
    let mut out = Vec::new();
    let mut p = first;
    while !p.is_null() && out.len() < 256 {
        // SAFETY: non-null node of a valid list (guaranteed by the caller).
        let node = unsafe { &*p };
        out.push(node);
        p = next(node);
    }
    out
}

/// Link state of an interface from `GetIfEntry2`: (physical, connected, speed).
fn if_entry(index: u32) -> Option<(bool, bool, Option<u64>)> {
    let mut row = MIB_IF_ROW2 {
        InterfaceIndex: index,
        ..Default::default()
    };
    // SAFETY: valid row with the index to look up.
    if unsafe { GetIfEntry2(&mut row) } != NO_ERROR {
        return None;
    }
    let physical = row.InterfaceAndOperStatusFlags._bitfield & HARDWARE_INTERFACE != 0;
    let up =
        row.OperStatus == IfOperStatusUp && row.MediaConnectState == MediaConnectStateConnected;
    let speed = Some(row.ReceiveLinkSpeed).filter(|&s| s != 0 && s != u64::MAX && up);
    Some((physical, up, speed))
}

/// DNS servers were typed in (the interface has a `NameServer` value) rather than from DHCP.
fn dns_set_by_hand(guid: &str) -> bool {
    let subkey = format!(r"SYSTEM\CurrentControlSet\Services\Tcpip\Parameters\Interfaces\{guid}");
    registry::string(Hive::LocalMachine, &subkey, "NameServer")
        .is_some_and(|v| !v.trim().is_empty())
}

fn ip_adapters() -> Vec<Adapter> {
    let Some(buf) = adapters_buffer() else {
        return Vec::new();
    };
    let first = buf.as_ptr() as *mut IP_ADAPTER_ADDRESSES_LH;
    // SAFETY: GetAdaptersAddresses filled `buf` with a linked list that lives as long as `buf`.
    let nodes = unsafe { list(first, |a: &IP_ADAPTER_ADDRESSES_LH| a.Next) };
    let mut out = Vec::new();
    for a in nodes {
        if SKIP_IF_TYPES.contains(&a.IfType) {
            continue;
        }
        // SAFETY: the strings and lists below point into `buf` (or are null) and are only read
        // while it lives.
        let (guid, name, description, dns_suffix) = unsafe {
            (
                a.AdapterName.to_string().unwrap_or_default().to_uppercase(),
                a.FriendlyName.to_string().unwrap_or_default(),
                a.Description.to_string().unwrap_or_default(),
                a.DnsSuffix.to_string().unwrap_or_default(),
            )
        };
        // SAFETY: as above; `Flags` is the plain-integer view of the union.
        let flags = unsafe { a.Anonymous2.Flags };
        // SAFETY: as above; IfIndex is the plain view of the first union.
        let if_index = unsafe { a.Anonymous1.Anonymous.IfIndex };
        let index = if if_index != 0 {
            if_index
        } else {
            a.Ipv6IfIndex
        };
        let (physical, connected, speed) =
            if_entry(index).unwrap_or((false, a.OperStatus == IfOperStatusUp, None));
        let dns_static = dns_set_by_hand(&guid);
        let mut ad = Adapter {
            guid,
            if_index: index,
            name,
            description,
            media: Media::from_if_type(a.IfType),
            physical,
            status: if connected {
                Status::Up
            } else {
                Status::Disconnected
            },
            mac: format_mac(&a.PhysicalAddress[..(a.PhysicalAddressLength as usize).min(8)]),
            speed_bps: speed,
            mtu: Some(a.Mtu).filter(|&m| m != 0 && m != u32::MAX),
            dns_suffix,
            dhcp_enabled: flags & IP_ADAPTER_DHCP_ENABLED != 0,
            dhcp_server: sock_addr(&a.Dhcpv4Server),
            dns_static,
            ..Default::default()
        };
        // SAFETY: lists inside `buf`.
        let unicast = unsafe { list(a.FirstUnicastAddress, |u| u.Next) };
        for u in unicast {
            let Some(addr) = sock_addr(&u.Address) else {
                continue;
            };
            let dhcp = u.PrefixOrigin == IpPrefixOriginDhcp;
            let e = IpEntry {
                addr,
                prefix: u.OnLinkPrefixLength,
                duplicate: u.DadState == IpDadStateDuplicate,
                dhcp,
                lease_secs: (dhcp && addr.is_ipv4()).then_some(u.LeaseLifetime),
            };
            match addr {
                IpAddr::V4(_) => ad.ipv4.push(e),
                IpAddr::V6(_) => ad.ipv6.push(e),
            }
        }
        // SAFETY: lists inside `buf`.
        let gateways = unsafe { list(a.FirstGatewayAddress, |g| g.Next) };
        ad.gateways = gateways
            .iter()
            .filter_map(|g| sock_addr(&g.Address))
            .filter(|g| !g.is_unspecified())
            .collect();
        // SAFETY: lists inside `buf`.
        let dns = unsafe { list(a.FirstDnsServerAddress, |d| d.Next) };
        ad.dns = dns
            .iter()
            .filter_map(|d| sock_addr(&d.Address))
            // fec0:0:0:ffff::1-3 are placeholders Windows lists when IPv6 DNS is unset.
            .filter(|d| !matches!(d, IpAddr::V6(v) if v.segments()[0] == 0xfec0))
            .collect();
        out.push(ad);
    }
    out
}

/// Driver details, duplex, disabled adapters and advanced properties from WMI. Adapters
/// Windows hides from Network Connections are dropped. `None` if WMI couldn't be read.
fn wmi_details(adapters: &mut Vec<Adapter>) -> Option<()> {
    let w = Wmi::connect(r"root\StandardCimv2").ok()?;
    let rows = w.query("SELECT * FROM MSFT_NetAdapter").ok()?;
    if rows.is_empty() {
        return None;
    }
    // MSFT_NetAdapter lists what Network Connections shows (hidden adapters only on request).
    let mut visible = Vec::new();
    for o in &rows {
        let Some(guid) = wmi::string(o, "InterfaceGuid").map(|g| g.to_uppercase()) else {
            continue;
        };
        if wmi::boolean(o, "Hidden") == Some(true) {
            continue;
        }
        visible.push(guid.clone());
        let disabled = wmi::uint(o, "State") == Some(3);
        let pos = match adapters.iter().position(|a| a.guid == guid) {
            Some(i) => i,
            None => {
                adapters.push(Adapter {
                    guid: guid.clone(),
                    if_index: if disabled {
                        0
                    } else {
                        wmi::uint(o, "InterfaceIndex").unwrap_or(0)
                    },
                    name: wmi::string(o, "Name").unwrap_or_default(),
                    description: wmi::string(o, "InterfaceDescription").unwrap_or_default(),
                    status: if disabled {
                        Status::Disabled
                    } else {
                        Status::Unknown
                    },
                    mac: wmi::string(o, "PermanentAddress")
                        .map(|m| normalize_mac(&m))
                        .unwrap_or_default(),
                    ..Default::default()
                });
                adapters.len() - 1
            }
        };
        let a = &mut adapters[pos];
        if let Some(m) = wmi::uint(o, "NdisPhysicalMedium").and_then(Media::from_physical_medium)
            && a.media != Media::WiFi
        {
            a.media = m;
        }
        if wmi::boolean(o, "HardwareInterface") == Some(true) {
            a.physical = true;
        }
        a.duplex = match wmi::uint(o, "MediaDuplexState") {
            Some(1) if a.status == Status::Up => Some(Duplex::Half),
            Some(2) if a.status == Status::Up => Some(Duplex::Full),
            _ => None,
        };
        let driver = Driver {
            version: wmi::string(o, "DriverVersionString").unwrap_or_default(),
            date: wmi::string(o, "DriverDate").unwrap_or_default(),
            provider: wmi::string(o, "DriverProvider").unwrap_or_default(),
        };
        if !driver.version.is_empty() || !driver.date.is_empty() {
            a.driver = Some(driver);
        }
        if a.status == Status::Up && a.speed_bps.is_none() {
            a.speed_bps = wmi::uint64(o, "Speed").filter(|&s| s != 0 && s != u64::MAX);
        }
    }
    adapters.retain(|a| visible.contains(&a.guid));
    advanced_properties(&w, adapters);
    Some(())
}

fn advanced_properties(w: &Wmi, adapters: &mut [Adapter]) {
    let Ok(rows) = w.query(
        "SELECT InstanceID, DisplayName, DisplayValue, RegistryKeyword, RegistryValue, \
         ValidDisplayValues, ValidRegistryValues FROM MSFT_NetAdapterAdvancedPropertySettingData",
    ) else {
        return;
    };
    for o in &rows {
        // "{GUID}::Keyword"
        let Some(id) = wmi::string(o, "InstanceID") else {
            continue;
        };
        let guid = id.split("::").next().unwrap_or_default().to_uppercase();
        let Some(a) = adapters.iter_mut().find(|a| a.guid == guid) else {
            continue;
        };
        let Some(keyword) = wmi::string(o, "RegistryKeyword") else {
            continue;
        };
        let regs = wmi::strings(o, "ValidRegistryValues");
        let disp = wmi::strings(o, "ValidDisplayValues");
        let choices = if regs.len() == disp.len() {
            regs.into_iter().zip(disp).collect()
        } else {
            Vec::new()
        };
        a.advanced.push(AdvancedProperty {
            display_name: wmi::string(o, "DisplayName").unwrap_or_else(|| keyword.clone()),
            keyword,
            display_value: wmi::string(o, "DisplayValue").unwrap_or_default(),
            value: wmi::strings(o, "RegistryValue")
                .into_iter()
                .next()
                .unwrap_or_default(),
            choices,
        });
    }
    for a in adapters {
        a.advanced.sort_by_key(|x| x.display_name.to_lowercase());
        a.max_speed_bps = a.property("*SpeedDuplex").and_then(max_speed);
    }
}
