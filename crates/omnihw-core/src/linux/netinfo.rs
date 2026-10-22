//! The Network page's adapters: addresses and routes from iproute2 (`ip -j addr`, `ip -j route`),
//! link details from `/sys/class/net`, DNS servers from systemd-resolved (`resolvectl`, or
//! `/etc/resolv.conf`), Wi-Fi from `iw`, the DHCP server from NetworkManager (`nmcli`) or
//! systemd-networkd's lease files, and the fastest supported speed from the ethtool ioctl.

use super::Root;
use super::net::adapter_model;
use crate::network::{
    Adapter, Band, Driver, Duplex, IpEntry, Media, NetworkInfo, Status, Wifi, channel_from_mhz,
    phy_name,
};
use serde_json::Value;
use std::net::IpAddr;
use std::process::Command;

const NET: &str = "/sys/class/net";

/// An interface as `ip -j addr` prints it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Link {
    pub index: u32,
    pub name: String,
    /// Administratively up (`ip link set up`).
    pub up: bool,
    /// Carrier present.
    pub lower_up: bool,
    pub operstate: String,
    pub link_type: String,
    pub mac: String,
    pub mtu: Option<u32>,
    pub ipv4: Vec<IpEntry>,
    pub ipv6: Vec<IpEntry>,
}

/// Interfaces from `ip -j addr`.
pub fn parse_ip_addr(json: &str) -> Vec<Link> {
    let Ok(Value::Array(links)) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    links
        .iter()
        .filter_map(|l| {
            let flags: Vec<&str> = l["flags"]
                .as_array()
                .map(|f| f.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            let mut link = Link {
                index: l["ifindex"].as_u64()? as u32,
                name: l["ifname"].as_str()?.to_string(),
                up: flags.contains(&"UP"),
                lower_up: flags.contains(&"LOWER_UP"),
                operstate: l["operstate"].as_str().unwrap_or_default().to_string(),
                link_type: l["link_type"].as_str().unwrap_or_default().to_string(),
                mac: l["address"]
                    .as_str()
                    .unwrap_or_default()
                    .to_ascii_uppercase()
                    .replace(':', "-"),
                mtu: l["mtu"].as_u64().map(|m| m as u32),
                ..Default::default()
            };
            for a in l["addr_info"].as_array().into_iter().flatten() {
                let Some(addr) = a["local"].as_str().and_then(|s| s.parse::<IpAddr>().ok()) else {
                    continue;
                };
                let dynamic = a["dynamic"].as_bool().unwrap_or(false);
                // u32::MAX is "forever" (a fixed address).
                let lease = a["valid_life_time"]
                    .as_u64()
                    .filter(|&v| dynamic && v < u64::from(u32::MAX))
                    .map(|v| v as u32);
                let entry = IpEntry {
                    addr,
                    prefix: a["prefixlen"].as_u64().unwrap_or(0) as u8,
                    duplicate: a["dadfailed"].as_bool().unwrap_or(false),
                    dhcp: dynamic,
                    lease_secs: lease,
                };
                if addr.is_ipv4() {
                    link.ipv4.push(entry);
                } else {
                    link.ipv6.push(entry);
                }
            }
            Some(link)
        })
        .collect()
}

/// Default gateways from `ip -j route` / `ip -j -6 route`: (interface, gateway).
pub fn parse_default_routes(json: &str) -> Vec<(String, IpAddr)> {
    let Ok(Value::Array(routes)) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    routes
        .iter()
        .filter(|r| r["dst"].as_str() == Some("default"))
        .filter_map(|r| {
            Some((
                r["dev"].as_str()?.to_string(),
                r["gateway"].as_str()?.parse().ok()?,
            ))
        })
        .collect()
}

/// DNS servers and search domains: global and per interface.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DnsConfig {
    pub global: Vec<IpAddr>,
    pub links: Vec<(String, Vec<IpAddr>)>,
    pub domains: Vec<(String, String)>,
}

/// "192.168.1.1", "fe80::1%3" or "9.9.9.9#dns.quad9.net" as an address.
fn dns_addr(s: &str) -> Option<IpAddr> {
    s.split(['%', '#']).next()?.parse().ok()
}

/// `resolvectl dns` / `resolvectl domain` lines: ("Global" or the interface name, values).
fn resolvectl_lines(text: &str) -> Vec<(String, Vec<String>)> {
    text.lines()
        .filter_map(|l| {
            let (head, rest) = l.split_once(':')?;
            let key = match head.trim() {
                "Global" => "Global".to_string(),
                h => h.split_once('(')?.1.trim_end_matches(')').to_string(),
            };
            Some((key, rest.split_whitespace().map(str::to_string).collect()))
        })
        .collect()
}

pub fn parse_resolvectl(dns: &str, domains: &str) -> DnsConfig {
    let mut c = DnsConfig::default();
    for (key, values) in resolvectl_lines(dns) {
        let addrs: Vec<IpAddr> = values.iter().filter_map(|v| dns_addr(v)).collect();
        if key == "Global" {
            c.global = addrs;
        } else if !addrs.is_empty() {
            c.links.push((key, addrs));
        }
    }
    for (key, values) in resolvectl_lines(domains) {
        // "~." and other routing-only domains aren't search suffixes.
        if let Some(d) = values.iter().find(|v| !v.starts_with('~')) {
            c.domains.push((key, d.clone()));
        }
    }
    c
}

/// Name servers in a `resolv.conf`, without systemd-resolved's local stub.
pub fn parse_resolv_conf(text: &str) -> DnsConfig {
    let mut c = DnsConfig::default();
    for l in text.lines() {
        let mut f = l.split_whitespace();
        match f.next() {
            Some("nameserver") => {
                if let Some(a) = f.next().and_then(dns_addr).filter(|a| !a.is_loopback()) {
                    c.global.push(a);
                }
            }
            Some("search") | Some("domain") => {
                if let Some(d) = f.next() {
                    c.domains.push(("Global".into(), d.to_string()));
                }
            }
            _ => {}
        }
    }
    c
}

/// `iw dev <iface> link`; `None` when not associated.
pub fn parse_iw_link(text: &str) -> Option<Wifi> {
    if !text.starts_with("Connected to") {
        return None;
    }
    let field = |k: &str| {
        text.lines()
            .find_map(|l| l.trim().strip_prefix(k).map(str::trim))
    };
    let mbps = |k: &str| -> Option<u32> {
        let v: f64 = field(k)?.split_whitespace().next()?.parse().ok()?;
        Some(v.round() as u32)
    };
    let freq_mhz = field("freq:")
        .and_then(|f| f.parse::<f64>().ok())
        .map(|f| f as u32);
    let rssi_dbm: Option<i32> = field("signal:")
        .and_then(|s| s.split_whitespace().next())
        .and_then(|s| s.parse().ok());
    let rate = field("rx bitrate:").unwrap_or_default();
    let band = freq_mhz.and_then(Band::from_mhz);
    let phy = if rate.contains("EHT-MCS") {
        phy_name(11)
    } else if rate.contains("HE-MCS") {
        phy_name(10)
    } else if rate.contains("VHT-MCS") {
        phy_name(8)
    } else if rate.contains("MCS") {
        phy_name(7)
    } else if band == Some(Band::G2_4) {
        phy_name(6)
    } else {
        phy_name(4)
    };
    Some(Wifi {
        ssid: field("SSID:").map(str::to_string),
        signal_pct: rssi_dbm.map(signal_pct),
        rssi_dbm,
        channel: freq_mhz.and_then(channel_from_mhz),
        freq_mhz,
        band,
        phy,
        rx_mbps: mbps("rx bitrate:"),
        tx_mbps: mbps("tx bitrate:"),
    })
}

/// Signal quality from dBm, the way Windows shows it: -100 dBm is 0 %, -50 dBm and above 100 %.
pub fn signal_pct(dbm: i32) -> u32 {
    ((dbm + 100) * 2).clamp(0, 100) as u32
}

/// Signal level of `iface` from `/proc/net/wireless`, when `iw` isn't installed.
fn proc_wireless(root: &Root, iface: &str) -> Option<Wifi> {
    let text = root.read("/proc/net/wireless")?;
    let line = text
        .lines()
        .find(|l| l.trim_start().starts_with(&format!("{iface}:")))?;
    let dbm: f64 = line
        .split_whitespace()
        .nth(3)?
        .trim_end_matches('.')
        .parse()
        .ok()?;
    let dbm = dbm as i32;
    Some(Wifi {
        rssi_dbm: Some(dbm),
        signal_pct: Some(signal_pct(dbm)),
        ..Default::default()
    })
}

/// The DHCP server from NetworkManager (`nmcli -t -f DHCP4 device show`).
pub fn parse_nmcli_dhcp_server(text: &str) -> Option<IpAddr> {
    text.lines().find_map(|l| {
        let v = l.split_once("dhcp_server_identifier")?.1;
        v.trim_start_matches([' ', '=']).trim().parse().ok()
    })
}

/// The DHCP server of `iface`: systemd-networkd's lease file, else NetworkManager.
fn dhcp_server(root: &Root, index: u32, iface: &str) -> Option<IpAddr> {
    if let Some(lease) = root.read(&format!("/run/systemd/netif/leases/{index}")) {
        return lease
            .lines()
            .find_map(|l| l.strip_prefix("SERVER_ADDRESS="))
            .and_then(|s| s.parse().ok());
    }
    let out = Command::new("nmcli")
        .args(["-t", "-f", "DHCP4", "device", "show", iface])
        .output()
        .ok()?;
    parse_nmcli_dhcp_server(&String::from_utf8_lossy(&out.stdout))
}

/// ethtool link mode bits (`ETHTOOL_LINK_MODE_*_BIT`) of consumer Ethernet speeds, in bps.
const LINK_MODES: [(u32, u64); 19] = [
    (0, 10_000_000),
    (1, 10_000_000),
    (2, 100_000_000),
    (3, 100_000_000),
    (4, 1_000_000_000),
    (5, 1_000_000_000),
    (12, 10_000_000_000),
    (15, 2_500_000_000),
    (17, 1_000_000_000),
    (18, 10_000_000_000),
    (19, 10_000_000_000),
    (41, 1_000_000_000),
    (42, 10_000_000_000),
    (43, 10_000_000_000),
    (44, 10_000_000_000),
    (45, 10_000_000_000),
    (46, 10_000_000_000),
    (47, 2_500_000_000),
    (48, 5_000_000_000),
];

/// The fastest speed in a supported-link-modes bitmap.
pub fn max_speed_from_modes(supported: &[u32]) -> Option<u64> {
    LINK_MODES
        .iter()
        .filter(|(bit, _)| {
            supported
                .get((bit / 32) as usize)
                .is_some_and(|w| w & (1 << (bit % 32)) != 0)
        })
        .map(|&(_, bps)| bps)
        .max()
}

/// The fastest speed `iface` supports, from the `ETHTOOL_GLINKSETTINGS` ioctl (no root needed).
fn max_link_speed(iface: &str) -> Option<u64> {
    const SIOCETHTOOL: libc::c_ulong = 0x8946;
    const ETHTOOL_GLINKSETTINGS: u32 = 0x4c;
    /// `struct ethtool_link_settings` is 48 bytes; `link_mode_masks_nwords` is its 16th byte.
    const HEADER_WORDS: usize = 12;
    const MAX_NWORDS: usize = 127;

    let name = iface.as_bytes();
    if name.len() >= libc::IFNAMSIZ {
        return None;
    }
    // SAFETY: a plain datagram socket, closed below.
    let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, 0) };
    if fd < 0 {
        return None;
    }
    let mut buf = [0u32; HEADER_WORDS + 3 * MAX_NWORDS];
    let call = |buf: &mut [u32], nwords: i8| -> bool {
        buf.fill(0);
        buf[0] = ETHTOOL_GLINKSETTINGS;
        buf[3] = u32::from_ne_bytes([0, 0, 0, nwords as u8]);
        // SAFETY: zeroed ifreq with a NUL-terminated name; ifru_data points to `buf`, which is
        // large enough for the header plus three masks of MAX_NWORDS words and outlives the call.
        unsafe {
            let mut ifr: libc::ifreq = std::mem::zeroed();
            for (d, s) in ifr.ifr_name.iter_mut().zip(name) {
                *d = *s as libc::c_char;
            }
            ifr.ifr_ifru.ifru_data = buf.as_mut_ptr().cast();
            libc::ioctl(fd, SIOCETHTOOL as _, &mut ifr) == 0
        }
    };
    // The first call returns the mask size as a negative word count (the handshake).
    let nwords = if call(&mut buf, 0) {
        -(buf[3].to_ne_bytes()[3] as i8)
    } else {
        0
    };
    let ok = nwords > 0 && nwords as usize <= MAX_NWORDS && call(&mut buf, nwords);
    // SAFETY: closing the socket opened above.
    unsafe { libc::close(fd) };
    if !ok {
        return None;
    }
    let n = nwords as usize;
    max_speed_from_modes(&buf[HEADER_WORDS..HEADER_WORDS + n])
}

/// Run a command and return its standard output, if it ran successfully.
fn output(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd)
        .args(args)
        .env("LC_ALL", "C")
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The kind of interface and whether it's backed by hardware.
fn media(root: &Root, link: &Link) -> (Media, bool) {
    let p = format!("{NET}/{}", link.name);
    let physical = root.exists(&format!("{p}/device"));
    let uevent = root.read(&format!("{p}/uevent")).unwrap_or_default();
    let devtype = uevent
        .lines()
        .find_map(|l| l.strip_prefix("DEVTYPE="))
        .unwrap_or_default();
    let m = if root.exists(&format!("{p}/wireless")) || root.exists(&format!("{p}/phy80211")) {
        Media::WiFi
    } else if devtype == "wwan" {
        Media::Cellular
    } else if devtype == "bluetooth" || link.name.starts_with("bnep") {
        Media::Bluetooth
    } else if link.link_type == "ether" {
        Media::Ethernet
    } else if link.link_type == "none" || devtype == "wireguard" {
        Media::Tunnel
    } else {
        Media::Other
    };
    (m, physical)
}

fn description(root: &Root, link: &Link, physical: bool) -> String {
    if let Some(m) = adapter_model(root, &link.name) {
        return m;
    }
    let p = format!("{NET}/{}", link.name);
    if physical {
        return "Network adapter".into();
    }
    let kind = if root.exists(&format!("{p}/bridge")) {
        "bridge"
    } else if root.exists(&format!("{p}/bonding")) {
        "bond"
    } else if root.exists(&format!("{p}/tun_flags")) {
        "TUN/TAP"
    } else {
        ""
    };
    match kind {
        "" => "Virtual adapter".into(),
        k => format!("Virtual adapter ({k})"),
    }
}

fn driver(root: &Root, iface: &str) -> Option<Driver> {
    let name = root.link_name(&format!("{NET}/{iface}/device/driver"))?;
    let version = root.read_nonempty(&format!("/sys/module/{name}/version"));
    Some(Driver {
        version: match version {
            Some(v) => format!("{name} {v}"),
            None => name,
        },
        ..Default::default()
    })
}

/// Build the adapters from the parsed iproute2 output and DNS settings. Wi-Fi, the DHCP server
/// and the maximum speed are filled in by [`gather`].
pub fn adapters(
    root: &Root,
    links: Vec<Link>,
    gateways: &[(String, IpAddr)],
    dns: &DnsConfig,
) -> Vec<Adapter> {
    links
        .into_iter()
        // Loopback and the per-container veth pairs Docker and Podman create.
        .filter(|l| l.link_type != "loopback" && !l.name.starts_with("veth"))
        .map(|l| {
            let p = format!("{NET}/{}", l.name);
            let (media, physical) = media(root, &l);
            let has_address = !l.ipv4.is_empty() || l.ipv6.iter().any(|e| !is_link_local(e.addr));
            let status = if !l.up {
                Status::Disabled
            } else if l.operstate == "UP" || (l.operstate == "UNKNOWN" && l.lower_up && has_address)
            {
                Status::Up
            } else {
                Status::Disconnected
            };
            let gws: Vec<IpAddr> = gateways
                .iter()
                .filter(|(dev, _)| *dev == l.name)
                .map(|(_, g)| *g)
                .collect();
            let link_dns = dns
                .links
                .iter()
                .find(|(n, _)| *n == l.name)
                .map(|(_, d)| d.clone());
            // Global servers apply to the interfaces that carry the default route.
            let servers = match link_dns {
                Some(d) => d,
                None if !gws.is_empty() => dns.global.clone(),
                None => Vec::new(),
            };
            let suffix = dns
                .domains
                .iter()
                .find(|(n, _)| *n == l.name)
                .or_else(|| {
                    (!gws.is_empty())
                        .then(|| dns.domains.iter().find(|(n, _)| n == "Global"))
                        .flatten()
                })
                .map(|(_, d)| d.clone())
                .unwrap_or_default();
            let wired_up = media == Media::Ethernet && status == Status::Up;
            Adapter {
                if_index: l.index,
                description: description(root, &l, physical),
                media,
                physical,
                status,
                mac: l.mac.clone(),
                speed_bps: root
                    .read_i64(&format!("{p}/speed"))
                    .filter(|&s| wired_up && s > 0)
                    .map(|s| s as u64 * 1_000_000),
                duplex: match root.read(&format!("{p}/duplex")).as_deref() {
                    Some("full") if wired_up => Some(Duplex::Full),
                    Some("half") if wired_up => Some(Duplex::Half),
                    _ => None,
                },
                mtu: l.mtu,
                dhcp_enabled: l.ipv4.iter().any(|e| e.dhcp),
                gateways: gws,
                dns: servers,
                dns_suffix: suffix,
                driver: driver(root, &l.name),
                name: l.name,
                ipv4: l.ipv4,
                ipv6: l.ipv6,
                ..Default::default()
            }
        })
        .collect()
}

fn is_link_local(a: IpAddr) -> bool {
    match a {
        IpAddr::V4(v) => v.is_link_local(),
        IpAddr::V6(v) => v.segments()[0] & 0xffc0 == 0xfe80,
    }
}

/// Read every adapter. `detailed` also asks NetworkManager for the DHCP server (a process per
/// adapter); otherwise it's carried over from `previous`.
pub fn gather(detailed: bool, previous: Option<&NetworkInfo>) -> NetworkInfo {
    let root = Root::system();
    let links = parse_ip_addr(&output("ip", &["-j", "addr"]).unwrap_or_default());
    let mut gateways = parse_default_routes(&output("ip", &["-j", "route"]).unwrap_or_default());
    gateways.extend(parse_default_routes(
        &output("ip", &["-j", "-6", "route"]).unwrap_or_default(),
    ));
    let dns = match (
        output("resolvectl", &["dns"]),
        output("resolvectl", &["domain"]),
    ) {
        (Some(d), domains) => parse_resolvectl(&d, &domains.unwrap_or_default()),
        _ => parse_resolv_conf(&root.read("/etc/resolv.conf").unwrap_or_default()),
    };
    let mut adapters = adapters(&root, links, &gateways, &dns);
    for a in &mut adapters {
        let old = previous.and_then(|p| p.adapters.iter().find(|o| o.name == a.name));
        if a.media == Media::Ethernet && a.physical {
            a.max_speed_bps = max_link_speed(&a.name);
        }
        if a.media == Media::WiFi && a.status == Status::Up {
            a.wifi = output("iw", &["dev", &a.name, "link"])
                .and_then(|t| parse_iw_link(&t))
                .or_else(|| proc_wireless(&root, &a.name));
        }
        if a.dhcp_enabled && a.status == Status::Up {
            a.dhcp_server = if detailed {
                dhcp_server(&root, a.if_index, &a.name)
            } else {
                old.and_then(|o| o.dhcp_server)
            };
        }
    }
    NetworkInfo {
        adapters,
        detailed: detailed || previous.is_some_and(|p| p.detailed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::linux::Fixture;

    const IP_ADDR: &str = r#"[{"ifindex":1,"ifname":"lo","flags":["LOOPBACK","UP","LOWER_UP"],"mtu":65536,"operstate":"UNKNOWN","link_type":"loopback","address":"00:00:00:00:00:00","addr_info":[{"family":"inet","local":"127.0.0.1","prefixlen":8,"valid_life_time":4294967295}]},
    {"ifindex":2,"ifname":"enp5s0","flags":["BROADCAST","MULTICAST","UP","LOWER_UP"],"mtu":1500,"operstate":"UP","link_type":"ether","address":"d8:bb:c1:12:34:56","addr_info":[{"family":"inet","local":"192.168.1.20","prefixlen":24,"dynamic":true,"valid_life_time":85000},{"family":"inet6","local":"fe80::1234","prefixlen":64,"scope":"link","valid_life_time":4294967295}]},
    {"ifindex":3,"ifname":"wlp6s0","flags":["BROADCAST","MULTICAST","UP"],"mtu":1500,"operstate":"DOWN","link_type":"ether","address":"aa:bb:cc:dd:ee:ff","addr_info":[]},
    {"ifindex":4,"ifname":"eno1","flags":["BROADCAST","MULTICAST"],"mtu":1500,"operstate":"DOWN","link_type":"ether","address":"11:22:33:44:55:66","addr_info":[]},
    {"ifindex":5,"ifname":"wg0","flags":["POINTOPOINT","NOARP","UP","LOWER_UP"],"mtu":1420,"operstate":"UNKNOWN","link_type":"none","addr_info":[{"family":"inet","local":"10.8.0.2","prefixlen":32,"valid_life_time":4294967295,"dadfailed":true}]},
    {"ifindex":6,"ifname":"veth1a2b","flags":["UP"],"operstate":"UP","link_type":"ether","addr_info":[]}]"#;

    const ROUTES: &str = r#"[{"dst":"default","gateway":"192.168.1.1","dev":"enp5s0","protocol":"dhcp"},{"dst":"192.168.1.0/24","dev":"enp5s0","scope":"link"}]"#;

    #[test]
    fn iproute2_and_sysfs() {
        let links = parse_ip_addr(IP_ADDR);
        assert_eq!(links.len(), 6);
        let e = &links[1];
        assert_eq!((e.index, e.name.as_str(), e.mtu), (2, "enp5s0", Some(1500)));
        assert_eq!(e.mac, "D8-BB-C1-12-34-56");
        assert!(e.ipv4[0].dhcp);
        assert_eq!(e.ipv4[0].lease_secs, Some(85000));
        assert_eq!(e.ipv6[0].lease_secs, None);
        assert!(links[4].ipv4[0].duplicate);

        let gws = parse_default_routes(ROUTES);
        assert_eq!(
            gws,
            vec![("enp5s0".to_string(), "192.168.1.1".parse().unwrap())]
        );

        let f = Fixture::new();
        f.dir("sys/class/net/enp5s0/device")
            .file("sys/class/net/enp5s0/speed", "1000\n")
            .file("sys/class/net/enp5s0/duplex", "full\n")
            .dir("sys/class/net/wlp6s0/device")
            .dir("sys/class/net/wlp6s0/wireless")
            .dir("sys/class/net/eno1/device")
            .file(
                "sys/class/net/wg0/uevent",
                "DEVTYPE=wireguard\nINTERFACE=wg0\n",
            );
        let dns = DnsConfig {
            global: vec!["1.1.1.1".parse().unwrap()],
            ..Default::default()
        };
        let a = adapters(&f.root(), links, &gws, &dns);
        assert_eq!(
            a.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(),
            ["enp5s0", "wlp6s0", "eno1", "wg0"],
            "loopback and veth are left out"
        );
        let eth = &a[0];
        assert_eq!(
            (eth.media, eth.physical, eth.status),
            (Media::Ethernet, true, Status::Up)
        );
        assert_eq!(eth.link_text(), "1 Gbps, full duplex");
        assert!(eth.dhcp_enabled);
        assert_eq!(
            eth.dns, dns.global,
            "global DNS on the default-route adapter"
        );
        assert_eq!(
            (a[1].media, a[1].status),
            (Media::WiFi, Status::Disconnected)
        );
        assert_eq!(a[2].status, Status::Disabled);
        assert_eq!(
            (a[3].media, a[3].status, a[3].physical),
            (Media::Tunnel, Status::Up, false)
        );
        assert!(a[3].dns.is_empty() && a[3].has_duplicate());
    }

    #[test]
    fn dns_settings() {
        let c = parse_resolvectl(
            "Global: 10.255.255.254\nLink 2 (enp5s0): 192.168.1.1 fe80::1%2 9.9.9.9#dns.quad9.net\nLink 3 (wlp6s0):\n",
            "Global:\nLink 2 (enp5s0): ~. lan\n",
        );
        assert_eq!(c.global, vec!["10.255.255.254".parse::<IpAddr>().unwrap()]);
        assert_eq!(c.links.len(), 1);
        assert_eq!(c.links[0].1.len(), 3);
        assert_eq!(c.domains, vec![("enp5s0".to_string(), "lan".to_string())]);

        let r = parse_resolv_conf(
            "# stub\nnameserver 127.0.0.53\nnameserver 192.168.1.1\nsearch home.arpa\n",
        );
        assert_eq!(r.global, vec!["192.168.1.1".parse::<IpAddr>().unwrap()]);
        assert_eq!(r.domains[0].1, "home.arpa");
    }

    #[test]
    fn wifi_and_dhcp() {
        let w = parse_iw_link(
            "Connected to aa:bb:cc:dd:ee:ff (on wlp6s0)\n\tSSID: HomeNet\n\tfreq: 5180.0\n\tRX: 1234 bytes (10 packets)\n\tsignal: -52 dBm\n\trx bitrate: 866.7 MBit/s VHT-MCS 9 80MHz short GI VHT-NSS 2\n\ttx bitrate: 780.0 MBit/s VHT-MCS 8 80MHz VHT-NSS 2\n",
        )
        .unwrap();
        assert_eq!(w.ssid.as_deref(), Some("HomeNet"));
        assert_eq!(
            (w.freq_mhz, w.channel, w.band),
            (Some(5180), Some(36), Some(Band::G5))
        );
        assert_eq!((w.rssi_dbm, w.signal_pct), (Some(-52), Some(96)));
        assert_eq!((w.rx_mbps, w.tx_mbps), (Some(867), Some(780)));
        assert_eq!(w.phy, phy_name(8));
        assert_eq!(parse_iw_link("Not connected.\n"), None);

        assert_eq!(
            parse_nmcli_dhcp_server(
                "DHCP4.OPTION[1]:broadcast_address = 192.168.1.255\nDHCP4.OPTION[3]:dhcp_server_identifier = 192.168.1.1\n"
            ),
            Some("192.168.1.1".parse().unwrap())
        );

        let f = Fixture::new();
        f.file(
            "proc/net/wireless",
            "Inter-| sta-|   Quality        |\n face | tus | link level noise |\nwlp6s0: 0000   58.  -52.  -256        0      0      0      0     12        0\n",
        );
        assert_eq!(
            proc_wireless(&f.root(), "wlp6s0").unwrap().rssi_dbm,
            Some(-52)
        );
    }

    #[test]
    fn link_modes() {
        // 1000baseT_Full (bit 5) and 2500baseT_Full (bit 47).
        assert_eq!(
            max_speed_from_modes(&[1 << 5, 1 << 15]),
            Some(2_500_000_000)
        );
        assert_eq!(max_speed_from_modes(&[1 << 3]), Some(100_000_000));
        assert_eq!(max_speed_from_modes(&[1 << 6]), None, "autoneg only");
    }
}
