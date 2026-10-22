//! The LAN Security Report: a defensive scan of the user's own local network.
//!
//! It runs only when the user clicks the button, and it stays on the local network: the
//! subnets it looks at come from the connected adapters' own private IPv4 ranges (never a
//! public range, see [`crate::network::is_local`]), and it refuses a range larger than
//! [`MIN_PREFIX`] so a scan can't balloon to tens of thousands of hosts.
//!
//! What it does is identify and explain, never exploit: it finds which devices answer, which
//! well-known TCP ports they have open, and reads the greeting a service offers on connect
//! (a "banner"). From that, plus a couple of read-only checks of this PC's own posture, it
//! produces ranked [`Finding`]s with a plain explanation and a help link for each. It never
//! tries passwords, never sends a service anything beyond a bare HTTP request used to read a
//! banner, and changes nothing on any device.
//!
//! The pure logic here (subnet enumeration, port table, banner tidy-up, vendor lookup and
//! [`analyze`]) is platform-agnostic and unit-tested. The parts that touch the network —
//! ICMP liveness, the neighbour (ARP) table and reverse DNS — live in [`imp`] (Windows), and
//! the TCP connect/banner step uses the standard library so it works anywhere.

use crate::network::{Adapter, Status, is_local};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::sync::atomic::{AtomicU8, AtomicU32, Ordering};
use std::time::Duration;

/// Smallest prefix length (largest subnet) a scan will enumerate: /22 is 1024 addresses. A
/// bigger range is refused so a mistyped or unusually wide subnet can't start a huge scan.
pub const MIN_PREFIX: u8 = 22;

/// How long to wait for a TCP connect when probing a port.
const CONNECT_TIMEOUT: Duration = Duration::from_millis(400);
/// How long to wait for a service to send its banner after connecting.
const BANNER_TIMEOUT: Duration = Duration::from_millis(600);
/// Most bytes of a banner kept.
const BANNER_MAX: usize = 200;
/// Worker threads used for the parallel connect/ping work.
const WORKERS: usize = 64;

/// How risky it is, in general, for a service to be reachable on the LAN. Context in
/// [`analyze`] can raise or lower the finding a port leads to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Risk {
    /// Normal to see; listed for completeness.
    Info,
    /// Worth a look.
    Low,
    /// Should usually be locked down or off.
    Medium,
    /// Commonly unauthenticated or clear-text; treat as exposed.
    High,
}

/// A well-known TCP port the scan probes, with the service it usually means and a baseline
/// [`Risk`]. Kept deliberately short and common; this is a health check, not an audit tool.
pub struct PortInfo {
    pub port: u16,
    pub service: &'static str,
    pub risk: Risk,
    /// Why it matters, shown in a finding.
    pub note: &'static str,
}

const fn p(port: u16, service: &'static str, risk: Risk, note: &'static str) -> PortInfo {
    PortInfo {
        port,
        service,
        risk,
        note,
    }
}

/// The ports probed on each live host. Ordered low to high.
pub const SCAN_PORTS: &[PortInfo] = &[
    p(21, "FTP", Risk::High, "File transfer with no encryption"),
    p(22, "SSH", Risk::Low, "Encrypted remote login"),
    p(
        23,
        "Telnet",
        Risk::High,
        "Remote console with no encryption",
    ),
    p(25, "SMTP", Risk::Info, "Mail transfer"),
    p(53, "DNS", Risk::Info, "Name lookups"),
    p(
        80,
        "HTTP",
        Risk::Low,
        "Web page or admin console without encryption",
    ),
    p(110, "POP3", Risk::Low, "Mail retrieval without encryption"),
    p(135, "MS RPC", Risk::Low, "Windows remote procedure calls"),
    p(
        139,
        "NetBIOS",
        Risk::Medium,
        "Legacy Windows file/printer sharing",
    ),
    p(143, "IMAP", Risk::Low, "Mail retrieval without encryption"),
    p(445, "SMB", Risk::Medium, "Windows file and printer sharing"),
    p(515, "LPD", Risk::Info, "Network printing"),
    p(554, "RTSP", Risk::Info, "Camera or media stream"),
    p(631, "IPP", Risk::Info, "Network printing"),
    p(1433, "MS SQL", Risk::Medium, "Database server"),
    p(
        2323,
        "Telnet (alt)",
        Risk::High,
        "Remote console with no encryption",
    ),
    p(3306, "MySQL", Risk::Medium, "Database server"),
    p(3389, "RDP", Risk::Medium, "Windows Remote Desktop"),
    p(5000, "UPnP/HTTP", Risk::Low, "Device web service"),
    p(5432, "PostgreSQL", Risk::Medium, "Database server"),
    p(
        5900,
        "VNC",
        Risk::High,
        "Remote desktop, often with weak or no password",
    ),
    p(
        6379,
        "Redis",
        Risk::High,
        "Database that is unauthenticated by default",
    ),
    p(
        7547,
        "TR-069",
        Risk::Medium,
        "ISP remote management of the router",
    ),
    p(
        8080,
        "HTTP (alt)",
        Risk::Low,
        "Web page or admin console without encryption",
    ),
    p(8443, "HTTPS (alt)", Risk::Info, "Encrypted web console"),
    p(9100, "Raw print", Risk::Info, "Direct-to-printer port"),
    p(
        27017,
        "MongoDB",
        Risk::High,
        "Database that is unauthenticated by default",
    ),
];

/// Look up a port in [`SCAN_PORTS`].
pub fn port_info(port: u16) -> Option<&'static PortInfo> {
    SCAN_PORTS.iter().find(|p| p.port == port)
}

/// An open port found on a host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenPort {
    pub port: u16,
    pub service: &'static str,
    pub risk: Risk,
    /// The service's greeting on connect, tidied to printable text (empty if none).
    pub banner: String,
}

/// A device found on the local network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Host {
    pub ip: Ipv4Addr,
    /// Hardware address from the neighbour table, "D8-BB-C1-12-34-56" (empty if not known).
    pub mac: String,
    /// Maker guessed from the MAC's OUI (empty if not recognised).
    pub vendor: String,
    /// Reverse-DNS name (empty if none).
    pub hostname: String,
    /// This PC itself.
    pub is_self: bool,
    /// The default gateway (router) for its subnet.
    pub is_gateway: bool,
    pub ports: Vec<OpenPort>,
}

impl Host {
    /// A short name for the device: its hostname, else the maker, else the IP.
    pub fn label(&self) -> String {
        if !self.hostname.is_empty() {
            self.hostname.clone()
        } else if !self.vendor.is_empty() {
            self.vendor.clone()
        } else {
            self.ip.to_string()
        }
    }
}

/// One subnet that was scanned (an adapter's own private IPv4 range).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubnetScan {
    pub adapter: String,
    /// "192.168.1.0/24".
    pub cidr: String,
    pub self_ip: Ipv4Addr,
    pub gateway: Option<Ipv4Addr>,
    pub hosts: Vec<Host>,
}

/// How serious a [`Finding`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// Just so you know.
    Info,
    /// Worth checking or tightening.
    Warn,
    /// Should be fixed.
    High,
}

impl Severity {
    pub fn name(self) -> &'static str {
        match self {
            Severity::Info => "INFO",
            Severity::Warn => "CHECK",
            Severity::High => "RISK",
        }
    }
}

/// One issue the report raises, with a plain explanation and a link to fix steps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub severity: Severity,
    /// Device the finding is about (empty for whole-network or this-PC findings).
    pub host: String,
    pub title: String,
    pub detail: String,
    /// A help page the user can open to fix it (empty if none).
    pub help_url: &'static str,
}

/// Read-only facts about this PC's own posture, folded into the findings.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LocalPosture {
    /// The SMBv1 client/server is installed and enabled (an old, insecure protocol).
    pub smb1_enabled: Option<bool>,
}

/// A finished report.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LanScan {
    pub subnets: Vec<SubnetScan>,
    pub findings: Vec<Finding>,
}

impl LanScan {
    pub fn total_hosts(&self) -> usize {
        self.subnets.iter().map(|s| s.hosts.len()).sum()
    }

    pub fn counts(&self) -> (usize, usize, usize) {
        let mut c = (0, 0, 0);
        for f in &self.findings {
            match f.severity {
                Severity::High => c.0 += 1,
                Severity::Warn => c.1 += 1,
                Severity::Info => c.2 += 1,
            }
        }
        c
    }
}

/// Which stage the scan is in, for the progress bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Finding which addresses answer.
    Discovering,
    /// Probing ports on the live hosts.
    Scanning,
    Done,
}

/// Shared progress a running scan updates and the UI reads.
#[derive(Debug)]
pub struct Progress {
    phase: AtomicU8,
    done: AtomicU32,
    total: AtomicU32,
}

impl Default for Progress {
    fn default() -> Self {
        Progress {
            phase: AtomicU8::new(0),
            done: AtomicU32::new(0),
            total: AtomicU32::new(0),
        }
    }
}

impl Progress {
    fn set_phase(&self, p: Phase) {
        self.phase.store(p as u8, Ordering::Relaxed);
    }

    fn start(&self, phase: Phase, total: usize) {
        self.done.store(0, Ordering::Relaxed);
        self.total.store(total as u32, Ordering::Relaxed);
        self.set_phase(phase);
    }

    fn tick(&self) {
        self.done.fetch_add(1, Ordering::Relaxed);
    }

    /// (phase, done, total) for the UI.
    pub fn read(&self) -> (Phase, u32, u32) {
        let phase = match self.phase.load(Ordering::Relaxed) {
            0 => Phase::Discovering,
            1 => Phase::Scanning,
            _ => Phase::Done,
        };
        (
            phase,
            self.done.load(Ordering::Relaxed),
            self.total.load(Ordering::Relaxed),
        )
    }
}

/// The addresses to scan for one adapter, or `None` when the adapter has no usable private
/// IPv4 address or its subnet is larger than [`MIN_PREFIX`] allows.
pub fn subnet_hosts(ip: Ipv4Addr, prefix: u8) -> Option<Vec<Ipv4Addr>> {
    if !is_local(IpAddr::V4(ip)) || ip.is_loopback() || !(MIN_PREFIX..=30).contains(&prefix) {
        return None;
    }
    let bits = 32 - prefix as u32;
    let mask = u32::MAX.checked_shl(bits).unwrap_or(0);
    let base = u32::from(ip) & mask;
    let count = 1u32 << bits;
    // Leave out the network and broadcast addresses.
    Some((1..count - 1).map(|i| Ipv4Addr::from(base + i)).collect())
}

/// "192.168.1.0/24" for the subnet `ip` sits in.
pub fn cidr(ip: Ipv4Addr, prefix: u8) -> String {
    let mask = u32::MAX.checked_shl(32 - prefix as u32).unwrap_or(0);
    format!("{}/{prefix}", Ipv4Addr::from(u32::from(ip) & mask))
}

/// Run `jobs` across a small thread pool, returning the results in order. `each` is called on a
/// worker thread for each input; `tick` runs after each one (for progress).
fn parallel<T, R, F>(jobs: Vec<T>, tick: &(dyn Fn() + Sync), each: F) -> Vec<R>
where
    T: Send + Sync,
    R: Send + Default + Clone,
    F: Fn(&T) -> R + Sync,
{
    let n = jobs.len();
    if n == 0 {
        return Vec::new();
    }
    let out: Vec<std::sync::Mutex<R>> = (0..n)
        .map(|_| std::sync::Mutex::new(R::default()))
        .collect();
    let next = AtomicU32::new(0);
    let workers = WORKERS.min(n);
    std::thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed) as usize;
                    if i >= n {
                        break;
                    }
                    let r = each(&jobs[i]);
                    *out[i].lock().unwrap_or_else(|e| e.into_inner()) = r;
                    tick();
                }
            });
        }
    });
    out.into_iter()
        .map(|m| m.into_inner().unwrap_or_else(|e| e.into_inner()))
        .collect()
}

/// Scan the private subnets of every connected adapter. Blocks for up to a minute or so; call
/// on a worker thread. `progress` is updated as it goes.
pub fn scan(adapters: &[Adapter], progress: &Progress) -> LanScan {
    // Each connected adapter's primary private IPv4 and the hosts to probe for it.
    struct Net<'a> {
        adapter: &'a Adapter,
        self_ip: Ipv4Addr,
        prefix: u8,
        hosts: Vec<Ipv4Addr>,
    }
    let mut nets: Vec<Net> = Vec::new();
    for a in adapters {
        if a.status != Status::Up {
            continue;
        }
        let Some(entry) = a
            .ipv4
            .iter()
            .find(|e| is_local(e.addr) && !e.addr.is_loopback())
        else {
            continue;
        };
        let IpAddr::V4(self_ip) = entry.addr else {
            continue;
        };
        if let Some(hosts) = subnet_hosts(self_ip, entry.prefix) {
            nets.push(Net {
                adapter: a,
                self_ip,
                prefix: entry.prefix,
                hosts,
            });
        }
    }

    // Phase 1: which addresses answer (ICMP) or are already in the neighbour table.
    let all: Vec<Ipv4Addr> = nets.iter().flat_map(|n| n.hosts.iter().copied()).collect();
    progress.start(Phase::Discovering, all.len());
    let neighbours = imp::neighbours();
    let scope_for = |ip: Ipv4Addr| {
        nets.iter()
            .find(|n| same_subnet(ip, n.self_ip, n.prefix))
            .map(|n| n.adapter.if_index)
            .unwrap_or(0)
    };
    let alive = parallel(all, &|| progress.tick(), |&ip| {
        let up = imp::alive(ip, scope_for(ip)) || neighbours.iter().any(|(n, _)| *n == ip);
        up.then_some(ip)
    });
    let live: Vec<Ipv4Addr> = alive.into_iter().flatten().collect();

    // Phase 2: probe ports on every (host, port) pair.
    let jobs: Vec<(Ipv4Addr, &'static PortInfo)> = live
        .iter()
        .flat_map(|&ip| SCAN_PORTS.iter().map(move |pi| (ip, pi)))
        .collect();
    progress.start(Phase::Scanning, jobs.len());
    let probed = parallel(jobs, &|| progress.tick(), |&(ip, pi)| {
        probe_port(ip, pi).map(|banner| (ip, pi, banner))
    });
    progress.set_phase(Phase::Done);

    // Collect open ports per host.
    let mut open: std::collections::BTreeMap<Ipv4Addr, Vec<OpenPort>> = Default::default();
    for (ip, pi, banner) in probed.into_iter().flatten() {
        open.entry(ip).or_default().push(OpenPort {
            port: pi.port,
            service: pi.service,
            risk: pi.risk,
            banner,
        });
    }

    // Build the per-subnet host lists.
    let mac_of = |ip: Ipv4Addr| {
        neighbours
            .iter()
            .find(|(n, _)| *n == ip)
            .map(|(_, m)| crate::network::format_mac(m))
            .unwrap_or_default()
    };
    let mut subnets = Vec::new();
    for n in &nets {
        let gateway = n.adapter.gateways.iter().find_map(|g| match g {
            IpAddr::V4(v) if same_subnet(*v, n.self_ip, n.prefix) => Some(*v),
            _ => None,
        });
        let mut hosts: Vec<Host> = Vec::new();
        for &ip in live
            .iter()
            .filter(|&&ip| same_subnet(ip, n.self_ip, n.prefix))
        {
            let is_self = ip == n.self_ip;
            let mac = mac_of(ip);
            hosts.push(Host {
                ip,
                vendor: vendor_from_mac(&mac).unwrap_or_default().to_string(),
                mac,
                hostname: imp::reverse_dns(ip),
                is_self,
                is_gateway: gateway == Some(ip),
                ports: open.get(&ip).cloned().unwrap_or_default(),
            });
        }
        hosts.sort_by_key(|h| u32::from(h.ip));
        subnets.push(SubnetScan {
            adapter: n.adapter.name.clone(),
            cidr: cidr(n.self_ip, n.prefix),
            self_ip: n.self_ip,
            gateway,
            hosts,
        });
    }

    let findings = analyze(&subnets, &imp::local_posture());
    LanScan { subnets, findings }
}

/// Are `a` and `b` in the same `prefix`-bit subnet?
fn same_subnet(a: Ipv4Addr, b: Ipv4Addr, prefix: u8) -> bool {
    let mask = u32::MAX.checked_shl(32 - prefix as u32).unwrap_or(0);
    (u32::from(a) & mask) == (u32::from(b) & mask)
}

/// Connect to one port; `Some(banner)` if it's open (banner may be empty).
fn probe_port(ip: Ipv4Addr, pi: &PortInfo) -> Option<String> {
    let addr = SocketAddr::from((ip, pi.port));
    let mut stream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT).ok()?;
    Some(read_banner(&mut stream, pi.port))
}

/// Read a short banner from an open connection, tidied to printable text. For web ports a bare
/// HTTP request is sent first so the server replies; nothing else is ever sent.
fn read_banner(stream: &mut TcpStream, port: u16) -> String {
    use std::io::{Read, Write};
    let _ = stream.set_read_timeout(Some(BANNER_TIMEOUT));
    let _ = stream.set_write_timeout(Some(BANNER_TIMEOUT));
    if matches!(port, 80 | 8080 | 5000 | 7547) {
        let _ = stream.write_all(b"HEAD / HTTP/1.0\r\n\r\n");
    }
    let mut buf = [0u8; 512];
    let n = stream.read(&mut buf).unwrap_or(0);
    tidy_banner(&buf[..n], port)
}

/// Turn raw banner bytes into a short printable string. For HTTP, pull out the Server header.
fn tidy_banner(bytes: &[u8], port: u16) -> String {
    let text = String::from_utf8_lossy(bytes);
    if matches!(port, 80 | 8080 | 5000 | 7547)
        && let Some(server) = text.lines().find_map(|l| {
            l.strip_prefix("Server:")
                .or_else(|| l.strip_prefix("server:"))
        })
    {
        return clean(server.trim());
    }
    clean(text.lines().next().unwrap_or("").trim())
}

/// Keep printable ASCII, collapse the rest, and cap the length.
fn clean(s: &str) -> String {
    let mut out: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_graphic() || c == ' ' {
                c
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if out.len() > BANNER_MAX {
        out.truncate(BANNER_MAX);
        out.push('…');
    }
    out
}

/// Help links (open in the user's own browser; the app makes no request).
const HELP_SMB1: &str = "https://learn.microsoft.com/windows-server/storage/file-server/troubleshoot/detect-enable-and-disable-smbv1-v2-v3";
const HELP_ROUTER: &str = "https://www.cisa.gov/news-events/news/home-network-security";
const HELP_TELNET: &str = "https://www.google.com/search?q=disable+telnet+on+my+router";
const HELP_RDP: &str = "https://learn.microsoft.com/windows-server/remote/remote-desktop-services/clients/remote-desktop-allow-access";
const HELP_DB: &str = "https://www.google.com/search?q=restrict+database+to+localhost";
const HELP_VNC: &str = "https://www.google.com/search?q=secure+VNC+password+encryption";
const HELP_CAMERA: &str =
    "https://www.google.com/search?q=secure+IP+camera+change+default+password";

/// Turn the scanned subnets and this PC's posture into ranked findings.
pub fn analyze(subnets: &[SubnetScan], local: &LocalPosture) -> Vec<Finding> {
    let mut out: Vec<Finding> = Vec::new();

    if local.smb1_enabled == Some(true) {
        out.push(Finding {
            severity: Severity::High,
            host: "This PC".into(),
            title: "SMBv1 is enabled".into(),
            detail: "This PC still has the old SMBv1 file-sharing protocol turned on. It is \
                insecure and was used by WannaCry and NotPetya. Turn off the \"SMB 1.0/CIFS \
                File Sharing Support\" Windows feature unless a very old device needs it."
                .into(),
            help_url: HELP_SMB1,
        });
    }

    for subnet in subnets {
        for host in &subnet.hosts {
            let who = host.label();
            let at = if host.is_gateway {
                format!("{who} (your router, {})", host.ip)
            } else if host.is_self {
                format!("{who} (this PC, {})", host.ip)
            } else {
                format!("{who} ({})", host.ip)
            };
            for port in &host.ports {
                if let Some(f) = finding_for(&at, host, port) {
                    out.push(f);
                }
            }
        }
    }

    // Most serious first, then by device.
    out.sort_by(|a, b| b.severity.cmp(&a.severity).then(a.host.cmp(&b.host)));
    out
}

/// A finding for one open port in context, or `None` if it isn't worth raising on its own.
fn finding_for(at: &str, host: &Host, port: &OpenPort) -> Option<Finding> {
    let with_banner = |mut d: String| {
        if !port.banner.is_empty() {
            d.push_str(&format!(" (reports: \"{}\")", port.banner));
        }
        d
    };
    let (severity, title, detail, help): (Severity, String, String, &'static str) = match port.port
    {
        23 | 2323 => (
            Severity::High,
            "Telnet is open".into(),
            with_banner(format!(
                "{at} accepts Telnet, which sends everything — including the password — in clear \
                 text. Anyone on the network can read it. Disable Telnet and use SSH or the \
                 device's web console instead."
            )),
            HELP_TELNET,
        ),
        21 => (
            Severity::High,
            "FTP is open".into(),
            with_banner(format!(
                "{at} accepts FTP, which sends logins and files unencrypted. Prefer SFTP/FTPS or \
                 turn it off if it isn't needed."
            )),
            HELP_TELNET,
        ),
        5900 => (
            Severity::High,
            "VNC is open".into(),
            with_banner(format!(
                "{at} exposes VNC remote desktop. Many VNC servers use a weak password or none. \
                 Make sure it has a strong password and encryption, or restrict it."
            )),
            HELP_VNC,
        ),
        6379 | 27017 => (
            Severity::High,
            format!("{} is reachable", port.service),
            with_banner(format!(
                "{at} exposes {}, which is unauthenticated by default. If this isn't deliberate, \
                 bind it to localhost or require a password.",
                port.service
            )),
            HELP_DB,
        ),
        1433 | 3306 | 5432 => (
            Severity::Warn,
            format!("{} database is reachable on the LAN", port.service),
            with_banner(format!(
                "{at} exposes the {} database to the local network. If only this machine needs \
                 it, bind it to localhost.",
                port.service
            )),
            HELP_DB,
        ),
        3389 => (
            Severity::Warn,
            "Remote Desktop is open".into(),
            format!(
                "{at} has Windows Remote Desktop (RDP) open. That's fine on a trusted network, \
                 but make sure accounts have strong passwords and it isn't forwarded from the \
                 internet."
            ),
            HELP_RDP,
        ),
        80 | 8080 | 5000 if host.is_gateway => (
            Severity::Warn,
            "Router admin over plain HTTP".into(),
            with_banner(format!(
                "{at} serves its admin page over unencrypted HTTP. Change the default admin \
                 password if you haven't, and use HTTPS if the router offers it."
            )),
            HELP_ROUTER,
        ),
        7547 => (
            Severity::Warn,
            "TR-069 remote management is open".into(),
            with_banner(format!(
                "{at} has the TR-069 (CWMP) ISP-management port open. It should only be reachable \
                 from your ISP; a past bug (Mirai) abused it. Check your router's remote-management \
                 setting."
            )),
            HELP_ROUTER,
        ),
        139 => (
            Severity::Warn,
            "Legacy NetBIOS sharing is open".into(),
            format!(
                "{at} has NetBIOS (port 139) open — old Windows file sharing. Modern sharing uses \
                 port 445; NetBIOS can usually be turned off."
            ),
            HELP_SMB1,
        ),
        554 | 9100 | 515 | 631 => (
            Severity::Info,
            format!("{} service found", port.service),
            format!(
                "{at} looks like a printer or camera ({}). Make sure it has a password set and its \
                 firmware is up to date.",
                port.service
            ),
            HELP_CAMERA,
        ),
        _ => return None,
    };
    Some(Finding {
        severity,
        host: host.label(),
        title,
        detail,
        help_url: help,
    })
}

/// A short maker name for a MAC's OUI (first three bytes), best-effort. Not a full OUI database:
/// just common router, console and IoT makers so a device list is easier to read.
pub fn vendor_from_mac(mac: &str) -> Option<&'static str> {
    let hex: String = mac
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .take(6)
        .collect();
    if hex.len() < 6 {
        return None;
    }
    let oui = hex.to_ascii_uppercase();
    // A tiny curated table; unknown MACs just show no maker.
    const OUIS: &[(&str, &str)] = &[
        ("001018", "Broadcom"),
        ("0050F2", "Microsoft"),
        ("000C29", "VMware"),
        ("005056", "VMware"),
        ("080027", "VirtualBox"),
        ("001A11", "Google"),
        ("3C5AB4", "Google"),
        ("F4F5E8", "Google"),
        ("B827EB", "Raspberry Pi"),
        ("DCA632", "Raspberry Pi"),
        ("E45F01", "Raspberry Pi"),
        ("D83ADD", "Raspberry Pi"),
        ("FCFBFB", "Cisco"),
        ("00000C", "Cisco"),
        ("0018F3", "ASUS"),
        ("2C56DC", "ASUS"),
        ("AC220B", "ASUS"),
        ("1C872C", "ASUS"),
        ("C86000", "ASUS"),
        ("50465D", "ASUS"),
        ("0014BF", "Linksys"),
        ("001217", "Cisco-Linksys"),
        ("00226B", "Cisco-Linksys"),
        ("C0C1C0", "TP-Link"),
        ("50C7BF", "TP-Link"),
        ("1C61B4", "TP-Link"),
        ("003192", "Netgear"),
        ("A040A0", "Netgear"),
        ("2C3033", "Netgear"),
        ("9C3DCF", "Netgear"),
        ("744401", "Netgear"),
        ("0024B2", "Netgear"),
        ("E8FCAF", "Netgear"),
        ("F0B4D2", "D-Link"),
        ("1C7EE5", "D-Link"),
        ("842E27", "Samsung"),
        ("D0176A", "Samsung"),
        ("8425DB", "Samsung"),
        ("F0C1F1", "Apple"),
        ("ACBC32", "Apple"),
        ("3C0754", "Apple"),
        ("A4B197", "Apple"),
        ("DC2B2A", "Apple"),
        ("F86214", "Amazon"),
        ("FC65DE", "Amazon"),
        ("68DBF5", "Amazon"),
        ("50DCE7", "Amazon"),
        ("18742E", "Espressif (ESP IoT)"),
        ("240AC4", "Espressif (ESP IoT)"),
        ("7CDFA1", "Espressif (ESP IoT)"),
        ("CC50E3", "Espressif (ESP IoT)"),
        ("ECFABC", "Reolink"),
        ("E0509B", "Hangzhou (camera)"),
        ("BCAD28", "Hangzhou (camera)"),
        ("4419B6", "Hangzhou (camera)"),
    ];
    OUIS.iter()
        .find(|(prefix, _)| oui.starts_with(prefix))
        .map(|(_, name)| *name)
}

// ---- Platform backend -----------------------------------------------------------------------

mod imp {
    use super::LocalPosture;
    use std::net::Ipv4Addr;

    pub fn alive(_ip: Ipv4Addr, _scope: u32) -> bool {
        false
    }
    pub fn neighbours() -> Vec<(Ipv4Addr, [u8; 6])> {
        Vec::new()
    }
    pub fn reverse_dns(_ip: Ipv4Addr) -> String {
        String::new()
    }
    pub fn local_posture() -> LocalPosture {
        LocalPosture::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subnet_enumeration_and_bounds() {
        // A /24 has 254 usable hosts, first and last right.
        let hosts = subnet_hosts(Ipv4Addr::new(192, 168, 1, 50), 24).unwrap();
        assert_eq!(hosts.len(), 254);
        assert_eq!(hosts[0], Ipv4Addr::new(192, 168, 1, 1));
        assert_eq!(*hosts.last().unwrap(), Ipv4Addr::new(192, 168, 1, 254));
        // /30 has 2 usable hosts.
        assert_eq!(
            subnet_hosts(Ipv4Addr::new(10, 0, 0, 1), 30).unwrap().len(),
            2
        );
        // A public address is refused.
        assert!(subnet_hosts(Ipv4Addr::new(8, 8, 8, 8), 24).is_none());
        // Too wide (smaller prefix than MIN_PREFIX) is refused.
        assert!(subnet_hosts(Ipv4Addr::new(10, 0, 0, 1), 8).is_none());
        // Loopback refused.
        assert!(subnet_hosts(Ipv4Addr::new(127, 0, 0, 1), 24).is_none());
    }

    #[test]
    fn cidr_and_same_subnet() {
        assert_eq!(cidr(Ipv4Addr::new(192, 168, 1, 50), 24), "192.168.1.0/24");
        assert_eq!(cidr(Ipv4Addr::new(10, 1, 2, 3), 16), "10.1.0.0/16");
        assert!(same_subnet(
            Ipv4Addr::new(192, 168, 1, 1),
            Ipv4Addr::new(192, 168, 1, 254),
            24
        ));
        assert!(!same_subnet(
            Ipv4Addr::new(192, 168, 1, 1),
            Ipv4Addr::new(192, 168, 2, 1),
            24
        ));
    }

    #[test]
    fn banners_are_tidied() {
        assert_eq!(
            tidy_banner(b"220 ProFTPD 1.3.5 Server ready\r\n", 21),
            "220 ProFTPD 1.3.5 Server ready"
        );
        // HTTP: the Server header is pulled out.
        assert_eq!(
            tidy_banner(b"HTTP/1.1 200 OK\r\nServer: nginx/1.18\r\n\r\n", 80),
            "nginx/1.18"
        );
        // Control bytes are collapsed and it's capped.
        let long = vec![b'A'; 400];
        assert!(tidy_banner(&long, 22).ends_with('…'));
    }

    #[test]
    fn vendor_lookup() {
        assert_eq!(vendor_from_mac("B8-27-EB-12-34-56"), Some("Raspberry Pi"));
        assert_eq!(vendor_from_mac("b827eb123456"), Some("Raspberry Pi"));
        assert_eq!(vendor_from_mac("00-11-22-33-44-55"), None);
        assert_eq!(vendor_from_mac("short"), None);
    }

    #[test]
    fn analyze_flags_telnet_and_smb1() {
        let host = Host {
            ip: Ipv4Addr::new(192, 168, 1, 10),
            mac: String::new(),
            vendor: String::new(),
            hostname: "nas".into(),
            is_self: false,
            is_gateway: false,
            ports: vec![OpenPort {
                port: 23,
                service: "Telnet",
                risk: Risk::High,
                banner: String::new(),
            }],
        };
        let subnet = SubnetScan {
            adapter: "Ethernet".into(),
            cidr: "192.168.1.0/24".into(),
            self_ip: Ipv4Addr::new(192, 168, 1, 2),
            gateway: Some(Ipv4Addr::new(192, 168, 1, 1)),
            hosts: vec![host],
        };
        let findings = analyze(
            &[subnet],
            &LocalPosture {
                smb1_enabled: Some(true),
            },
        );
        // SMBv1 (High) sorts first, Telnet (High) present.
        assert!(findings.iter().any(|f| f.title.contains("SMBv1")));
        assert!(findings.iter().any(|f| f.title.contains("Telnet")));
        assert_eq!(findings[0].severity, Severity::High);
    }

    #[test]
    fn router_http_is_a_warning() {
        let gw = Host {
            ip: Ipv4Addr::new(192, 168, 1, 1),
            mac: String::new(),
            vendor: "Netgear".into(),
            hostname: String::new(),
            is_self: false,
            is_gateway: true,
            ports: vec![OpenPort {
                port: 80,
                service: "HTTP",
                risk: Risk::Low,
                banner: String::new(),
            }],
        };
        let subnet = SubnetScan {
            adapter: "Ethernet".into(),
            cidr: "192.168.1.0/24".into(),
            self_ip: Ipv4Addr::new(192, 168, 1, 2),
            gateway: Some(Ipv4Addr::new(192, 168, 1, 1)),
            hosts: vec![gw],
        };
        let f = analyze(&[subnet], &LocalPosture::default());
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].severity, Severity::Warn);
        assert!(f[0].title.contains("Router admin"));
    }
}
