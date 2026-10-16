//! Network checks: addresses (APIPA, duplicates, missing DNS) and Wi-Fi signal. They read the
//! [`NetworkInfo`](crate::network::NetworkInfo) gathered on the sampler thread; nothing here
//! sends anything over the network. Link speed lives in `system::ethernet_speed`.

use super::helpers::guided;
use super::{Category, Context, Level, Outcome, Step};
use crate::network::{Adapter, Band, Media, NetworkInfo};

const NETWORK_STATUS: &str = "ms-settings:network-status";
const WIFI_SETTINGS: &str = "ms-settings:network-wifi";

/// Signal quality below which Wi-Fi is called weak.
pub(super) const WEAK_SIGNAL_PCT: u32 = 40;

fn info<'a>(ctx: &'a Context) -> Result<&'a NetworkInfo, Outcome> {
    ctx.network
        .ok_or_else(|| Outcome::Skipped("Network adapters are still being read".into()))
}

/// Connected hardware adapters that carry traffic (not virtual switches, Bluetooth PAN or tunnels).
fn connected(n: &NetworkInfo) -> Vec<&Adapter> {
    n.connected()
        .filter(|a| a.physical && !matches!(a.media, Media::Bluetooth | Media::Tunnel))
        .collect()
}

pub(super) fn addresses(ctx: &Context) -> Outcome {
    let n = match info(ctx) {
        Ok(n) => n,
        Err(o) => return o,
    };
    let up = connected(n);
    if up.is_empty() {
        return Outcome::Skipped("No connected network adapter".into());
    }
    let mut out = Vec::new();
    for a in &up {
        // Another adapter reaching the network means this one is just a spare, not an outage.
        let other_online = up
            .iter()
            .any(|o| !std::ptr::eq(*o, *a) && o.has_usable_ipv4() && !o.gateways.is_empty());
        if a.apipa_only() && !other_online {
            let wifi = a.media == Media::WiFi;
            out.push(guided(
                Level::Warning,
                Category::Network,
                format!("No DHCP address on {} (APIPA)", a.name),
                format!(
                    "{} only has a self-assigned 169.254.x.x address: it is connected, but no DHCP server (normally the router) gave it an address, so it can't reach the network.",
                    a.name
                ),
                vec![
                    Step::new("Restart the router (unplug it for 30 seconds), then wait a minute."),
                    Step::new(if wifi {
                        "Forget the Wi-Fi network and connect again."
                    } else {
                        "Try another cable and router port."
                    }),
                    Step::new("On the Network page, click Renew for this adapter (or Get IP automatically if it was set by hand)."),
                    Step::settings("Run the Windows network troubleshooter.", NETWORK_STATUS),
                ],
            ));
        }
        let dups: Vec<String> = a
            .ipv4
            .iter()
            .chain(&a.ipv6)
            .filter(|e| e.duplicate)
            .map(|e| e.addr.to_string())
            .collect();
        if !dups.is_empty() {
            out.push(guided(
                Level::Warning,
                Category::Network,
                format!("IP address conflict on {}", a.name),
                format!(
                    "Another device on the network uses {}, so this PC's connection is unreliable or down.",
                    dups.join(", ")
                ),
                vec![
                    Step::new("If you set this address by hand, switch the adapter back to Get IP automatically on the Network page, or pick a free address."),
                    Step::new("Otherwise restart the router so it hands out addresses again."),
                ],
            ));
        }
        if a.has_usable_ipv4() && !a.gateways.is_empty() && a.dns.is_empty() {
            out.push(guided(
                Level::Warning,
                Category::Network,
                format!("No DNS servers on {}", a.name),
                "Without a DNS server, websites and games can't be found by name even though the connection works.".to_string(),
                vec![
                    Step::new("On the Network page, set DNS to automatic, or enter your router's address or a public DNS server."),
                    Step::settings("Or in Settings > Network & internet > the adapter > DNS server assignment.", NETWORK_STATUS),
                ],
            ));
        }
    }
    if out.is_empty() {
        return Outcome::Pass(
            up.iter()
                .map(|a| {
                    let ip = a
                        .ipv4
                        .first()
                        .map(|e| e.addr.to_string())
                        .unwrap_or_else(|| "IPv6 only".into());
                    format!("{}: {ip}", a.name)
                })
                .collect::<Vec<_>>()
                .join("; "),
        );
    }
    Outcome::Findings(out)
}

pub(super) fn wifi_signal(ctx: &Context) -> Outcome {
    let n = match info(ctx) {
        Ok(n) => n,
        Err(o) => return o,
    };
    let wifi: Vec<_> = n
        .connected()
        .filter_map(|a| Some((a, a.wifi.as_ref()?)))
        .collect();
    if wifi.is_empty() {
        return Outcome::Skipped("Not connected to Wi-Fi".into());
    }
    let mut out = Vec::new();
    let mut seen = Vec::new();
    for (a, w) in wifi {
        let Some(signal) = w.signal_pct else {
            continue;
        };
        let band = w.band.map(Band::name).unwrap_or("unknown band");
        seen.push(format!("{}: {signal} % signal on {band}", a.name));
        if signal >= WEAK_SIGNAL_PCT {
            continue;
        }
        let on_24 = w.band == Some(Band::G2_4);
        let title = if on_24 {
            format!("Wi-Fi on 2.4 GHz with a weak signal ({signal} %)")
        } else {
            format!("Weak Wi-Fi signal ({signal} %)")
        };
        let mut steps = vec![Step::new(
            "Move the PC or router so fewer walls and floors are in between, and keep the router up high and in the open.",
        )];
        if on_24 {
            steps.push(Step::settings(
                "2.4 GHz is slow and crowded: if your router also offers 5 GHz (often a second network name ending in 5G), connect to that when you're close enough.",
                WIFI_SETTINGS,
            ));
        }
        steps.push(Step::new(
            "For gaming or video calls, use an Ethernet cable, a mesh system or a powerline adapter.",
        ));
        steps.push(Step::search(
            "Update the Wi-Fi adapter's driver.",
            format!("{} driver download", a.description),
        ));
        out.push(guided(
            Level::Warning,
            Category::Network,
            title,
            format!(
                "{} is connected{} with {signal} % signal quality on {band}. A weak signal means low speed, lag spikes and drop-outs.",
                a.name,
                w.ssid
                    .as_ref()
                    .map(|s| format!(" to \"{s}\""))
                    .unwrap_or_default()
            ),
            steps,
        ));
    }
    if !out.is_empty() {
        Outcome::Findings(out)
    } else if seen.is_empty() {
        Outcome::Skipped("Wi-Fi signal not available".into())
    } else {
        Outcome::Pass(seen.join("; "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SystemInfo;
    use crate::diagnostics::testutil::{ctx, levels};
    use crate::network::{IpEntry, Status, Wifi};
    use crate::sensors::SensorTable;

    fn adapter(ip: &str) -> Adapter {
        Adapter {
            name: "Ethernet".into(),
            media: Media::Ethernet,
            physical: true,
            status: Status::Up,
            ipv4: vec![IpEntry {
                addr: ip.parse().unwrap(),
                prefix: 24,
                duplicate: false,
                dhcp: true,
                lease_secs: None,
            }],
            gateways: vec!["192.168.1.1".parse().unwrap()],
            dns: vec!["192.168.1.1".parse().unwrap()],
            ..Default::default()
        }
    }

    fn run(check: fn(&Context) -> Outcome, n: &NetworkInfo) -> Outcome {
        let info = SystemInfo::default();
        let s = SensorTable::default();
        let mut c = ctx(&info, &s);
        c.network = Some(n);
        check(&c)
    }

    #[test]
    fn skipped_until_read() {
        let info = SystemInfo::default();
        let s = SensorTable::default();
        assert!(matches!(addresses(&ctx(&info, &s)), Outcome::Skipped(_)));
        assert!(matches!(wifi_signal(&ctx(&info, &s)), Outcome::Skipped(_)));
        let none = NetworkInfo::default();
        assert!(matches!(run(addresses, &none), Outcome::Skipped(_)));
    }

    #[test]
    fn address_problems() {
        let good = NetworkInfo {
            adapters: vec![adapter("192.168.1.20")],
            detailed: true,
        };
        assert!(matches!(run(addresses, &good), Outcome::Pass(_)));

        let mut apipa = adapter("169.254.7.8");
        apipa.ipv4[0].duplicate = true;
        let mut down = adapter("169.254.1.1");
        down.status = Status::Disconnected;
        let bad = NetworkInfo {
            adapters: vec![apipa, down],
            detailed: true,
        };
        let o = run(addresses, &bad);
        assert_eq!(levels(&o), vec![Level::Warning; 2]);
        let Outcome::Findings(f) = o else {
            unreachable!()
        };
        assert!(f[0].title.contains("APIPA"));
        assert!(f.iter().all(|f| !f.steps.is_empty()));

        let mut no_dns = adapter("192.168.1.30");
        no_dns.name = "Ethernet 2".into();
        no_dns.dns.clear();
        let o = run(
            addresses,
            &NetworkInfo {
                adapters: vec![no_dns],
                detailed: true,
            },
        );
        assert_eq!(levels(&o), vec![Level::Warning]);
    }

    #[test]
    fn apipa_only_raised_when_nothing_else_is_online() {
        let net = |adapters| NetworkInfo {
            adapters,
            detailed: true,
        };
        let mut virt = adapter("169.254.7.8");
        virt.physical = false;
        // A virtual adapter is not even considered.
        assert!(matches!(
            run(addresses, &net(vec![virt])),
            Outcome::Skipped(_)
        ));
        // Another physical adapter is online: the APIPA one is a spare.
        let mut spare = adapter("169.254.7.8");
        spare.name = "Ethernet 2".into();
        let online = adapter("192.168.1.20");
        assert!(matches!(
            run(addresses, &net(vec![spare.clone(), online])),
            Outcome::Pass(_)
        ));
        // Alone, it is a real problem.
        let o = run(addresses, &net(vec![spare]));
        assert_eq!(levels(&o), vec![Level::Warning]);
    }

    #[test]
    fn weak_wifi() {
        let mut a = adapter("192.168.1.20");
        a.media = Media::WiFi;
        a.wifi = Some(Wifi {
            signal_pct: Some(25),
            band: Some(Band::G2_4),
            ..Default::default()
        });
        let mut n = NetworkInfo {
            adapters: vec![a],
            detailed: true,
        };
        let o = run(wifi_signal, &n);
        let Outcome::Findings(f) = &o else {
            panic!("{o:?}")
        };
        assert!(f[0].title.contains("2.4 GHz"));
        assert!(f[0].steps.iter().any(|s| s.link.is_some()));
        if let Some(w) = &mut n.adapters[0].wifi {
            w.band = Some(Band::G5);
        }
        let Outcome::Findings(f) = run(wifi_signal, &n) else {
            panic!()
        };
        assert!(!f[0].title.contains("2.4"));
        if let Some(w) = &mut n.adapters[0].wifi {
            w.signal_pct = Some(80);
        }
        assert!(matches!(run(wifi_signal, &n), Outcome::Pass(_)));
        n.adapters[0].wifi = None;
        assert!(matches!(run(wifi_signal, &n), Outcome::Skipped(_)));
    }
}
