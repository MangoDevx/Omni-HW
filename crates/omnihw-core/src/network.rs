//! Network adapters: the data model behind the Network page, the changes it can make (encoded
//! for the elevated helper and validated again there), and the logic of the local and internet
//! tests. The adapter reads and changes aren't implemented on Linux yet (empty data).
//!
//! Nothing in this module talks to the network by itself. The local tests only contact the
//! adapter's own gateway and DNS servers on the local network; the internet tests (see
//! [`INTERNET_TARGETS`]) run only when the user clicks their button.

use crate::checkup::format_link_speed;
use std::net::{IpAddr, Ipv4Addr};

/// Every adapter Windows shows in Network Connections, plus how much detail was read.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NetworkInfo {
    pub adapters: Vec<Adapter>,
    /// Driver details, duplex, disabled adapters and advanced properties were read (WMI).
    pub detailed: bool,
}

impl NetworkInfo {
    /// Read the adapters. `detailed` adds the slower WMI details (driver, duplex, disabled
    /// adapters, advanced properties); otherwise those are carried over from `previous`.
    pub fn gather(detailed: bool, previous: Option<&NetworkInfo>) -> NetworkInfo {
        imp::gather(detailed, previous)
    }

    /// Adapters that are up with an address, i.e. carrying traffic.
    pub fn connected(&self) -> impl Iterator<Item = &Adapter> {
        self.adapters.iter().filter(|a| a.status == Status::Up)
    }
}

mod imp {
    use super::NetworkInfo;
    pub fn gather(_detailed: bool, _previous: Option<&NetworkInfo>) -> NetworkInfo {
        NetworkInfo::default()
    }
}

/// Kind of network connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Media {
    Ethernet,
    WiFi,
    Cellular,
    Bluetooth,
    /// VPN and other tunnels.
    Tunnel,
    #[default]
    Other,
}

impl Media {
    pub fn name(self) -> &'static str {
        match self {
            Media::Ethernet => "Ethernet",
            Media::WiFi => "Wi-Fi",
            Media::Cellular => "Cellular",
            Media::Bluetooth => "Bluetooth",
            Media::Tunnel => "Tunnel / VPN",
            Media::Other => "Other",
        }
    }

    /// From the IANA interface type (`IfType`).
    pub fn from_if_type(t: u32) -> Media {
        match t {
            6 => Media::Ethernet,
            71 => Media::WiFi,
            243 | 244 => Media::Cellular,
            131 | 53 => Media::Tunnel,
            _ => Media::Other,
        }
    }

    /// From the NDIS physical medium (`NdisPhysicalMedium`), when it says more than the type.
    pub fn from_physical_medium(m: u32) -> Option<Media> {
        match m {
            14 => Some(Media::Ethernet),
            1 | 9 => Some(Media::WiFi),
            8 => Some(Media::Cellular),
            10 => Some(Media::Bluetooth),
            _ => None,
        }
    }
}

/// Connection state, as Network Connections shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Status {
    /// Connected and passing traffic.
    Up,
    /// Enabled, but no cable / not associated.
    Disconnected,
    Disabled,
    #[default]
    Unknown,
}

impl Status {
    pub fn name(self) -> &'static str {
        match self {
            Status::Up => "Connected",
            Status::Disconnected => "Disconnected",
            Status::Disabled => "Disabled",
            Status::Unknown => "Unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Duplex {
    Half,
    Full,
}

impl Duplex {
    pub fn name(self) -> &'static str {
        match self {
            Duplex::Half => "half duplex",
            Duplex::Full => "full duplex",
        }
    }
}

/// An IP address assigned to an adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IpEntry {
    pub addr: IpAddr,
    pub prefix: u8,
    /// Another device on the network uses the same address (duplicate address detection).
    pub duplicate: bool,
    /// Handed out by DHCP.
    pub dhcp: bool,
    /// Seconds left on the DHCP lease (IPv4 DHCP addresses).
    pub lease_secs: Option<u32>,
}

impl IpEntry {
    /// "192.168.1.20/24".
    pub fn text(&self) -> String {
        format!("{}/{}", self.addr, self.prefix)
    }
}

/// Wi-Fi association details.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Wifi {
    /// Network name; `None` when Windows hides it (location permission off).
    pub ssid: Option<String>,
    /// Signal quality 0-100 %.
    pub signal_pct: Option<u32>,
    pub rssi_dbm: Option<i32>,
    pub channel: Option<u32>,
    /// Centre frequency in MHz, when known.
    pub freq_mhz: Option<u32>,
    pub band: Option<Band>,
    /// e.g. "Wi-Fi 6 (802.11ax)".
    pub phy: Option<&'static str>,
    pub rx_mbps: Option<u32>,
    pub tx_mbps: Option<u32>,
}

/// Wi-Fi frequency band.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Band {
    G2_4,
    G5,
    G6,
}

impl Band {
    pub fn name(self) -> &'static str {
        match self {
            Band::G2_4 => "2.4 GHz",
            Band::G5 => "5 GHz",
            Band::G6 => "6 GHz",
        }
    }

    /// From a centre frequency in MHz.
    pub fn from_mhz(mhz: u32) -> Option<Band> {
        match mhz {
            2400..=2500 => Some(Band::G2_4),
            4900..=5924 => Some(Band::G5),
            5925..=7125 => Some(Band::G6),
            _ => None,
        }
    }

    /// Best guess from a channel number alone (6 GHz channels overlap 2.4/5 GHz numbers, so
    /// this only tells 2.4 from 5).
    pub fn from_channel(ch: u32) -> Option<Band> {
        match ch {
            1..=14 => Some(Band::G2_4),
            32..=177 => Some(Band::G5),
            _ => None,
        }
    }
}

/// Channel number for a centre frequency in MHz.
pub fn channel_from_mhz(mhz: u32) -> Option<u32> {
    match mhz {
        2484 => Some(14),
        2412..=2472 => Some((mhz - 2407) / 5),
        5955..=7115 => Some((mhz - 5950) / 5),
        5160..=5885 => Some((mhz - 5000) / 5),
        _ => None,
    }
}

/// Wi-Fi generation from the `DOT11_PHY_TYPE`.
pub fn phy_name(t: i32) -> Option<&'static str> {
    Some(match t {
        4 => "802.11a",
        5 => "802.11b",
        6 => "802.11g",
        7 => "Wi-Fi 4 (802.11n)",
        8 => "Wi-Fi 5 (802.11ac)",
        10 => "Wi-Fi 6 (802.11ax)",
        11 => "Wi-Fi 7 (802.11be)",
        _ => return None,
    })
}

/// An advanced driver property (Device Manager > adapter > Properties > Advanced).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AdvancedProperty {
    /// Registry keyword, e.g. "*SpeedDuplex".
    pub keyword: String,
    /// e.g. "Speed & Duplex".
    pub display_name: String,
    /// Current value as shown in Device Manager.
    pub display_value: String,
    /// Current registry value.
    pub value: String,
    /// Allowed (registry value, display value) pairs; empty for free-form numbers and text,
    /// which are shown read-only.
    pub choices: Vec<(String, String)>,
}

/// Driver details from Windows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Driver {
    pub version: String,
    /// "2024-03-11".
    pub date: String,
    pub provider: String,
}

/// One network adapter.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Adapter {
    /// Interface GUID, "{xxxxxxxx-...}" upper-case.
    pub guid: String,
    /// Interface index (0 when the adapter is disabled).
    pub if_index: u32,
    /// Connection name, e.g. "Ethernet 2".
    pub name: String,
    /// Adapter model, e.g. "Intel(R) Ethernet Controller I225-V".
    pub description: String,
    pub media: Media,
    /// A real (hardware) adapter rather than a virtual one.
    pub physical: bool,
    pub status: Status,
    /// "D8-BB-C1-12-34-56".
    pub mac: String,
    /// Current link speed in bits per second.
    pub speed_bps: Option<u64>,
    /// Fastest speed the adapter offers in its Speed & Duplex setting.
    pub max_speed_bps: Option<u64>,
    pub duplex: Option<Duplex>,
    pub mtu: Option<u32>,
    pub ipv4: Vec<IpEntry>,
    pub ipv6: Vec<IpEntry>,
    pub gateways: Vec<IpAddr>,
    pub dns: Vec<IpAddr>,
    pub dns_suffix: String,
    /// IPv4 address from DHCP (vs set by hand).
    pub dhcp_enabled: bool,
    pub dhcp_server: Option<IpAddr>,
    /// DNS servers were set by hand (not from DHCP).
    pub dns_static: bool,
    pub driver: Option<Driver>,
    pub wifi: Option<Wifi>,
    pub advanced: Vec<AdvancedProperty>,
}

impl Adapter {
    /// Only self-assigned 169.254.x.x addresses: DHCP didn't answer.
    pub fn apipa_only(&self) -> bool {
        !self.ipv4.is_empty() && self.ipv4.iter().all(|e| is_apipa(e.addr))
    }

    /// A usable (not self-assigned) IPv4 address.
    pub fn has_usable_ipv4(&self) -> bool {
        self.ipv4.iter().any(|e| !is_apipa(e.addr) && !e.duplicate)
    }

    pub fn has_duplicate(&self) -> bool {
        self.ipv4.iter().chain(&self.ipv6).any(|e| e.duplicate)
    }

    /// "1 Gbps, full duplex".
    pub fn link_text(&self) -> String {
        match (self.speed_bps, self.duplex) {
            (Some(s), Some(d)) => format!("{}, {}", format_link_speed(s), d.name()),
            (Some(s), None) => format_link_speed(s),
            _ => String::new(),
        }
    }

    /// The advanced property with this keyword.
    pub fn property(&self, keyword: &str) -> Option<&AdvancedProperty> {
        self.advanced
            .iter()
            .find(|p| p.keyword.eq_ignore_ascii_case(keyword))
    }

    /// An Ethernet adapter linked slower than its Speed & Duplex setting allows.
    pub fn below_max_speed(&self) -> Option<(u64, u64)> {
        let (cur, max) = (self.speed_bps?, self.max_speed_bps?);
        (self.media == Media::Ethernet && self.status == Status::Up && cur < max)
            .then_some((cur, max))
    }
}

/// 169.254.0.0/16, the address Windows picks itself when no DHCP server answers.
pub fn is_apipa(a: IpAddr) -> bool {
    matches!(a, IpAddr::V4(v) if v.is_link_local())
}

/// An address on the local network (private, link-local or unique-local), so contacting it
/// stays off the internet.
pub fn is_local(a: IpAddr) -> bool {
    match a {
        IpAddr::V4(v) => v.is_private() || v.is_link_local() || v.is_loopback(),
        IpAddr::V6(v) => {
            let s = v.segments()[0];
            v.is_loopback() || (s & 0xfe00) == 0xfc00 || (s & 0xffc0) == 0xfe80
        }
    }
}

/// "D8-BB-C1-12-34-56" from raw bytes (empty for none).
pub fn format_mac(b: &[u8]) -> String {
    b.iter()
        .map(|x| format!("{x:02X}"))
        .collect::<Vec<_>>()
        .join("-")
}

/// Normalize a MAC from WMI ("D8BBC1123456" or "D8-BB-C1-12-34-56").
pub fn normalize_mac(s: &str) -> String {
    let hex: String = s.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    if hex.len() != 12 {
        return s.to_string();
    }
    let bytes: Vec<u8> = (0..6)
        .filter_map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok())
        .collect();
    format_mac(&bytes)
}

/// "6h 12m" for a DHCP lease.
pub fn format_lease(secs: u32) -> String {
    if secs == u32::MAX {
        return "never expires".into();
    }
    format!("expires in {}", crate::format_uptime(u64::from(secs)))
}

/// Speed in bits per second from an advanced-property display value such as
/// "2.5 Gbps Full Duplex", "1.0 Gbps Full Duplex" or "100 Mbps Half Duplex".
pub fn speed_from_display(s: &str) -> Option<u64> {
    let lower = s.to_ascii_lowercase();
    let (unit_at, mult) = ["gbps", "gb", "g ", "mbps", "mb", "m "]
        .iter()
        .find_map(|u| {
            let i = lower.find(u)?;
            Some((i, if u.starts_with('g') { 1e9 } else { 1e6 }))
        })?;
    let num: String = lower[..unit_at]
        .trim_end()
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let v: f64 = num.parse().ok()?;
    (v > 0.0).then_some((v * mult) as u64)
}

/// Fastest speed listed in a Speed & Duplex property's choices.
pub fn max_speed(p: &AdvancedProperty) -> Option<u64> {
    p.choices
        .iter()
        .filter_map(|(_, d)| speed_from_display(d))
        .max()
}

/// Subnet prefix length from "24" or "255.255.255.0".
pub fn parse_prefix(s: &str) -> Option<u8> {
    let s = s.trim().trim_start_matches('/');
    if let Ok(n) = s.parse::<u8>() {
        return (1..=32).contains(&n).then_some(n);
    }
    let m: Ipv4Addr = s.parse().ok()?;
    let bits = u32::from(m);
    let n = bits.leading_ones();
    (n >= 1 && bits.checked_shl(n).unwrap_or(0) == 0).then_some(n as u8)
}

/// Dotted subnet mask for a prefix length.
pub fn prefix_mask(prefix: u8) -> Ipv4Addr {
    let p = u32::from(prefix.min(32));
    Ipv4Addr::from(if p == 0 { 0 } else { u32::MAX << (32 - p) })
}

/// Parse "1.1.1.1, 8.8.8.8" (commas, semicolons or spaces). Errors name the bad entry.
pub fn parse_ipv4_list(s: &str) -> Result<Vec<Ipv4Addr>, String> {
    s.split([',', ';', ' '])
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(|t| {
            t.parse()
                .map_err(|_| format!("\"{t}\" is not an IPv4 address"))
        })
        .collect()
}

fn usable_host(ip: Ipv4Addr) -> bool {
    !(ip.is_unspecified()
        || ip.is_broadcast()
        || ip.is_multicast()
        || ip.is_loopback()
        || ip.is_link_local())
}

fn valid_guid(g: &str) -> bool {
    let inner = g.strip_prefix('{').and_then(|s| s.strip_suffix('}'));
    inner.is_some_and(|s| {
        let parts: Vec<&str> = s.split('-').collect();
        parts.iter().map(|p| p.len()).eq([8, 4, 4, 4, 12])
            && parts
                .iter()
                .all(|p| p.chars().all(|c| c.is_ascii_hexdigit()))
    })
}

/// Connection names Windows accepts: 1-64 characters, none of `\ / : * ? " < > |` or control
/// characters, no leading/trailing spaces.
pub fn valid_adapter_name(n: &str) -> Result<(), String> {
    if n.is_empty() || n.chars().count() > 64 {
        return Err("The name must be 1 to 64 characters long".into());
    }
    if n.trim() != n {
        return Err("The name can't start or end with a space".into());
    }
    if n.chars()
        .any(|c| c.is_control() || r#"\/:*?"<>|"#.contains(c))
    {
        return Err(r#"The name can't contain \ / : * ? " < > |"#.into());
    }
    Ok(())
}

/// Lowest and highest MTU that may be set.
pub const MTU_RANGE: std::ops::RangeInclusive<u32> = 576..=9216;
/// Most DNS servers that may be set at once.
pub const MAX_DNS: usize = 4;

/// A change to a network adapter or the network stack. Runs elevated (see `needs_admin`)
/// after the user confirms it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetAction {
    Enable {
        guid: String,
        name: String,
    },
    Disable {
        guid: String,
        name: String,
    },
    Rename {
        guid: String,
        name: String,
        new_name: String,
    },
    /// Get the IPv4 address and DNS servers automatically (DHCP).
    Dhcp {
        if_index: u32,
        name: String,
    },
    /// A fixed IPv4 address.
    StaticIp {
        if_index: u32,
        name: String,
        ip: Ipv4Addr,
        prefix: u8,
        gateway: Option<Ipv4Addr>,
    },
    /// IPv4 DNS servers; empty means "automatic" (from DHCP).
    Dns {
        if_index: u32,
        name: String,
        servers: Vec<Ipv4Addr>,
    },
    Mtu {
        if_index: u32,
        name: String,
        mtu: u32,
    },
    Release {
        if_index: u32,
        name: String,
    },
    Renew {
        if_index: u32,
        name: String,
    },
    /// Set an advanced driver property to one of its allowed values.
    Advanced {
        guid: String,
        name: String,
        keyword: String,
        value: String,
        /// The property's name and the chosen option as shown to the user (display only).
        display_name: String,
        display_value: String,
    },
    FlushDns,
    ResetWinsock,
    ResetTcpIp,
}

impl NetAction {
    /// Short description for buttons and the status bar.
    pub fn label(&self) -> String {
        match self {
            NetAction::Enable { name, .. } => format!("Enable {name}"),
            NetAction::Disable { name, .. } => format!("Disable {name}"),
            NetAction::Rename { name, new_name, .. } => {
                format!("Rename \"{name}\" to \"{new_name}\"")
            }
            NetAction::Dhcp { name, .. } => format!("Get an IP address automatically on {name}"),
            NetAction::StaticIp {
                name,
                ip,
                prefix,
                gateway,
                ..
            } => format!(
                "Set {name} to {ip}/{prefix} (mask {}, {})",
                prefix_mask(*prefix),
                match gateway {
                    Some(g) => format!("gateway {g}"),
                    None => "no gateway".into(),
                }
            ),
            NetAction::Dns { name, servers, .. } if servers.is_empty() => {
                format!("Get DNS servers automatically on {name}")
            }
            NetAction::Dns { name, servers, .. } => format!(
                "Set DNS on {name} to {}",
                servers
                    .iter()
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            NetAction::Mtu { name, mtu, .. } => format!("Set MTU of {name} to {mtu}"),
            NetAction::Release { name, .. } => format!("Release the IP address of {name}"),
            NetAction::Renew { name, .. } => format!("Renew the IP address of {name}"),
            NetAction::Advanced {
                name,
                display_name,
                display_value,
                ..
            } => format!("Set {display_name} on {name} to {display_value}"),
            NetAction::FlushDns => "Flush the DNS cache".into(),
            NetAction::ResetWinsock => "Reset Winsock".into(),
            NetAction::ResetTcpIp => "Reset TCP/IP".into(),
        }
    }

    /// Needs administrator rights (everything but flushing the DNS cache).
    pub fn needs_admin(&self) -> bool {
        !matches!(self, NetAction::FlushDns)
    }

    /// Takes effect only after a restart.
    pub fn needs_restart(&self) -> bool {
        matches!(self, NetAction::ResetWinsock | NetAction::ResetTcpIp)
    }

    /// What the confirmation dialog says before applying.
    pub fn confirm_text(&self) -> &'static str {
        match self {
            NetAction::Enable { .. } => "The adapter is turned back on and reconnects.",
            NetAction::Disable { .. } => {
                "The adapter is turned off and its connection drops until you enable it again (here or in Network Connections)."
            }
            NetAction::Rename { .. } => "Only the connection's name changes.",
            NetAction::Dhcp { .. } => {
                "The adapter asks the router for its IPv4 address and DNS servers. The connection may drop for a few seconds."
            }
            NetAction::StaticIp { .. } => {
                "The adapter stops using DHCP and keeps this IPv4 address. A wrong address, mask or gateway cuts the connection until you switch back to automatic."
            }
            NetAction::Dns { .. } => {
                "Name lookups on this adapter go to these DNS servers. Wrong servers make websites fail to load."
            }
            NetAction::Mtu { .. } => {
                "A wrong MTU can make some websites or games hang. The default for Ethernet and Wi-Fi is 1500."
            }
            NetAction::Release { .. } => {
                "The adapter gives up its IP address and goes offline until you click Renew (or restart)."
            }
            NetAction::Renew { .. } => {
                "The adapter asks the DHCP server for an address again. The connection may drop for a few seconds."
            }
            NetAction::Advanced { .. } => {
                "The network adapter will restart: the connection drops for a few seconds. Unsaved downloads or calls may be interrupted."
            }
            NetAction::FlushDns => {
                "Windows forgets the names it has looked up and looks them up again."
            }
            NetAction::ResetWinsock => {
                "Resets the Windows network catalog to its defaults, which fixes many \"connected but no internet\" problems caused by broken VPN or security software. Some VPN or firewall apps may need reinstalling. Restart required."
            }
            NetAction::ResetTcpIp => {
                "Resets the TCP/IP settings stored in the registry to their defaults. Fixed IP addresses and DNS servers you set by hand may be lost. Restart required."
            }
        }
    }

    /// Check the action's values; `Err` explains what's wrong.
    pub fn validate(&self) -> Result<(), String> {
        let index = |i: &u32| {
            if *i == 0 {
                Err("Unknown network interface".to_string())
            } else {
                Ok(())
            }
        };
        let guid = |g: &str| {
            if valid_guid(g) {
                Ok(())
            } else {
                Err("Unknown network adapter".to_string())
            }
        };
        match self {
            NetAction::Enable { guid: g, .. } | NetAction::Disable { guid: g, .. } => guid(g),
            NetAction::Rename {
                guid: g, new_name, ..
            } => {
                guid(g)?;
                valid_adapter_name(new_name)
            }
            NetAction::Dhcp { if_index, .. }
            | NetAction::Release { if_index, .. }
            | NetAction::Renew { if_index, .. } => index(if_index),
            NetAction::StaticIp {
                if_index,
                ip,
                prefix,
                gateway,
                ..
            } => {
                index(if_index)?;
                if !usable_host(*ip) {
                    return Err(format!("{ip} can't be used as an adapter address"));
                }
                if !(8..=30).contains(prefix) {
                    return Err("The subnet prefix must be between /8 and /30".into());
                }
                let mask = u32::from(prefix_mask(*prefix));
                let host = u32::from(*ip) & !mask;
                if host == 0 || host == !mask {
                    return Err(format!(
                        "{ip} is the network or broadcast address of its subnet"
                    ));
                }
                if let Some(gw) = gateway {
                    if !usable_host(*gw) || gw == ip {
                        return Err(format!("{gw} can't be the gateway"));
                    }
                    if u32::from(*gw) & mask != u32::from(*ip) & mask {
                        return Err(format!(
                            "The gateway {gw} is not in the same subnet as {ip}/{prefix}"
                        ));
                    }
                }
                Ok(())
            }
            NetAction::Dns {
                if_index, servers, ..
            } => {
                index(if_index)?;
                if servers.len() > MAX_DNS {
                    return Err(format!("At most {MAX_DNS} DNS servers"));
                }
                match servers.iter().find(|s| !usable_host(**s)) {
                    Some(s) => Err(format!("{s} can't be a DNS server")),
                    None => Ok(()),
                }
            }
            NetAction::Mtu { if_index, mtu, .. } => {
                index(if_index)?;
                if MTU_RANGE.contains(mtu) {
                    Ok(())
                } else {
                    Err(format!(
                        "The MTU must be between {} and {}",
                        MTU_RANGE.start(),
                        MTU_RANGE.end()
                    ))
                }
            }
            NetAction::Advanced {
                guid: g,
                keyword,
                value,
                display_name,
                display_value,
                ..
            } => {
                guid(g)?;
                // Display text is never used to change anything, but it is shown in the prompt.
                let text_ok =
                    |t: &str| t.chars().count() <= 128 && !t.chars().any(char::is_control);
                if !text_ok(display_name) || !text_ok(display_value) {
                    return Err("Unknown adapter property".into());
                }
                let kw_ok = !keyword.is_empty()
                    && keyword.len() <= 64
                    && keyword
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '*' || c == '_');
                let value_ok = !value.is_empty()
                    && value.len() <= 64
                    && value.chars().all(|c| c.is_ascii_graphic() || c == ' ');
                if kw_ok && value_ok {
                    Ok(())
                } else {
                    Err("Unknown adapter property".into())
                }
            }
            NetAction::FlushDns | NetAction::ResetWinsock | NetAction::ResetTcpIp => Ok(()),
        }
    }

    /// Encode for the elevated helper's command line (hex, so no quoting issues).
    pub fn to_arg(&self) -> String {
        let opt = |g: &Option<Ipv4Addr>| g.map(|g| g.to_string()).unwrap_or_default();
        let fields: Vec<String> = match self {
            NetAction::Enable { guid, name } => vec!["enable".into(), guid.clone(), name.clone()],
            NetAction::Disable { guid, name } => {
                vec!["disable".into(), guid.clone(), name.clone()]
            }
            NetAction::Rename {
                guid,
                name,
                new_name,
            } => vec![
                "rename".into(),
                guid.clone(),
                name.clone(),
                new_name.clone(),
            ],
            NetAction::Dhcp { if_index, name } => {
                vec!["dhcp".into(), if_index.to_string(), name.clone()]
            }
            NetAction::StaticIp {
                if_index,
                name,
                ip,
                prefix,
                gateway,
            } => vec![
                "static".into(),
                if_index.to_string(),
                name.clone(),
                ip.to_string(),
                prefix.to_string(),
                opt(gateway),
            ],
            NetAction::Dns {
                if_index,
                name,
                servers,
            } => vec![
                "dns".into(),
                if_index.to_string(),
                name.clone(),
                servers
                    .iter()
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>()
                    .join(","),
            ],
            NetAction::Mtu {
                if_index,
                name,
                mtu,
            } => vec![
                "mtu".into(),
                if_index.to_string(),
                name.clone(),
                mtu.to_string(),
            ],
            NetAction::Release { if_index, name } => {
                vec!["release".into(), if_index.to_string(), name.clone()]
            }
            NetAction::Renew { if_index, name } => {
                vec!["renew".into(), if_index.to_string(), name.clone()]
            }
            NetAction::Advanced {
                guid,
                name,
                keyword,
                value,
                display_name,
                display_value,
            } => vec![
                "advanced".into(),
                guid.clone(),
                name.clone(),
                keyword.clone(),
                value.clone(),
                display_name.clone(),
                display_value.clone(),
            ],
            NetAction::FlushDns => vec!["flushdns".into()],
            NetAction::ResetWinsock => vec!["winsock".into()],
            NetAction::ResetTcpIp => vec!["tcpip".into()],
        };
        fields
            .join("\t")
            .bytes()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    /// Decode [`NetAction::to_arg`]; `None` for anything malformed or invalid.
    pub fn from_arg(arg: &str) -> Option<NetAction> {
        if arg.is_empty() || !arg.len().is_multiple_of(2) || arg.len() > 4096 {
            return None;
        }
        let bytes: Option<Vec<u8>> = (0..arg.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(arg.get(i..i + 2)?, 16).ok())
            .collect();
        let s = String::from_utf8(bytes?).ok()?;
        let f: Vec<&str> = s.split('\t').collect();
        let idx = |v: &str| v.parse::<u32>().ok();
        let a = match f.as_slice() {
            ["enable", g, n] => NetAction::Enable {
                guid: g.to_string(),
                name: n.to_string(),
            },
            ["disable", g, n] => NetAction::Disable {
                guid: g.to_string(),
                name: n.to_string(),
            },
            ["rename", g, n, new] => NetAction::Rename {
                guid: g.to_string(),
                name: n.to_string(),
                new_name: new.to_string(),
            },
            ["dhcp", i, n] => NetAction::Dhcp {
                if_index: idx(i)?,
                name: n.to_string(),
            },
            ["static", i, n, ip, p, gw] => NetAction::StaticIp {
                if_index: idx(i)?,
                name: n.to_string(),
                ip: ip.parse().ok()?,
                prefix: p.parse().ok()?,
                gateway: if gw.is_empty() {
                    None
                } else {
                    Some(gw.parse().ok()?)
                },
            },
            ["dns", i, n, list] => NetAction::Dns {
                if_index: idx(i)?,
                name: n.to_string(),
                servers: parse_ipv4_list(list).ok()?,
            },
            ["mtu", i, n, m] => NetAction::Mtu {
                if_index: idx(i)?,
                name: n.to_string(),
                mtu: m.parse().ok()?,
            },
            ["release", i, n] => NetAction::Release {
                if_index: idx(i)?,
                name: n.to_string(),
            },
            ["renew", i, n] => NetAction::Renew {
                if_index: idx(i)?,
                name: n.to_string(),
            },
            ["advanced", g, n, k, v, dn, dv] => NetAction::Advanced {
                guid: g.to_string(),
                name: n.to_string(),
                keyword: k.to_string(),
                value: v.to_string(),
                display_name: dn.to_string(),
                display_value: dv.to_string(),
            },
            ["flushdns"] => NetAction::FlushDns,
            ["winsock"] => NetAction::ResetWinsock,
            ["tcpip"] => NetAction::ResetTcpIp,
            _ => return None,
        };
        a.validate().ok()?;
        Some(a)
    }
}

/// Apply a change. Blocks.
pub fn apply(action: &NetAction) -> Result<(), String> {
    action.validate()?;
    Err("Not supported on Linux yet".into())
}

// ---- Tests (ping, DNS, HTTP) -------------------------------------------------------------

/// Round-trip times of a series of pings.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PingStats {
    pub sent: u32,
    /// Round-trip time in ms of each reply, in order.
    pub rtts: Vec<u32>,
}

impl PingStats {
    pub fn received(&self) -> u32 {
        self.rtts.len() as u32
    }

    /// Packet loss in percent.
    pub fn loss_pct(&self) -> f64 {
        if self.sent == 0 {
            return 0.0;
        }
        f64::from(self.sent - self.received().min(self.sent)) * 100.0 / f64::from(self.sent)
    }

    pub fn avg_ms(&self) -> Option<f64> {
        (!self.rtts.is_empty())
            .then(|| self.rtts.iter().map(|&r| f64::from(r)).sum::<f64>() / self.rtts.len() as f64)
    }

    pub fn min_ms(&self) -> Option<u32> {
        self.rtts.iter().copied().min()
    }

    pub fn max_ms(&self) -> Option<u32> {
        self.rtts.iter().copied().max()
    }

    /// Jitter: mean difference between consecutive round-trip times (RFC 3550 style, unsmoothed).
    pub fn jitter_ms(&self) -> Option<f64> {
        if self.rtts.len() < 2 {
            return None;
        }
        let sum: f64 = self
            .rtts
            .windows(2)
            .map(|w| (f64::from(w[0]) - f64::from(w[1])).abs())
            .sum();
        Some(sum / (self.rtts.len() - 1) as f64)
    }

    /// "12.3 ms avg (10-18), jitter 2.1 ms, 0 % loss" or "no replies".
    pub fn summary(&self) -> String {
        match (self.avg_ms(), self.min_ms(), self.max_ms()) {
            (Some(avg), Some(lo), Some(hi)) => {
                let jitter = self
                    .jitter_ms()
                    .map(|j| format!(", jitter {j:.1} ms"))
                    .unwrap_or_default();
                format!(
                    "{avg:.1} ms avg ({lo}-{hi}){jitter}, {:.0} % loss ({}/{} replies)",
                    self.loss_pct(),
                    self.received(),
                    self.sent
                )
            }
            _ => format!("no replies ({} sent)", self.sent),
        }
    }
}

/// Verdict of one test line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Pass,
    /// Works, but something isn't ideal.
    Warn,
    Fail,
    /// Not applicable or not tested.
    Skip,
}

/// One line of test results.
#[derive(Debug, Clone, PartialEq)]
pub struct TestLine {
    pub name: String,
    pub verdict: Verdict,
    pub detail: String,
}

fn line(name: impl Into<String>, verdict: Verdict, detail: impl Into<String>) -> TestLine {
    TestLine {
        name: name.into(),
        verdict,
        detail: detail.into(),
    }
}

/// Did a local DNS server answer a query?
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DnsProbe {
    Answered,
    NoAnswer,
    /// A public server: not contacted by the local tests.
    NotLocal,
}

/// Raw results of the local probes for one adapter.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LocalProbes {
    pub gateways: Vec<(IpAddr, PingStats)>,
    pub dns: Vec<(IpAddr, DnsProbe)>,
}

/// Pings sent to each gateway in the local tests.
pub const LOCAL_PINGS: u32 = 4;

/// Run the local tests for one adapter (blocks for a few seconds). Only the adapter's gateway
/// and DNS servers on the local network are contacted.
pub fn probe_local(a: &Adapter) -> LocalProbes {
    let mut p = LocalProbes::default();
    if a.status != Status::Up {
        return p;
    }
    // A public gateway (e.g. a cellular modem) is not part of the local network: not pinged.
    for &gw in a.gateways.iter().filter(|&&g| is_local(g)) {
        p.gateways
            .push((gw, imp_probe::ping_series(gw, a.if_index, LOCAL_PINGS, 250)));
    }
    for &d in &a.dns {
        let r = if !is_local(d) {
            DnsProbe::NotLocal
        } else if imp_probe::dns_server_answers(d, a.if_index) {
            DnsProbe::Answered
        } else {
            DnsProbe::NoAnswer
        };
        p.dns.push((d, r));
    }
    p
}

/// Judge one adapter from its details and the local probes.
pub fn local_results(a: &Adapter, p: &LocalProbes) -> Vec<TestLine> {
    let mut out = Vec::new();
    match a.status {
        Status::Up => out.push(line("Link", Verdict::Pass, a.status.name())),
        s => {
            let why = match (s, a.media) {
                (Status::Disconnected, Media::Ethernet) => {
                    "Disconnected: check the cable is plugged in at both ends"
                }
                (Status::Disconnected, Media::WiFi) => "Not connected to a Wi-Fi network",
                (Status::Disabled, _) => "The adapter is disabled",
                _ => "Not connected",
            };
            out.push(line("Link", Verdict::Fail, why));
            return out;
        }
    }
    let v4: Vec<String> = a.ipv4.iter().map(IpEntry::text).collect();
    if a.has_usable_ipv4() {
        out.push(line("IP address", Verdict::Pass, v4.join(", ")));
    } else if a.apipa_only() {
        out.push(line(
            "IP address",
            Verdict::Fail,
            format!(
                "{} is self-assigned (169.254.x.x): no DHCP server answered, so there's no working connection",
                v4.join(", ")
            ),
        ));
    } else if a.ipv6.iter().any(|e| !e.addr.is_loopback()) {
        out.push(line(
            "IP address",
            Verdict::Warn,
            "No IPv4 address (IPv6 only)",
        ));
    } else {
        out.push(line("IP address", Verdict::Fail, "No IP address"));
    }
    let dups: Vec<String> = a
        .ipv4
        .iter()
        .chain(&a.ipv6)
        .filter(|e| e.duplicate)
        .map(|e| e.addr.to_string())
        .collect();
    if dups.is_empty() {
        out.push(line("Duplicate address", Verdict::Pass, "None detected"));
    } else {
        out.push(line(
            "Duplicate address",
            Verdict::Fail,
            format!(
                "{} is also used by another device on the network",
                dups.join(", ")
            ),
        ));
    }
    for gw in a.gateways.iter().filter(|&&g| !is_local(g)) {
        out.push(line(
            "Gateway reachable",
            Verdict::Skip,
            format!("{gw}: not on your local network, so not contacted by the local tests"),
        ));
    }
    if p.gateways.is_empty() && a.gateways.is_empty() {
        out.push(line(
            "Gateway",
            Verdict::Warn,
            "No default gateway: this adapter can only reach the local network",
        ));
    }
    for (gw, s) in &p.gateways {
        let v = if s.received() == 0 {
            Verdict::Fail
        } else if s.received() < s.sent {
            Verdict::Warn
        } else {
            Verdict::Pass
        };
        let detail = if s.received() == 0 {
            format!("{gw}: no replies (the router is unreachable, or ignores ping)")
        } else {
            format!("{gw}: {}", s.summary())
        };
        out.push(line("Gateway reachable", v, detail));
    }
    if p.dns.is_empty() {
        out.push(line(
            "DNS server",
            Verdict::Fail,
            "No DNS servers set: websites won't load by name",
        ));
    }
    for (d, r) in &p.dns {
        let (v, detail) = match r {
            DnsProbe::Answered => (Verdict::Pass, format!("{d} answers")),
            DnsProbe::NoAnswer => (Verdict::Fail, format!("{d} doesn't answer DNS queries")),
            DnsProbe::NotLocal => (
                Verdict::Skip,
                format!("{d} is on the internet; checked by the internet tests"),
            ),
        };
        out.push(line("DNS server reachable", v, detail));
    }
    match (a.below_max_speed(), a.speed_bps, a.media) {
        (Some((cur, max)), _, _) => out.push(line(
            "Link speed",
            Verdict::Warn,
            format!(
                "{} although the adapter supports {}: fine if the router or switch port is only {0}, otherwise check the cable",
                format_link_speed(cur),
                format_link_speed(max)
            ),
        )),
        (None, Some(s), Media::Ethernet) if s <= 100_000_000 => out.push(line(
            "Link speed",
            Verdict::Warn,
            format!("Only {}: usually a damaged or 4-wire cable", format_link_speed(s)),
        )),
        (None, Some(_), _) => out.push(line("Link speed", Verdict::Pass, a.link_text())),
        _ => {}
    }
    out
}

/// What the internet tests contact. Shown next to their button.
pub const INTERNET_TARGETS: &str = "DNS lookup of dns.msftncsi.com, an HTTP request to www.msftconnecttest.com/connecttest.txt (Windows' own connectivity probes) and pings to the public DNS resolvers 1.1.1.1 (Cloudflare) and 8.8.8.8 (Google)";
const NCSI_DNS_NAME: &str = "dns.msftncsi.com";
const NCSI_DNS_ADDR: Ipv4Addr = Ipv4Addr::new(131, 107, 255, 255);
const NCSI_HTTP_HOST: &str = "www.msftconnecttest.com";
const NCSI_HTTP_PATH: &str = "/connecttest.txt";
const NCSI_HTTP_BODY: &str = "Microsoft Connect Test";
/// Public resolvers pinged for latency, jitter and loss.
pub const PING_TARGETS: [(Ipv4Addr, &str); 2] = [
    (Ipv4Addr::new(1, 1, 1, 1), "Cloudflare"),
    (Ipv4Addr::new(8, 8, 8, 8), "Google"),
];
/// Pings sent to each public resolver.
pub const INTERNET_PINGS: u32 = 20;

/// Raw results of the internet tests.
#[derive(Debug, Clone, PartialEq)]
pub struct InternetProbes {
    pub dns: Result<Vec<Ipv4Addr>, String>,
    pub http: Result<(u16, String), String>,
    pub pings: Vec<(Ipv4Addr, &'static str, PingStats)>,
}

/// Run the internet tests (blocks for several seconds). Only call on a user's click.
pub fn probe_internet() -> InternetProbes {
    std::thread::scope(|s| {
        let pings: Vec<_> = PING_TARGETS
            .iter()
            .map(|&(ip, who)| {
                s.spawn(move || {
                    (
                        ip,
                        who,
                        imp_probe::ping_series(IpAddr::V4(ip), 0, INTERNET_PINGS, 200),
                    )
                })
            })
            .collect();
        let dns = imp_probe::resolve_a(NCSI_DNS_NAME);
        let http = imp_probe::http_get(NCSI_HTTP_HOST, NCSI_HTTP_PATH);
        InternetProbes {
            dns,
            http,
            pings: pings.into_iter().filter_map(|h| h.join().ok()).collect(),
        }
    })
}

/// Judge the internet probes.
pub fn internet_results(p: &InternetProbes) -> Vec<TestLine> {
    let mut out = Vec::new();
    out.push(match &p.dns {
        Ok(addrs) if addrs.contains(&NCSI_DNS_ADDR) => line(
            "DNS lookup",
            Verdict::Pass,
            format!("{NCSI_DNS_NAME} resolved to {NCSI_DNS_ADDR}"),
        ),
        Ok(addrs) => line(
            "DNS lookup",
            Verdict::Warn,
            format!(
                "{NCSI_DNS_NAME} resolved to {} instead of {NCSI_DNS_ADDR}: a captive portal, filter or DNS hijacking",
                addrs
                    .iter()
                    .map(|a| a.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ),
        Err(e) => line("DNS lookup", Verdict::Fail, format!("{NCSI_DNS_NAME} failed: {e}")),
    });
    out.push(match &p.http {
        Ok((200, body)) if body.trim() == NCSI_HTTP_BODY => {
            line("Web (HTTP)", Verdict::Pass, format!("{NCSI_HTTP_HOST} answered as expected"))
        }
        Ok((code, _)) => line(
            "Web (HTTP)",
            Verdict::Warn,
            format!("{NCSI_HTTP_HOST} gave an unexpected answer (HTTP {code}): a captive portal (hotel or airport Wi-Fi sign-in) or a proxy is in the way"),
        ),
        Err(e) => line("Web (HTTP)", Verdict::Fail, format!("{NCSI_HTTP_HOST} failed: {e}")),
    });
    for (ip, who, s) in &p.pings {
        let v = match (s.received(), s.loss_pct(), s.avg_ms(), s.jitter_ms()) {
            (0, ..) => Verdict::Fail,
            (_, loss, ..) if loss >= 5.0 => Verdict::Warn,
            (_, _, Some(avg), _) if avg >= 100.0 => Verdict::Warn,
            (_, _, _, Some(j)) if j >= 30.0 => Verdict::Warn,
            _ => Verdict::Pass,
        };
        out.push(line(format!("Ping {ip} ({who})"), v, s.summary()));
    }
    out
}

mod imp_probe {
    use super::PingStats;
    use std::net::{IpAddr, Ipv4Addr};
    pub fn ping_series(_ip: IpAddr, _scope: u32, count: u32, _gap_ms: u64) -> PingStats {
        PingStats {
            sent: count,
            rtts: Vec::new(),
        }
    }
    pub fn dns_server_answers(_server: IpAddr, _scope: u32) -> bool {
        false
    }
    pub fn resolve_a(_name: &str) -> Result<Vec<Ipv4Addr>, String> {
        Err("Only supported on Windows".into())
    }
    pub fn http_get(_host: &str, _path: &str) -> Result<(u16, String), String> {
        Err("Only supported on Windows".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v4(s: &str) -> Ipv4Addr {
        s.parse().unwrap()
    }

    fn entry(s: &str) -> IpEntry {
        IpEntry {
            addr: s.parse().unwrap(),
            prefix: 24,
            duplicate: false,
            dhcp: true,
            lease_secs: None,
        }
    }

    const GUID: &str = "{4D36E972-E325-11CE-BFC1-08002BE10318}";

    fn ethernet() -> Adapter {
        Adapter {
            guid: GUID.into(),
            if_index: 7,
            name: "Ethernet".into(),
            media: Media::Ethernet,
            physical: true,
            status: Status::Up,
            speed_bps: Some(1_000_000_000),
            ipv4: vec![entry("192.168.1.20")],
            gateways: vec!["192.168.1.1".parse().unwrap()],
            dns: vec!["192.168.1.1".parse().unwrap()],
            ..Default::default()
        }
    }

    #[test]
    fn apipa_and_local() {
        assert!(is_apipa("169.254.10.2".parse().unwrap()));
        assert!(!is_apipa("192.168.1.2".parse().unwrap()));
        assert!(!is_apipa("fe80::1".parse().unwrap()));
        assert!(is_local("10.0.0.1".parse().unwrap()));
        assert!(is_local("fe80::1".parse().unwrap()));
        assert!(is_local("fd00::1".parse().unwrap()));
        assert!(!is_local("1.1.1.1".parse().unwrap()));
        assert!(!is_local("2606:4700:4700::1111".parse().unwrap()));
        let mut a = ethernet();
        assert!(!a.apipa_only() && a.has_usable_ipv4());
        a.ipv4 = vec![entry("169.254.3.4")];
        assert!(a.apipa_only() && !a.has_usable_ipv4());
    }

    #[test]
    fn macs_and_leases() {
        assert_eq!(
            format_mac(&[0xd8, 0xbb, 0xc1, 0, 1, 0xff]),
            "D8-BB-C1-00-01-FF"
        );
        assert_eq!(normalize_mac("D8BBC10001FF"), "D8-BB-C1-00-01-FF");
        assert_eq!(normalize_mac("d8-bb-c1-00-01-ff"), "D8-BB-C1-00-01-FF");
        assert_eq!(normalize_mac(""), "");
        assert_eq!(format_lease(3 * 3600 + 60), "expires in 3h 1m");
        assert_eq!(format_lease(u32::MAX), "never expires");
    }

    #[test]
    fn wifi_bands_and_channels() {
        assert_eq!(channel_from_mhz(2412), Some(1));
        assert_eq!(channel_from_mhz(2437), Some(6));
        assert_eq!(channel_from_mhz(2484), Some(14));
        assert_eq!(channel_from_mhz(5180), Some(36));
        assert_eq!(channel_from_mhz(5745), Some(149));
        assert_eq!(channel_from_mhz(5975), Some(5));
        assert_eq!(Band::from_mhz(2437), Some(Band::G2_4));
        assert_eq!(Band::from_mhz(5180), Some(Band::G5));
        assert_eq!(Band::from_mhz(5975), Some(Band::G6));
        assert_eq!(Band::from_mhz(60000), None);
        assert_eq!(Band::from_channel(11), Some(Band::G2_4));
        assert_eq!(Band::from_channel(149), Some(Band::G5));
        assert_eq!(phy_name(10), Some("Wi-Fi 6 (802.11ax)"));
        assert_eq!(phy_name(99), None);
        assert_eq!(Media::from_if_type(71), Media::WiFi);
        assert_eq!(Media::from_physical_medium(14), Some(Media::Ethernet));
    }

    #[test]
    fn speeds_from_driver_text() {
        assert_eq!(
            speed_from_display("2.5 Gbps Full Duplex"),
            Some(2_500_000_000)
        );
        assert_eq!(
            speed_from_display("1.0 Gbps Full Duplex"),
            Some(1_000_000_000)
        );
        assert_eq!(
            speed_from_display("100 Mbps Half Duplex"),
            Some(100_000_000)
        );
        assert_eq!(speed_from_display("10Mb Full"), Some(10_000_000));
        assert_eq!(speed_from_display("Auto Negotiation"), None);
        let p = AdvancedProperty {
            keyword: "*SpeedDuplex".into(),
            choices: vec![
                ("0".into(), "Auto Negotiation".into()),
                ("4".into(), "100 Mbps Full Duplex".into()),
                ("6".into(), "1.0 Gbps Full Duplex".into()),
                ("9".into(), "2.5 Gbps Full Duplex".into()),
            ],
            ..Default::default()
        };
        assert_eq!(max_speed(&p), Some(2_500_000_000));
        let mut a = ethernet();
        a.max_speed_bps = Some(2_500_000_000);
        assert_eq!(a.below_max_speed(), Some((1_000_000_000, 2_500_000_000)));
        a.media = Media::WiFi;
        assert_eq!(a.below_max_speed(), None);
        assert_eq!(ethernet().link_text(), "1 Gbps");
        a.duplex = Some(Duplex::Full);
        assert_eq!(a.link_text(), "1 Gbps, full duplex");
    }

    #[test]
    fn prefixes_and_lists() {
        assert_eq!(parse_prefix("24"), Some(24));
        assert_eq!(parse_prefix("/16"), Some(16));
        assert_eq!(parse_prefix("255.255.255.0"), Some(24));
        assert_eq!(parse_prefix("255.255.252.0"), Some(22));
        assert_eq!(parse_prefix("255.0.255.0"), None);
        assert_eq!(parse_prefix("0"), None);
        assert_eq!(parse_prefix("33"), None);
        assert_eq!(prefix_mask(24), v4("255.255.255.0"));
        assert_eq!(prefix_mask(0), v4("0.0.0.0"));
        assert_eq!(
            parse_ipv4_list("1.1.1.1, 8.8.8.8;9.9.9.9"),
            Ok(vec![v4("1.1.1.1"), v4("8.8.8.8"), v4("9.9.9.9")])
        );
        assert_eq!(parse_ipv4_list(""), Ok(vec![]));
        assert!(parse_ipv4_list("1.1.1.1, nope").is_err());
    }

    #[test]
    fn adapter_names() {
        assert!(valid_adapter_name("Ethernet 2").is_ok());
        assert!(valid_adapter_name("").is_err());
        assert!(valid_adapter_name(" LAN").is_err());
        assert!(valid_adapter_name("a/b").is_err());
        assert!(valid_adapter_name("a\"b").is_err());
        assert!(valid_adapter_name("x\ny").is_err());
        assert!(valid_adapter_name(&"x".repeat(65)).is_err());
    }

    fn all_actions() -> Vec<NetAction> {
        let n = || "Ethernet".to_string();
        vec![
            NetAction::Enable {
                guid: GUID.into(),
                name: n(),
            },
            NetAction::Disable {
                guid: GUID.into(),
                name: n(),
            },
            NetAction::Rename {
                guid: GUID.into(),
                name: n(),
                new_name: "LAN – desk".into(),
            },
            NetAction::Dhcp {
                if_index: 7,
                name: n(),
            },
            NetAction::StaticIp {
                if_index: 7,
                name: n(),
                ip: v4("192.168.1.50"),
                prefix: 24,
                gateway: Some(v4("192.168.1.1")),
            },
            NetAction::StaticIp {
                if_index: 7,
                name: n(),
                ip: v4("10.0.0.5"),
                prefix: 8,
                gateway: None,
            },
            NetAction::Dns {
                if_index: 7,
                name: n(),
                servers: vec![v4("1.1.1.1"), v4("9.9.9.9")],
            },
            NetAction::Dns {
                if_index: 7,
                name: n(),
                servers: vec![],
            },
            NetAction::Mtu {
                if_index: 7,
                name: n(),
                mtu: 1500,
            },
            NetAction::Release {
                if_index: 7,
                name: n(),
            },
            NetAction::Renew {
                if_index: 7,
                name: n(),
            },
            NetAction::Advanced {
                guid: GUID.into(),
                name: n(),
                keyword: "*SpeedDuplex".into(),
                value: "0".into(),
                display_name: "Speed & Duplex".into(),
                display_value: "Auto Negotiation".into(),
            },
            NetAction::FlushDns,
            NetAction::ResetWinsock,
            NetAction::ResetTcpIp,
        ]
    }

    #[test]
    fn actions_round_trip() {
        for a in all_actions() {
            assert_eq!(a.validate(), Ok(()), "{a:?}");
            let arg = a.to_arg();
            assert!(arg.chars().all(|c| c.is_ascii_hexdigit()));
            assert_eq!(NetAction::from_arg(&arg), Some(a.clone()), "{a:?}");
            assert!(!a.label().is_empty() && !a.confirm_text().is_empty());
        }
        assert!(!NetAction::FlushDns.needs_admin());
        assert!(NetAction::ResetWinsock.needs_admin() && NetAction::ResetWinsock.needs_restart());
        assert!(
            !NetAction::Dhcp {
                if_index: 1,
                name: String::new()
            }
            .needs_restart()
        );
    }

    fn hex(s: &str) -> String {
        s.bytes().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn helper_rejects_bad_input() {
        for bad in [
            "",
            "zz",
            "abc",
            &hex("format\tC:"),
            &hex("enable\tnot-a-guid\tx"),
            &hex("enable\t{4D36E972-E325-11CE-BFC1-08002BE1031}\tx"),
            &hex(&format!("rename\t{GUID}\tx\tbad/name")),
            &hex("dhcp\t0\tx"),
            &hex("dhcp\t-1\tx"),
            &hex("static\t7\tx\t127.0.0.1\t8\t"),
            &hex("static\t7\tx\t192.168.1.0\t24\t"),
            &hex("static\t7\tx\t192.168.1.255\t24\t"),
            &hex("static\t7\tx\t192.168.1.5\t24\t10.0.0.1"),
            &hex("static\t7\tx\t192.168.1.5\t24\t192.168.1.5"),
            &hex("static\t7\tx\t192.168.1.5\t31\t"),
            &hex("dns\t7\tx\t1.1.1.1,0.0.0.0"),
            &hex("dns\t7\tx\t1.1.1.1,1.0.0.1,8.8.8.8,8.8.4.4,9.9.9.9"),
            &hex("dns\t7\tx\t1.1.1.1;rm -rf"),
            &hex("mtu\t7\tx\t100"),
            &hex("mtu\t7\tx\t65000"),
            &hex(&format!("advanced\t{GUID}\tx\t*Speed Duplex\t0")),
            &hex(&format!("advanced\t{GUID}\tx\t*SpeedDuplex\t")),
            &hex(&format!("advanced\t{GUID}\tx\t*SpeedDuplex\ta\"b\nc")),
            &hex("flushdns\textra"),
        ] {
            assert_eq!(NetAction::from_arg(bad), None, "accepted {bad}");
        }
    }

    fn stats(sent: u32, rtts: &[u32]) -> PingStats {
        PingStats {
            sent,
            rtts: rtts.to_vec(),
        }
    }

    #[test]
    fn ping_statistics() {
        let s = stats(4, &[10, 20, 10, 20]);
        assert_eq!(s.loss_pct(), 0.0);
        assert_eq!(s.avg_ms(), Some(15.0));
        assert_eq!(s.jitter_ms(), Some(10.0));
        assert_eq!((s.min_ms(), s.max_ms()), (Some(10), Some(20)));
        assert_eq!(
            s.summary(),
            "15.0 ms avg (10-20), jitter 10.0 ms, 0 % loss (4/4 replies)"
        );
        let s = stats(4, &[5]);
        assert_eq!(s.loss_pct(), 75.0);
        assert_eq!(s.jitter_ms(), None);
        assert_eq!(stats(3, &[]).summary(), "no replies (3 sent)");
        assert_eq!(stats(0, &[]).loss_pct(), 0.0);
    }

    fn verdict(lines: &[TestLine], name: &str) -> Vec<Verdict> {
        lines
            .iter()
            .filter(|l| l.name == name)
            .map(|l| l.verdict)
            .collect()
    }

    #[test]
    fn local_test_verdicts() {
        let a = ethernet();
        let gw: IpAddr = "192.168.1.1".parse().unwrap();
        let good = LocalProbes {
            gateways: vec![(gw, stats(4, &[1, 1, 2, 1]))],
            dns: vec![(gw, DnsProbe::Answered)],
        };
        let r = local_results(&a, &good);
        assert!(r.iter().all(|l| l.verdict == Verdict::Pass), "{r:?}");

        let bad = LocalProbes {
            gateways: vec![(gw, stats(4, &[]))],
            dns: vec![
                (gw, DnsProbe::NoAnswer),
                ("8.8.8.8".parse().unwrap(), DnsProbe::NotLocal),
            ],
        };
        let r = local_results(&a, &bad);
        assert_eq!(verdict(&r, "Gateway reachable"), [Verdict::Fail]);
        assert_eq!(
            verdict(&r, "DNS server reachable"),
            [Verdict::Fail, Verdict::Skip]
        );

        let mut apipa = ethernet();
        apipa.ipv4 = vec![entry("169.254.1.2")];
        apipa.ipv4[0].duplicate = true;
        apipa.max_speed_bps = Some(2_500_000_000);
        apipa.gateways.clear();
        let r = local_results(&apipa, &LocalProbes::default());
        assert_eq!(verdict(&r, "IP address"), [Verdict::Fail]);
        assert_eq!(verdict(&r, "Duplicate address"), [Verdict::Fail]);
        assert_eq!(verdict(&r, "Gateway"), [Verdict::Warn]);
        assert_eq!(verdict(&r, "DNS server"), [Verdict::Fail]);
        assert_eq!(verdict(&r, "Link speed"), [Verdict::Warn]);

        let mut down = ethernet();
        down.status = Status::Disconnected;
        let r = local_results(&down, &LocalProbes::default());
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].verdict, Verdict::Fail);
        assert_eq!(probe_local(&down), LocalProbes::default());
    }

    #[test]
    fn public_gateway_is_not_contacted() {
        let mut a = ethernet();
        a.gateways = vec!["8.8.4.4".parse().unwrap()];
        // Nothing is pinged for it, and the report says so instead of warning about no gateway.
        let r = local_results(&a, &LocalProbes::default());
        assert_eq!(verdict(&r, "Gateway reachable"), [Verdict::Skip]);
        assert!(verdict(&r, "Gateway").is_empty());
    }

    #[test]
    fn internet_test_verdicts() {
        let good = InternetProbes {
            dns: Ok(vec![NCSI_DNS_ADDR]),
            http: Ok((200, "Microsoft Connect Test".into())),
            pings: vec![(v4("1.1.1.1"), "Cloudflare", stats(4, &[12, 13, 12, 14]))],
        };
        assert!(
            internet_results(&good)
                .iter()
                .all(|l| l.verdict == Verdict::Pass)
        );
        let portal = InternetProbes {
            dns: Ok(vec![v4("10.0.0.1")]),
            http: Ok((302, String::new())),
            pings: vec![(v4("8.8.8.8"), "Google", stats(20, &[300; 15]))],
        };
        let r = internet_results(&portal);
        assert!(r.iter().all(|l| l.verdict == Verdict::Warn), "{r:?}");
        let offline = InternetProbes {
            dns: Err("timed out".into()),
            http: Err("no connection".into()),
            pings: vec![(v4("8.8.8.8"), "Google", stats(20, &[]))],
        };
        assert!(
            internet_results(&offline)
                .iter()
                .all(|l| l.verdict == Verdict::Fail)
        );
    }
}
