//! Network adapters: the data model behind the Network page, the changes it can make (encoded
//! for the elevated helper and validated again there), and the logic of the local and internet
//! tests. Adapters are read in [`crate::linux::netinfo`] and the tests send their packets from
//! [`crate::linux::netprobe`]; changes aren't implemented on Linux yet.
//!
//! Nothing in this module talks to the network by itself. The local tests only contact the
//! adapter's own gateway and DNS servers on the local network; the internet tests (see
//! [`INTERNET_TARGETS`]) run only when the user clicks their button.

use crate::checkup::format_link_speed;
use std::net::{IpAddr, Ipv4Addr};

/// Every network interface except loopback, plus how much detail was read.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NetworkInfo {
    pub adapters: Vec<Adapter>,
    /// The slower details (the DHCP server) were read.
    pub detailed: bool,
}

impl NetworkInfo {
    /// Read the adapters. `detailed` adds the slower details (the DHCP server); otherwise
    /// those are carried over from `previous`.
    pub fn gather(detailed: bool, previous: Option<&NetworkInfo>) -> NetworkInfo {
        crate::linux::netinfo::gather(detailed, previous)
    }

    /// Adapters that are up with an address, i.e. carrying traffic.
    pub fn connected(&self) -> impl Iterator<Item = &Adapter> {
        self.adapters.iter().filter(|a| a.status == Status::Up)
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
}

/// Interface state.
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
    /// Network name.
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

/// The adapter's kernel driver.
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
    /// Interface index.
    pub if_index: u32,
    /// Interface name, e.g. "enp5s0".
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
    /// Fastest speed the adapter supports (ethtool link modes).
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

    /// An Ethernet adapter linked slower than it supports.
    pub fn below_max_speed(&self) -> Option<(u64, u64)> {
        let (cur, max) = (self.speed_bps?, self.max_speed_bps?);
        (self.media == Media::Ethernet && self.status == Status::Up && cur < max)
            .then_some((cur, max))
    }
}

/// 169.254.0.0/16, the self-assigned address used when no DHCP server answers.
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

/// An interface name the kernel accepts (at most 15 bytes, no `/` or whitespace) that can't be
/// mistaken for a command-line option.
pub fn valid_interface(n: &str) -> Result<(), String> {
    let ok = !n.is_empty()
        && n.len() <= 15
        && !n.starts_with('-')
        && n != "."
        && n != ".."
        && n.chars().all(|c| c.is_ascii_graphic() && c != '/');
    if ok {
        Ok(())
    } else {
        Err("Unknown network interface".into())
    }
}

/// Lowest and highest MTU that may be set.
pub const MTU_RANGE: std::ops::RangeInclusive<u32> = 576..=9216;
/// Most DNS servers that may be set at once.
pub const MAX_DNS: usize = 4;

/// A change to a network interface, made through NetworkManager after the user confirms it.
/// `name` is the interface name ("enp5s0").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetAction {
    /// Bring the interface up and activate its connection.
    Connect {
        name: String,
    },
    /// Deactivate the interface's connection.
    Disconnect {
        name: String,
    },
    /// Get the IPv4 address and DNS servers automatically (DHCP).
    Dhcp {
        name: String,
    },
    /// A fixed IPv4 address.
    StaticIp {
        name: String,
        ip: Ipv4Addr,
        prefix: u8,
        gateway: Option<Ipv4Addr>,
    },
    /// IPv4 DNS servers; empty means "automatic" (from DHCP).
    Dns {
        name: String,
        servers: Vec<Ipv4Addr>,
    },
    Mtu {
        name: String,
        mtu: u32,
    },
    /// Activate the connection again, which asks the DHCP server for a new lease.
    Reconnect {
        name: String,
    },
    /// Clear systemd-resolved's cache.
    FlushDns,
}

impl NetAction {
    /// Short description for buttons and the status bar.
    pub fn label(&self) -> String {
        match self {
            NetAction::Connect { name } => format!("Connect {name}"),
            NetAction::Disconnect { name } => format!("Disconnect {name}"),
            NetAction::Dhcp { name } => format!("Get an IP address automatically on {name}"),
            NetAction::StaticIp {
                name,
                ip,
                prefix,
                gateway,
            } => format!(
                "Set {name} to {ip}/{prefix} (mask {}, {})",
                prefix_mask(*prefix),
                match gateway {
                    Some(g) => format!("gateway {g}"),
                    None => "no gateway".into(),
                }
            ),
            NetAction::Dns { name, servers } if servers.is_empty() => {
                format!("Get DNS servers automatically on {name}")
            }
            NetAction::Dns { name, servers } => format!(
                "Set DNS on {name} to {}",
                servers
                    .iter()
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            NetAction::Mtu { name, mtu } => format!("Set MTU of {name} to {mtu}"),
            NetAction::Reconnect { name } => format!("Reconnect {name}"),
            NetAction::FlushDns => "Flush the DNS cache".into(),
        }
    }

    /// What the confirmation dialog says before applying.
    pub fn confirm_text(&self) -> &'static str {
        match self {
            NetAction::Connect { .. } => "NetworkManager brings the interface up and connects it.",
            NetAction::Disconnect { .. } => {
                "The connection drops, and NetworkManager doesn't reconnect it by itself until you click Connect (here or in the system menu)."
            }
            NetAction::Dhcp { .. } => {
                "The connection asks the router for its IPv4 address and DNS servers. It may drop for a few seconds."
            }
            NetAction::StaticIp { .. } => {
                "The connection stops using DHCP and keeps this IPv4 address. A wrong address, mask or gateway cuts it off until you switch back to automatic."
            }
            NetAction::Dns { .. } => {
                "Name lookups on this connection go to these DNS servers. Wrong servers make websites fail to load."
            }
            NetAction::Mtu { .. } => {
                "A wrong MTU can make some websites or games hang. The default for Ethernet and Wi-Fi is 1500."
            }
            NetAction::Reconnect { .. } => {
                "The connection is activated again and asks the DHCP server for an address. It drops for a few seconds."
            }
            NetAction::FlushDns => {
                "systemd-resolved forgets the names it has looked up and looks them up again."
            }
        }
    }

    /// The interface the action changes.
    pub fn interface(&self) -> Option<&str> {
        match self {
            NetAction::Connect { name }
            | NetAction::Disconnect { name }
            | NetAction::Dhcp { name }
            | NetAction::StaticIp { name, .. }
            | NetAction::Dns { name, .. }
            | NetAction::Mtu { name, .. }
            | NetAction::Reconnect { name } => Some(name),
            NetAction::FlushDns => None,
        }
    }

    /// Check the action's values; `Err` explains what's wrong.
    pub fn validate(&self) -> Result<(), String> {
        if let Some(n) = self.interface() {
            valid_interface(n)?;
        }
        match self {
            NetAction::StaticIp {
                ip,
                prefix,
                gateway,
                ..
            } => {
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
            NetAction::Dns { servers, .. } => {
                if servers.len() > MAX_DNS {
                    return Err(format!("At most {MAX_DNS} DNS servers"));
                }
                match servers.iter().find(|s| !usable_host(**s)) {
                    Some(s) => Err(format!("{s} can't be a DNS server")),
                    None => Ok(()),
                }
            }
            NetAction::Mtu { mtu, .. } => {
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
            _ => Ok(()),
        }
    }

    /// The `nmcli connection modify` settings this action changes on connection type
    /// `conn_type` ("802-3-ethernet"), or `None` for actions that don't modify a connection.
    pub fn nm_settings(&self, conn_type: &str) -> Option<Result<Vec<String>, String>> {
        let s = |v: &[&str]| Some(Ok(v.iter().map(|x| x.to_string()).collect()));
        match self {
            NetAction::Dhcp { .. } => s(&[
                "ipv4.method",
                "auto",
                "ipv4.addresses",
                "",
                "ipv4.gateway",
                "",
            ]),
            NetAction::StaticIp {
                ip,
                prefix,
                gateway,
                ..
            } => s(&[
                "ipv4.method",
                "manual",
                "ipv4.addresses",
                &format!("{ip}/{prefix}"),
                "ipv4.gateway",
                &gateway.map(|g| g.to_string()).unwrap_or_default(),
            ]),
            NetAction::Dns { servers, .. } if servers.is_empty() => {
                s(&["ipv4.dns", "", "ipv4.ignore-auto-dns", "no"])
            }
            NetAction::Dns { servers, .. } => s(&[
                "ipv4.dns",
                &servers
                    .iter()
                    .map(|a| a.to_string())
                    .collect::<Vec<_>>()
                    .join(","),
                "ipv4.ignore-auto-dns",
                "yes",
            ]),
            NetAction::Mtu { mtu, .. } => match conn_type {
                "802-3-ethernet" | "802-11-wireless" => {
                    s(&[&format!("{conn_type}.mtu"), &mtu.to_string()])
                }
                t => Some(Err(format!(
                    "The MTU of a {} connection can't be set here",
                    if t.is_empty() { "unknown" } else { t }
                ))),
            },
            _ => None,
        }
    }
}

/// Run a command; `Err` carries what it printed on failure.
fn run(cmd: &str, args: &[&str]) -> Result<String, String> {
    let out = std::process::Command::new(cmd)
        .args(args)
        .env("LC_ALL", "C")
        .output()
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => format!("{cmd} isn't installed"),
            _ => e.to_string(),
        })?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).trim().to_string());
    }
    let err = String::from_utf8_lossy(&out.stderr);
    let err = err.trim().trim_start_matches("Error: ");
    Err(if err.is_empty() {
        format!("{cmd} failed")
    } else {
        err.to_string()
    })
}

/// Apply a change through NetworkManager (`nmcli`), which lets the signed-in desktop user
/// change network settings without root. Blocks.
pub fn apply(action: &NetAction) -> Result<(), String> {
    action.validate()?;
    let iface = match action {
        NetAction::FlushDns => {
            return run("resolvectl", &["flush-caches"]).map(drop).map_err(|e| {
                if e.contains("isn't installed") || e.contains("resolve") {
                    "This system doesn't run systemd-resolved, so there's no DNS cache to flush"
                        .into()
                } else {
                    e
                }
            });
        }
        NetAction::Connect { name } => return run("nmcli", &["device", "connect", name]).map(drop),
        NetAction::Disconnect { name } => {
            return run("nmcli", &["device", "disconnect", name]).map(drop);
        }
        a => a.interface().unwrap_or_default(),
    };
    let conn = run(
        "nmcli",
        &["-g", "GENERAL.CONNECTION", "device", "show", iface],
    )?;
    if conn.is_empty() {
        return Err(format!(
            "{iface} has no active NetworkManager connection to change"
        ));
    }
    if let NetAction::Reconnect { .. } = action {
        return run("nmcli", &["connection", "up", "id", &conn]).map(drop);
    }
    let conn_type = run(
        "nmcli",
        &["-g", "connection.type", "connection", "show", "id", &conn],
    )?;
    let Some(settings) = action.nm_settings(&conn_type) else {
        return Ok(());
    };
    let settings = settings?;
    let mut args = vec!["connection", "modify", "id", conn.as_str()];
    args.extend(settings.iter().map(String::as_str));
    run("nmcli", &args)?;
    // Apply to the running connection without a full reconnect where NetworkManager can.
    run("nmcli", &["device", "reapply", iface])
        .or_else(|_| run("nmcli", &["connection", "up", "id", &conn]))
        .map(drop)
}

// ---- Tests (ping, DNS, HTTP) -------------------------------------------------------------

/// Round-trip times of a series of pings.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PingStats {
    pub sent: u32,
    /// Round-trip time in ms of each reply, in order (fractional: LAN replies take under 1 ms).
    pub rtts: Vec<f64>,
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
        (!self.rtts.is_empty()).then(|| self.rtts.iter().sum::<f64>() / self.rtts.len() as f64)
    }

    pub fn min_ms(&self) -> Option<f64> {
        self.rtts.iter().copied().reduce(f64::min)
    }

    pub fn max_ms(&self) -> Option<f64> {
        self.rtts.iter().copied().reduce(f64::max)
    }

    /// Jitter: mean difference between consecutive round-trip times (RFC 3550 style, unsmoothed).
    pub fn jitter_ms(&self) -> Option<f64> {
        if self.rtts.len() < 2 {
            return None;
        }
        let sum: f64 = self.rtts.windows(2).map(|w| (w[0] - w[1]).abs()).sum();
        Some(sum / (self.rtts.len() - 1) as f64)
    }

    /// "12.3 ms avg (10.0-18.0), jitter 2.1 ms, 0 % loss" or "no replies".
    pub fn summary(&self) -> String {
        match (self.avg_ms(), self.min_ms(), self.max_ms()) {
            (Some(avg), Some(lo), Some(hi)) => {
                let jitter = self
                    .jitter_ms()
                    .map(|j| format!(", jitter {} ms", ms(j)))
                    .unwrap_or_default();
                format!(
                    "{} ms avg ({}-{}){jitter}, {:.0} % loss ({}/{} replies)",
                    ms(avg),
                    ms(lo),
                    ms(hi),
                    self.loss_pct(),
                    self.received(),
                    self.sent
                )
            }
            _ => format!("no replies ({} sent)", self.sent),
        }
    }
}

/// A round-trip time: two decimals under 1 ms (LAN), one above.
fn ms(v: f64) -> String {
    if v < 1.0 {
        format!("{v:.2}")
    } else {
        format!("{v:.1}")
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
pub const INTERNET_TARGETS: &str = "DNS lookup of dns.msftncsi.com, an HTTP request to www.msftconnecttest.com/connecttest.txt (Microsoft's connectivity-check hosts) and pings to the public DNS resolvers 1.1.1.1 (Cloudflare) and 8.8.8.8 (Google)";
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

use crate::linux::netprobe as imp_probe;

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

    fn ethernet() -> Adapter {
        Adapter {
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
    }

    #[test]
    fn link_speeds() {
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
    fn interface_names() {
        assert!(valid_interface("enp5s0").is_ok());
        assert!(valid_interface("wlp0s20f3").is_ok());
        for bad in ["", "-x", "a/b", "a b", "x\ny", "..", "abcdefghijklmnop"] {
            assert!(valid_interface(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn actions() {
        let n = || "enp5s0".to_string();
        let actions = [
            NetAction::Connect { name: n() },
            NetAction::Disconnect { name: n() },
            NetAction::Dhcp { name: n() },
            NetAction::StaticIp {
                name: n(),
                ip: v4("192.168.1.50"),
                prefix: 24,
                gateway: Some(v4("192.168.1.1")),
            },
            NetAction::Dns {
                name: n(),
                servers: vec![v4("1.1.1.1"), v4("9.9.9.9")],
            },
            NetAction::Dns {
                name: n(),
                servers: vec![],
            },
            NetAction::Mtu {
                name: n(),
                mtu: 1500,
            },
            NetAction::Reconnect { name: n() },
            NetAction::FlushDns,
        ];
        for a in &actions {
            assert_eq!(a.validate(), Ok(()), "{a:?}");
            assert!(!a.label().is_empty() && !a.confirm_text().is_empty());
        }
        let bad = [
            NetAction::Connect {
                name: "--help".into(),
            },
            NetAction::StaticIp {
                name: n(),
                ip: v4("127.0.0.1"),
                prefix: 8,
                gateway: None,
            },
            NetAction::StaticIp {
                name: n(),
                ip: v4("192.168.1.0"),
                prefix: 24,
                gateway: None,
            },
            NetAction::StaticIp {
                name: n(),
                ip: v4("192.168.1.5"),
                prefix: 24,
                gateway: Some(v4("10.0.0.1")),
            },
            NetAction::StaticIp {
                name: n(),
                ip: v4("192.168.1.5"),
                prefix: 31,
                gateway: None,
            },
            NetAction::Dns {
                name: n(),
                servers: vec![v4("1.1.1.1"), v4("0.0.0.0")],
            },
            NetAction::Dns {
                name: n(),
                servers: vec![v4("1.1.1.1"); 5],
            },
            NetAction::Mtu {
                name: n(),
                mtu: 100,
            },
        ];
        for a in &bad {
            assert!(a.validate().is_err(), "accepted {a:?}");
        }

        let settings = |a: &NetAction, t: &str| a.nm_settings(t).map(|r| r.map(|v| v.join(" ")));
        assert_eq!(
            settings(&actions[3], "802-3-ethernet"),
            Some(Ok(
                "ipv4.method manual ipv4.addresses 192.168.1.50/24 ipv4.gateway 192.168.1.1".into()
            ))
        );
        assert_eq!(
            settings(&actions[4], "802-3-ethernet"),
            Some(Ok(
                "ipv4.dns 1.1.1.1,9.9.9.9 ipv4.ignore-auto-dns yes".into()
            ))
        );
        assert_eq!(
            settings(&actions[6], "802-11-wireless"),
            Some(Ok("802-11-wireless.mtu 1500".into()))
        );
        assert!(matches!(settings(&actions[6], "vpn"), Some(Err(_))));
        assert_eq!(settings(&actions[0], "802-3-ethernet"), None);
    }

    fn stats(sent: u32, rtts: &[f64]) -> PingStats {
        PingStats {
            sent,
            rtts: rtts.to_vec(),
        }
    }

    #[test]
    fn ping_statistics() {
        let s = stats(4, &[10.0, 20.0, 10.0, 20.0]);
        assert_eq!(s.loss_pct(), 0.0);
        assert_eq!(s.avg_ms(), Some(15.0));
        assert_eq!(s.jitter_ms(), Some(10.0));
        assert_eq!((s.min_ms(), s.max_ms()), (Some(10.0), Some(20.0)));
        assert_eq!(
            s.summary(),
            "15.0 ms avg (10.0-20.0), jitter 10.0 ms, 0 % loss (4/4 replies)"
        );
        // LAN replies under a millisecond keep two decimals instead of rounding to 0.
        assert_eq!(
            stats(2, &[0.21, 0.35]).summary(),
            "0.28 ms avg (0.21-0.35), jitter 0.14 ms, 0 % loss (2/2 replies)"
        );
        let s = stats(4, &[5.0]);
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
            gateways: vec![(gw, stats(4, &[1.0, 1.0, 2.0, 1.0]))],
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
            pings: vec![(
                v4("1.1.1.1"),
                "Cloudflare",
                stats(4, &[12.0, 13.0, 12.0, 14.0]),
            )],
        };
        assert!(
            internet_results(&good)
                .iter()
                .all(|l| l.verdict == Verdict::Pass)
        );
        let portal = InternetProbes {
            dns: Ok(vec![v4("10.0.0.1")]),
            http: Ok((302, String::new())),
            pings: vec![(v4("8.8.8.8"), "Google", stats(20, &[300.0; 15]))],
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
