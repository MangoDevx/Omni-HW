//! The network tests on Linux: ICMP echo through the `ping` crate (unprivileged datagram
//! sockets), DNS queries built with `simple-dns` and sent over UDP, one plain HTTP GET, and the
//! neighbour (ARP) table from `/proc/net/arp`. Besides the update check these are the only
//! functions that send packets, and they only run when the user starts a test (see
//! `crate::network::probe_local`, `crate::network::probe_internet` and `crate::lansec`).

use super::Root;
use crate::network::PingStats;
use simple_dns::rdata::RData;
use simple_dns::{CLASS, Packet, PacketFlag, Question, RCODE, TYPE};
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream, ToSocketAddrs, UdpSocket};
use std::time::Duration;

const PING_TIMEOUT: Duration = Duration::from_secs(1);
const DNS_TIMEOUT: Duration = Duration::from_secs(2);
const HTTP_TIMEOUT: Duration = Duration::from_secs(5);

fn is_link_local_v6(ip: IpAddr) -> bool {
    matches!(ip, IpAddr::V6(v) if v.segments()[0] & 0xffc0 == 0xfe80)
}

/// Name of the interface with this index.
fn if_name(index: u32) -> Option<String> {
    let mut buf = [0 as libc::c_char; libc::IF_NAMESIZE];
    // SAFETY: the buffer is IF_NAMESIZE bytes, as if_indextoname requires; on success it holds
    // a NUL-terminated name.
    let p = unsafe { libc::if_indextoname(index, buf.as_mut_ptr()) };
    if p.is_null() {
        return None;
    }
    // SAFETY: NUL-terminated by if_indextoname.
    let name = unsafe { std::ffi::CStr::from_ptr(buf.as_ptr()) };
    Some(name.to_string_lossy().into_owned())
}

/// Ping `ip` `count` times, `gap_ms` apart. `scope` is the interface index used for IPv6
/// link-local addresses.
pub fn ping_series(ip: IpAddr, scope: u32, count: u32, gap_ms: u64) -> PingStats {
    let mut builder = ping::Pinger::builder();
    if is_link_local_v6(ip)
        && let Some(name) = if_name(scope)
    {
        builder = builder.bind_device(name);
    }
    let mut stats = PingStats::default();
    let Ok(mut pinger) = builder.build() else {
        stats.sent = count;
        return stats;
    };
    for i in 0..count {
        if i > 0 {
            std::thread::sleep(Duration::from_millis(gap_ms));
        }
        stats.sent += 1;
        if let Ok(reply) = pinger.ping(ip, PING_TIMEOUT) {
            stats
                .rtts
                .push((reply.rtt.as_secs_f64() * 1000.0).round() as u32);
        }
    }
    stats
}

/// The reverse-DNS name of an IPv4 address ("20.1.168.192.in-addr.arpa").
fn reverse_name(ip: Ipv4Addr) -> String {
    let o = ip.octets();
    format!("{}.{}.{}.{}.in-addr.arpa", o[3], o[2], o[1], o[0])
}

/// A query packet for `name`.
pub fn build_query(id: u16, name: &str, ty: TYPE) -> Result<Vec<u8>, String> {
    let mut q = Packet::new_query(id);
    q.set_flags(PacketFlag::RECURSION_DESIRED);
    let qname = simple_dns::Name::new(name).map_err(|e| e.to_string())?;
    q.questions
        .push(Question::new(qname, ty.into(), CLASS::IN.into(), false));
    q.build_bytes_vec().map_err(|e| e.to_string())
}

/// Send a query to `server` and return the response with the same id (one retry).
fn exchange(server: IpAddr, name: &str, ty: TYPE) -> Result<Vec<u8>, String> {
    let id = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0)
        ^ std::process::id()) as u16;
    let query = build_query(id, name, ty)?;
    let bind: SocketAddr = if server.is_ipv4() {
        (Ipv4Addr::UNSPECIFIED, 0).into()
    } else {
        (std::net::Ipv6Addr::UNSPECIFIED, 0).into()
    };
    let sock = UdpSocket::bind(bind).map_err(|e| e.to_string())?;
    sock.connect((server, 53)).map_err(|e| e.to_string())?;
    sock.set_read_timeout(Some(DNS_TIMEOUT))
        .map_err(|e| e.to_string())?;
    let mut buf = [0u8; 1500];
    for _ in 0..2 {
        sock.send(&query).map_err(|e| e.to_string())?;
        loop {
            match sock.recv(&mut buf) {
                Ok(n) if n >= 2 && u16::from_be_bytes([buf[0], buf[1]]) == id => {
                    return Ok(buf[..n].to_vec());
                }
                // Someone else's answer: keep waiting.
                Ok(_) => continue,
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    break;
                }
                Err(e) => return Err(e.to_string()),
            }
        }
    }
    Err("timed out".into())
}

/// The system's DNS server: the first `nameserver` in `/etc/resolv.conf` (systemd-resolved's
/// stub 127.0.0.53 on Ubuntu, which forwards to the real servers).
fn system_dns(root: &Root) -> IpAddr {
    root.read("/etc/resolv.conf")
        .unwrap_or_default()
        .lines()
        .find_map(|l| {
            let mut f = l.split_whitespace();
            (f.next() == Some("nameserver")).then(|| f.next()?.split('%').next()?.parse().ok())?
        })
        .unwrap_or(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 53)))
}

/// Does this (local) DNS server answer? Asks it for the reverse name of its own address, which
/// local servers answer themselves; any answer (even "no such name") counts. IPv6 servers are
/// pinged instead.
pub fn dns_server_answers(server: IpAddr, scope: u32) -> bool {
    match server {
        IpAddr::V4(v4) => {
            exchange(server, &reverse_name(v4), TYPE::PTR).is_ok_and(|r| Packet::parse(&r).is_ok())
        }
        IpAddr::V6(_) => ping_series(server, scope, 2, 200).received() > 0,
    }
}

/// The A records in a response, or why there are none.
pub fn parse_a(response: &[u8]) -> Result<Vec<Ipv4Addr>, String> {
    let p = Packet::parse(response).map_err(|e| e.to_string())?;
    match p.rcode() {
        RCODE::NoError => {}
        RCODE::NameError => return Err("name not found".into()),
        c => return Err(format!("the DNS server answered {c:?}")),
    }
    Ok(p.answers
        .iter()
        .filter_map(|r| match &r.rdata {
            RData::A(a) => Some(Ipv4Addr::from(a.address)),
            _ => None,
        })
        .collect())
}

/// The first PTR name in a response.
pub fn parse_ptr(response: &[u8]) -> Option<String> {
    let p = Packet::parse(response).ok()?;
    p.answers.iter().find_map(|r| match &r.rdata {
        RData::PTR(name) => Some(name.0.to_string()).filter(|s| !s.is_empty()),
        _ => None,
    })
}

/// IPv4 addresses of `name` from the system's DNS server.
pub fn resolve_a(name: &str) -> Result<Vec<Ipv4Addr>, String> {
    parse_a(&exchange(system_dns(&Root::system()), name, TYPE::A)?)
}

/// Reverse-DNS (PTR) name for `ip`, from the system's DNS server. Used by the LAN Security
/// Report to label devices; `None` when nothing answers.
pub fn reverse_dns(ip: Ipv4Addr) -> Option<String> {
    let r = exchange(system_dns(&Root::system()), &reverse_name(ip), TYPE::PTR).ok()?;
    parse_ptr(&r)
}

/// Status code and body of an HTTP/1.x response (chunked bodies decoded).
pub fn parse_http(raw: &[u8]) -> Result<(u16, String), String> {
    let text = String::from_utf8_lossy(raw);
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let code = head
        .lines()
        .next()
        .filter(|l| l.starts_with("HTTP/"))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|c| c.parse().ok())
        .ok_or("not an HTTP response")?;
    let chunked = head
        .lines()
        .any(|l| l.to_ascii_lowercase().replace(' ', "") == "transfer-encoding:chunked");
    let body = if chunked {
        let mut out = String::new();
        let mut rest = body;
        while let Some((size, after)) = rest.split_once("\r\n") {
            let n =
                usize::from_str_radix(size.split(';').next().unwrap_or("").trim(), 16).unwrap_or(0);
            if n == 0 || after.len() < n {
                out.push_str(&after[..n.min(after.len())]);
                break;
            }
            out.push_str(&after[..n]);
            rest = after[n..].trim_start_matches("\r\n");
        }
        out
    } else {
        body.to_string()
    };
    Ok((code, body))
}

/// Plain-HTTP GET of `http://host/path`: (status code, first 4 KB of the body). Redirects
/// aren't followed: a captive portal shows up as its redirect status.
pub fn http_get(host: &str, path: &str) -> Result<(u16, String), String> {
    let addrs: Vec<SocketAddr> = (host, 80)
        .to_socket_addrs()
        .map_err(|e| e.to_string())?
        .collect();
    let mut last = String::from("no address");
    let mut stream = None;
    for a in addrs {
        match TcpStream::connect_timeout(&a, HTTP_TIMEOUT) {
            Ok(s) => {
                stream = Some(s);
                break;
            }
            Err(e) => last = e.to_string(),
        }
    }
    let mut s = stream.ok_or(last)?;
    s.set_read_timeout(Some(HTTP_TIMEOUT))
        .and_then(|_| s.set_write_timeout(Some(HTTP_TIMEOUT)))
        .map_err(|e| e.to_string())?;
    let req = format!(
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nUser-Agent: Omni-HW/{}\r\nConnection: close\r\n\r\n",
        env!("CARGO_PKG_VERSION")
    );
    s.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
    let mut raw = Vec::new();
    let mut buf = [0u8; 2048];
    while raw.len() < 8192 {
        match s.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => raw.extend_from_slice(&buf[..n]),
            Err(e) if raw.is_empty() => return Err(e.to_string()),
            Err(_) => break,
        }
    }
    let (code, mut body) = parse_http(&raw)?;
    body.truncate(4096);
    Ok((code, body))
}

/// The IPv4 neighbour (ARP) table: (address, 6-byte MAC) for each complete entry.
pub fn neighbours(root: &Root) -> Vec<(Ipv4Addr, [u8; 6])> {
    let text = root.read("/proc/net/arp").unwrap_or_default();
    text.lines()
        .skip(1)
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            let ip: Ipv4Addr = f.first()?.parse().ok()?;
            // ATF_COM: the entry is complete.
            let flags = u32::from_str_radix(f.get(2)?.trim_start_matches("0x"), 16).ok()?;
            if flags & 0x2 == 0 {
                return None;
            }
            let mut mac = [0u8; 6];
            let parts: Vec<&str> = f.get(3)?.split(':').collect();
            if parts.len() != 6 {
                return None;
            }
            for (m, p) in mac.iter_mut().zip(parts) {
                *m = u8::from_str_radix(p, 16).ok()?;
            }
            (mac != [0; 6] && mac != [0xff; 6]).then_some((ip, mac))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::linux::Fixture;

    #[test]
    fn dns_packets() {
        let q = build_query(0x1234, "dns.msftncsi.com", TYPE::A).unwrap();
        assert_eq!(&q[..2], &[0x12, 0x34]);
        assert_eq!(q[2] & 0x01, 0x01, "recursion desired");

        // A response: the query plus one A record (pointer to the question's name).
        let mut r = q.clone();
        r[2] |= 0x80; // response
        r[7] = 1; // ANCOUNT
        r.extend_from_slice(&[
            0xc0, 0x0c, 0, 1, 0, 1, 0, 0, 0, 60, 0, 4, 131, 107, 255, 255,
        ]);
        assert_eq!(parse_a(&r), Ok(vec![Ipv4Addr::new(131, 107, 255, 255)]));

        let mut nx = q.clone();
        nx[2] |= 0x80;
        nx[3] |= 3; // NXDOMAIN
        assert_eq!(parse_a(&nx), Err("name not found".to_string()));

        let ptr_q =
            build_query(7, &reverse_name(Ipv4Addr::new(192, 168, 1, 1)), TYPE::PTR).unwrap();
        let mut ptr = ptr_q.clone();
        ptr[2] |= 0x80;
        ptr[7] = 1;
        ptr.extend_from_slice(&[0xc0, 0x0c, 0, 12, 0, 1, 0, 0, 0, 60, 0, 8, 6]);
        ptr.extend_from_slice(b"router");
        ptr.push(0);
        assert_eq!(parse_ptr(&ptr).as_deref(), Some("router"));
    }

    #[test]
    fn http_responses() {
        assert_eq!(
            parse_http(b"HTTP/1.1 200 OK\r\nContent-Length: 22\r\n\r\nMicrosoft Connect Test"),
            Ok((200, "Microsoft Connect Test".to_string()))
        );
        assert_eq!(
            parse_http(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n9\r\nMicrosoft\r\nd\r\n Connect Test\r\n0\r\n\r\n"),
            Ok((200, "Microsoft Connect Test".to_string()))
        );
        assert_eq!(
            parse_http(b"HTTP/1.1 302 Found\r\nLocation: http://portal/\r\n\r\n").map(|r| r.0),
            Ok(302)
        );
        assert!(parse_http(b"garbage").is_err());
    }

    #[test]
    fn arp_table() {
        let f = Fixture::new();
        f.file(
            "proc/net/arp",
            "IP address       HW type     Flags       HW address            Mask     Device\n\
             192.168.1.1      0x1         0x2         d8:bb:c1:12:34:56     *        enp5s0\n\
             192.168.1.50     0x1         0x0         00:00:00:00:00:00     *        enp5s0\n",
        )
        .file("etc/resolv.conf", "nameserver 127.0.0.53\noptions edns0\n");
        assert_eq!(
            neighbours(&f.root()),
            vec![(
                Ipv4Addr::new(192, 168, 1, 1),
                [0xd8, 0xbb, 0xc1, 0x12, 0x34, 0x56]
            )]
        );
        assert_eq!(
            system_dns(&f.root()),
            IpAddr::V4(Ipv4Addr::new(127, 0, 0, 53))
        );
    }
}
