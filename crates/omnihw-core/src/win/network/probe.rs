//! The network tests: ICMP ping (`IcmpSendEcho2` / `Icmp6SendEcho2`), DNS queries
//! (`DnsQuery_W`) and one plain HTTP GET (WinHTTP). These are the only functions in Omni-HW
//! besides the update check that send packets, and they only run when the user starts a test
//! (see `crate::network::probe_local` and `crate::network::probe_internet`).

use crate::network::PingStats;
use std::net::{IpAddr, Ipv4Addr};
use std::time::Duration;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::NetworkManagement::Dns::{
    DNS_QUERY_BYPASS_CACHE, DNS_QUERY_NO_HOSTS_FILE, DNS_RECORDA, DNS_TYPE, DNS_TYPE_A,
    DNS_TYPE_PTR, DnsFree, DnsFreeRecordList, DnsQuery_W, IP4_ARRAY,
};
use windows::Win32::NetworkManagement::IpHelper::{
    FreeMibTable, GetIpNetTable2, ICMP_ECHO_REPLY, ICMPV6_ECHO_REPLY_LH, Icmp6CreateFile,
    Icmp6ParseReplies, Icmp6SendEcho2, IcmpCloseHandle, IcmpCreateFile, IcmpSendEcho2,
    MIB_IPNET_TABLE2,
};
use windows::Win32::Networking::WinSock::{AF_INET, AF_INET6, SOCKADDR_IN6};
use windows::core::HSTRING;

const PING_TIMEOUT_MS: u32 = 1000;
const PAYLOAD: &[u8; 32] = b"abcdefghijklmnopqrstuvwabcdefghi";

/// An ICMP handle, closed on drop.
struct Icmp(HANDLE);

impl Drop for Icmp {
    fn drop(&mut self) {
        // SAFETY: a handle from IcmpCreateFile/Icmp6CreateFile, closed exactly once.
        let _ = unsafe { IcmpCloseHandle(self.0) };
    }
}

fn sockaddr6(ip: std::net::Ipv6Addr, scope: u32) -> SOCKADDR_IN6 {
    let mut s = SOCKADDR_IN6 {
        sin6_family: AF_INET6,
        ..Default::default()
    };
    s.sin6_addr.u.Byte = ip.octets();
    // Link-local addresses need the interface they're reached through.
    if ip.segments()[0] & 0xffc0 == 0xfe80 {
        s.Anonymous.sin6_scope_id = scope;
    }
    s
}

/// One echo request; the round-trip time in ms if a reply came back.
fn ping_once(h: &Icmp, ip: IpAddr, scope: u32) -> Option<u32> {
    // Room for the reply struct, the echoed payload and an ICMP error message.
    let mut reply = [0u64; 64];
    let size = size_of_val(&reply) as u32;
    match ip {
        IpAddr::V4(v4) => {
            // SAFETY: valid handle; the payload and reply buffers outlive the synchronous call
            // (no event or APC routine) and their sizes are passed.
            let n = unsafe {
                IcmpSendEcho2(
                    h.0,
                    None,
                    None,
                    None,
                    u32::from_ne_bytes(v4.octets()),
                    PAYLOAD.as_ptr().cast(),
                    PAYLOAD.len() as u16,
                    None,
                    reply.as_mut_ptr().cast(),
                    size,
                    PING_TIMEOUT_MS,
                )
            };
            if n == 0 {
                return None;
            }
            // SAFETY: on success the buffer starts with an ICMP_ECHO_REPLY (8-byte aligned).
            let r = unsafe { &*(reply.as_ptr() as *const ICMP_ECHO_REPLY) };
            (r.Status == 0).then_some(r.RoundTripTime)
        }
        IpAddr::V6(v6) => {
            let src = sockaddr6(std::net::Ipv6Addr::UNSPECIFIED, 0);
            let dst = sockaddr6(v6, scope);
            // SAFETY: as above, plus valid source and destination socket addresses.
            unsafe {
                Icmp6SendEcho2(
                    h.0,
                    None,
                    None,
                    None,
                    &src,
                    &dst,
                    PAYLOAD.as_ptr().cast(),
                    PAYLOAD.len() as u16,
                    None,
                    reply.as_mut_ptr().cast(),
                    size,
                    PING_TIMEOUT_MS,
                )
            };
            // SAFETY: parses the reply buffer filled above.
            if unsafe { Icmp6ParseReplies(reply.as_mut_ptr().cast(), size) } == 0 {
                return None;
            }
            // SAFETY: after a parsed reply the buffer starts with an ICMPV6_ECHO_REPLY.
            let r = unsafe { &*(reply.as_ptr() as *const ICMPV6_ECHO_REPLY_LH) };
            (r.Status == 0).then_some(r.RoundTripTime)
        }
    }
}

/// Ping `ip` `count` times, `gap_ms` apart. `scope` is the interface index used for IPv6
/// link-local addresses.
pub fn ping_series(ip: IpAddr, scope: u32, count: u32, gap_ms: u64) -> PingStats {
    let mut stats = PingStats::default();
    // SAFETY: plain handle creation; owned by `Icmp`.
    let handle = match ip {
        IpAddr::V4(_) => unsafe { IcmpCreateFile() },
        IpAddr::V6(_) => unsafe { Icmp6CreateFile() },
    };
    let Ok(h) = handle.map(Icmp) else {
        stats.sent = count;
        return stats;
    };
    for i in 0..count {
        if i > 0 {
            std::thread::sleep(Duration::from_millis(gap_ms));
        }
        stats.sent += 1;
        if let Some(rtt) = ping_once(&h, ip, scope) {
            stats.rtts.push(rtt);
        }
    }
    stats
}

/// A DNS record list, freed on drop.
struct Records(*mut DNS_RECORDA);

impl Drop for Records {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: a record list from DnsQuery_W, freed exactly once.
            unsafe { DnsFree(Some(self.0 as *const _), DnsFreeRecordList) };
        }
    }
}

/// Run a query (optionally against one IPv4 server); the Win32 status and the records.
fn dns_query(name: &str, ty: DNS_TYPE, server: Option<Ipv4Addr>) -> (u32, Records) {
    let name = HSTRING::from(name);
    let mut servers = IP4_ARRAY {
        AddrCount: 1,
        AddrArray: [server.map(|s| u32::from_ne_bytes(s.octets())).unwrap_or(0)],
    };
    let extra = server.map(|_| (&raw mut servers).cast::<core::ffi::c_void>());
    let mut out: *mut DNS_RECORDA = std::ptr::null_mut();
    // SAFETY: NUL-terminated name and (optional) server list outlive the call; the results
    // are owned by `Records`.
    let r = unsafe {
        DnsQuery_W(
            &name,
            ty,
            DNS_QUERY_BYPASS_CACHE | DNS_QUERY_NO_HOSTS_FILE,
            extra,
            &mut out,
            None,
        )
    };
    (r.0, Records(out))
}

/// Does this (local) DNS server answer? Asks it for the reverse name of its own address, which
/// local servers answer themselves; any answer (even "no such name") counts. IPv6 servers are
/// pinged instead.
pub fn dns_server_answers(server: IpAddr, scope: u32) -> bool {
    match server {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            let name = format!("{}.{}.{}.{}.in-addr.arpa", o[3], o[2], o[1], o[0]);
            let (status, _) = dns_query(&name, DNS_TYPE_PTR, Some(v4));
            // 0 = answer, 9001-9010 = DNS error codes from the server, 9501 = no records.
            status == 0 || (9001..=9010).contains(&status) || status == 9501
        }
        IpAddr::V6(_) => ping_series(server, scope, 2, 200).received() > 0,
    }
}

/// Reverse-DNS (PTR) name for `ip`, from the configured DNS servers. Used by the LAN Security
/// Report to label devices; `None` when nothing answers.
pub fn reverse_dns(ip: Ipv4Addr) -> Option<String> {
    let o = ip.octets();
    let name = format!("{}.{}.{}.{}.in-addr.arpa", o[3], o[2], o[1], o[0]);
    let (status, recs) = dns_query(&name, DNS_TYPE_PTR, None);
    if status != 0 {
        return None;
    }
    let mut p = recs.0;
    // SAFETY: a record list owned by `recs`; PTR records carry an ANSI host name in pNameHost.
    unsafe {
        while !p.is_null() {
            if (*p).wType == DNS_TYPE_PTR.0 && !(*p).Data.PTR.pNameHost.is_null() {
                return (*p)
                    .Data
                    .PTR
                    .pNameHost
                    .to_string()
                    .ok()
                    .filter(|s| !s.is_empty());
            }
            p = (*p).pNext;
        }
    }
    None
}

/// The IPv4 neighbour (ARP) table: (address, 6-byte MAC) for each reachable entry. Used by the
/// LAN Security Report to attach a hardware address to each discovered device.
pub fn neighbours() -> Vec<(Ipv4Addr, [u8; 6])> {
    let mut table: *mut MIB_IPNET_TABLE2 = std::ptr::null_mut();
    // SAFETY: GetIpNetTable2 allocates the table; it's freed by FreeMibTable below.
    let r = unsafe { GetIpNetTable2(AF_INET, &mut table) };
    if r.is_err() || table.is_null() {
        return Vec::new();
    }
    let mut out = Vec::new();
    // SAFETY: on success `table` points to a MIB_IPNET_TABLE2 with `NumEntries` rows.
    unsafe {
        let n = (*table).NumEntries as usize;
        let rows = (*table).Table.as_ptr();
        for i in 0..n {
            let row = &*rows.add(i);
            if row.Address.si_family != AF_INET {
                continue;
            }
            let addr = Ipv4Addr::from(row.Address.Ipv4.sin_addr.S_un.S_addr.to_ne_bytes());
            if row.PhysicalAddressLength < 6 {
                continue;
            }
            let mut mac = [0u8; 6];
            mac.copy_from_slice(&row.PhysicalAddress[..6]);
            // Skip all-zero or broadcast MACs.
            if mac != [0; 6] && mac != [0xff; 6] {
                out.push((addr, mac));
            }
        }
        FreeMibTable(table as *const _);
    }
    out
}

/// IPv4 addresses of `name` from the configured DNS servers (cache bypassed).
pub fn resolve_a(name: &str) -> Result<Vec<Ipv4Addr>, String> {
    let (status, recs) = dns_query(name, DNS_TYPE_A, None);
    if status != 0 {
        return Err(match status {
            1460 => "timed out".into(),
            9003 => "name not found".into(),
            s => {
                windows::core::Error::from_hresult(windows::core::HRESULT::from_win32(s)).message()
            }
        });
    }
    let mut out = Vec::new();
    let mut p = recs.0;
    while !p.is_null() {
        // SAFETY: a node of the list owned by `recs`; for A records the data union holds an
        // IPv4 address in network byte order.
        unsafe {
            if (*p).wType == DNS_TYPE_A.0 {
                out.push(Ipv4Addr::from((*p).Data.A.IpAddress.to_ne_bytes()));
            }
            p = (*p).pNext;
        }
    }
    Ok(out)
}

/// Plain-HTTP GET of `http://host/path`: (status code, first 4 KB of the body).
pub fn http_get(host: &str, path: &str) -> Result<(u16, String), String> {
    use windows::Win32::Networking::WinHttp::*;
    use windows::core::{PCWSTR, w};

    /// Closes a WinHTTP handle on drop.
    struct Handle(*mut core::ffi::c_void);
    impl Drop for Handle {
        fn drop(&mut self) {
            // SAFETY: a valid handle from WinHttpOpen/Connect/OpenRequest, closed once.
            let _ = unsafe { WinHttpCloseHandle(self.0) };
        }
    }
    fn handle(h: *mut core::ffi::c_void) -> Result<Handle, String> {
        if h.is_null() {
            Err(windows::core::Error::from_thread().message())
        } else {
            Ok(Handle(h))
        }
    }
    let err = |e: windows::core::Error| e.message();
    let agent = HSTRING::from(format!("Omni-HW/{}", env!("CARGO_PKG_VERSION")));
    let (host, path) = (HSTRING::from(host), HSTRING::from(path));
    // SAFETY: the strings are NUL-terminated and outlive the calls; handles are checked for
    // null and closed by `Handle` in reverse order; the read buffer's length is passed.
    unsafe {
        let session = handle(WinHttpOpen(
            &agent,
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            PCWSTR::null(),
            PCWSTR::null(),
            0,
        ))?;
        WinHttpSetTimeouts(session.0, 5_000, 5_000, 5_000, 5_000).map_err(err)?;
        let connect = handle(WinHttpConnect(
            session.0,
            &host,
            INTERNET_DEFAULT_HTTP_PORT,
            0,
        ))?;
        let request = handle(WinHttpOpenRequest(
            connect.0,
            w!("GET"),
            &path,
            PCWSTR::null(),
            PCWSTR::null(),
            std::ptr::null(),
            WINHTTP_OPEN_REQUEST_FLAGS(0),
        ))?;
        // A redirect means a captive portal: report it rather than follow it.
        WinHttpSetOption(
            Some(request.0),
            WINHTTP_OPTION_DISABLE_FEATURE,
            Some(&WINHTTP_DISABLE_REDIRECTS.to_ne_bytes()),
        )
        .map_err(err)?;
        WinHttpSendRequest(request.0, None, None, 0, 0, 0).map_err(err)?;
        WinHttpReceiveResponse(request.0, std::ptr::null_mut()).map_err(err)?;
        let mut code = 0u32;
        let mut len = size_of::<u32>() as u32;
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some((&raw mut code).cast()),
            &mut len,
            std::ptr::null_mut(),
        )
        .map_err(err)?;
        let mut body = Vec::new();
        let mut buf = [0u8; 1024];
        while body.len() < 4096 {
            let mut read = 0u32;
            WinHttpReadData(
                request.0,
                buf.as_mut_ptr().cast(),
                buf.len() as u32,
                &mut read,
            )
            .map_err(err)?;
            if read == 0 {
                break;
            }
            body.extend_from_slice(&buf[..read as usize]);
        }
        Ok((
            u16::try_from(code).unwrap_or(0),
            String::from_utf8_lossy(&body).into_owned(),
        ))
    }
}
