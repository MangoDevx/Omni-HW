//! Network page: every network interface with its link, addresses, Wi-Fi and driver details;
//! changes through NetworkManager (connect/disconnect, IP, DNS, MTU, reconnect, DNS cache
//! flush); and the local and internet tests. The page only reads the snapshot; changes and tests
//! are requested here and run by `dispatch` on worker threads.

mod actions;

use super::{group_box, page_title, pill};
use crate::heat::{BLUE, GREEN, ORANGE, RED};
use crate::sampler::{Snapshot, TestRun};
pub use actions::dispatch;
use actions::{Editor, INTERNET_TESTS, LOCAL_TESTS, REFRESH, action_button, edit, flag};
use egui::{Color32, RichText, Ui};
use omnihw_core::checkup::format_link_speed;
use omnihw_core::network::{
    Adapter, INTERNET_TARGETS, IpEntry, NetAction, Status, Verdict, format_lease,
};
use std::time::SystemTime;

pub fn show(ui: &mut Ui, snap: &Snapshot) {
    ui.horizontal(|ui| {
        page_title(ui, "Network");
        if snap.net_busy {
            ui.spinner();
            ui.weak("Working...");
        } else if ui
            .small_button("Refresh")
            .on_hover_text("Read the network adapters again")
            .clicked()
        {
            flag(ui.ctx(), REFRESH);
        }
    });
    let Some(n) = &snap.network else {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label("Reading network adapters...");
        });
        return;
    };
    ui.weak(
        "Details refresh every few seconds while this page is open. Changes go through NetworkManager after you confirm them; no password is needed for your own session.",
    );
    ui.add_space(8.0);
    if n.adapters.is_empty() {
        ui.label("No network adapters found.");
    }
    ui.add_enabled_ui(!snap.net_busy, |ui| {
        for a in &n.adapters {
            adapter_card(ui, a);
        }
    });
    tests(ui, snap);
    tools(ui, snap.net_busy);
}

fn status_color(s: Status) -> Color32 {
    match s {
        Status::Up => GREEN,
        Status::Disconnected => ORANGE,
        Status::Disabled | Status::Unknown => Color32::GRAY,
    }
}

fn ip_text(e: &IpEntry) -> String {
    let mut s = e.text();
    if e.duplicate {
        s.push_str("  (conflict: also used by another device)");
    } else if let Some(l) = e.lease_secs {
        s.push_str(&format!("  (DHCP, lease {})", format_lease(l)));
    }
    s
}

fn join<T: ToString>(v: &[T]) -> String {
    v.iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Label/value rows describing an adapter (empty values left out).
fn rows(a: &Adapter) -> Vec<(&'static str, String)> {
    let mut r: Vec<(&'static str, String)> = vec![
        ("Adapter", a.description.clone()),
        ("Type", {
            let kind = if a.physical { "" } else { " (virtual)" };
            format!("{}{kind}", a.media.name())
        }),
        ("MAC address", a.mac.clone()),
    ];
    if let Some(d) = &a.driver {
        let mut s = d.version.clone();
        if !d.date.is_empty() {
            s.push_str(&format!(" ({})", d.date));
        }
        if !d.provider.is_empty() {
            s.push_str(&format!(", {}", d.provider));
        }
        r.push(("Driver", s));
    }
    if a.status == Status::Up {
        let mut link = a.link_text();
        if let Some(max) = a
            .max_speed_bps
            .filter(|&m| a.speed_bps.is_some_and(|s| s < m))
        {
            link.push_str(&format!("  (adapter supports {})", format_link_speed(max)));
        }
        r.push(("Link speed", link));
    }
    if let Some(w) = &a.wifi {
        r.push((
            "Wi-Fi network",
            w.ssid
                .clone()
                .unwrap_or_else(|| "(install iw to see it)".into()),
        ));
        let mut band = w.band.map(|b| b.name().to_string()).unwrap_or_default();
        if let Some(c) = w.channel {
            band.push_str(&format!(", channel {c}"));
        }
        r.push(("Band", band.trim_start_matches(", ").to_string()));
        let mut sig = w.signal_pct.map(|s| format!("{s} %")).unwrap_or_default();
        if let Some(d) = w.rssi_dbm {
            sig.push_str(&format!(" ({d} dBm)"));
        }
        r.push(("Signal", sig.trim().to_string()));
        r.push(("Standard", w.phy.unwrap_or_default().to_string()));
        if let (Some(rx), Some(tx)) = (w.rx_mbps, w.tx_mbps) {
            r.push(("Rate", format!("{rx} Mbps down / {tx} Mbps up")));
        }
    }
    if a.status != Status::Disabled {
        r.push((
            "IPv4",
            a.ipv4.iter().map(ip_text).collect::<Vec<_>>().join("\n"),
        ));
        r.push((
            "IPv6",
            a.ipv6.iter().map(ip_text).collect::<Vec<_>>().join("\n"),
        ));
        if a.status == Status::Up {
            r.push(("Gateway", join(&a.gateways)));
            r.push(("DNS servers", join(&a.dns)));
        }
        r.push((
            "DHCP",
            if a.dhcp_enabled {
                match a.dhcp_server {
                    Some(s) => format!("On (server {s})"),
                    None => "On".into(),
                }
            } else {
                "Off (address set by hand)".into()
            },
        ));
        r.push(("MTU", a.mtu.map(|m| m.to_string()).unwrap_or_default()));
        r.push(("DNS suffix", a.dns_suffix.clone()));
    }
    r.retain(|(_, v)| !v.is_empty());
    r
}

fn adapter_card(ui: &mut Ui, a: &Adapter) {
    let title = RichText::new(&a.name).size(16.0).strong();
    ui.group(|ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(title);
            pill(ui, a.status.name(), status_color(a.status), true);
            if a.apipa_only() && a.status == Status::Up {
                pill(ui, "No DHCP address", ORANGE, true);
            }
        });
        egui::Grid::new(("net_rows", &a.name))
            .num_columns(2)
            .spacing([16.0, 3.0])
            .show(ui, |ui| {
                for (k, v) in rows(a) {
                    ui.label(k);
                    ui.add(egui::Label::new(RichText::new(v).strong()).wrap());
                    ui.end_row();
                }
            });
        ui.add_space(4.0);
        changes(ui, a);
    });
    ui.add_space(8.0);
}

fn changes(ui: &mut Ui, a: &Adapter) {
    let name = || a.name.clone();
    ui.horizontal_wrapped(|ui| {
        if a.status == Status::Up {
            action_button(
                ui,
                "Disconnect",
                NetAction::Disconnect { name: name() },
                "Deactivate this interface's connection",
            );
        } else {
            action_button(
                ui,
                "Connect",
                NetAction::Connect { name: name() },
                "Bring the interface up and connect it",
            );
            return;
        }
        if ui
            .button("IP address...")
            .on_hover_text("Get an IPv4 address automatically (DHCP) or set one by hand")
            .clicked()
        {
            let first = a
                .ipv4
                .iter()
                .find(|e| !omnihw_core::network::is_apipa(e.addr));
            let gw = a.gateways.iter().find(|g| g.is_ipv4());
            edit(
                ui.ctx(),
                Editor::Ip {
                    name: name(),
                    dhcp: a.dhcp_enabled,
                    ip: first.map(|e| e.addr.to_string()).unwrap_or_default(),
                    mask: first
                        .map(|e| omnihw_core::network::prefix_mask(e.prefix).to_string())
                        .unwrap_or_else(|| "255.255.255.0".into()),
                    gateway: gw.map(|g| g.to_string()).unwrap_or_default(),
                },
            );
        }
        if ui
            .button("DNS...")
            .on_hover_text("Get DNS servers automatically or choose your own")
            .clicked()
        {
            let v4: Vec<String> = a
                .dns
                .iter()
                .filter(|d| d.is_ipv4())
                .map(|d| d.to_string())
                .collect();
            edit(
                ui.ctx(),
                Editor::Dns {
                    name: name(),
                    auto: !a.dns_static,
                    text: v4.join(", "),
                },
            );
        }
        if ui.button("MTU...").clicked() {
            edit(
                ui.ctx(),
                Editor::Mtu {
                    name: name(),
                    text: a.mtu.unwrap_or(1500).to_string(),
                },
            );
        }
        if a.dhcp_enabled {
            action_button(
                ui,
                "Reconnect",
                NetAction::Reconnect { name: name() },
                "Activate the connection again, which renews the DHCP lease",
            );
        }
    });
}

fn verdict_pill(ui: &mut Ui, v: Verdict) {
    let (text, color) = match v {
        Verdict::Pass => ("OK", GREEN),
        Verdict::Warn => ("CHECK", ORANGE),
        Verdict::Fail => ("FAIL", RED),
        Verdict::Skip => ("-", BLUE),
    };
    pill(ui, text, color, true);
}

fn ago(t: SystemTime) -> String {
    let s = t.elapsed().map(|d| d.as_secs()).unwrap_or(0);
    if s < 60 {
        "just now".into()
    } else {
        format!("{} min ago", s / 60)
    }
}

fn results(ui: &mut Ui, id: &str, run: &TestRun) {
    let Some((groups, at)) = &run.results else {
        return;
    };
    ui.weak(format!("Last run {}", ago(*at)));
    if groups.is_empty() {
        ui.label("No connected adapters to test.");
    }
    for (i, (heading, lines)) in groups.iter().enumerate() {
        if !heading.is_empty() {
            ui.label(RichText::new(heading).strong());
        }
        egui::Grid::new(("net_results", id, i, heading))
            .num_columns(3)
            .spacing([10.0, 3.0])
            .show(ui, |ui| {
                for l in lines {
                    verdict_pill(ui, l.verdict);
                    ui.label(&l.name);
                    ui.add(egui::Label::new(&l.detail).wrap());
                    ui.end_row();
                }
            });
        ui.add_space(4.0);
    }
}

fn tests(ui: &mut Ui, snap: &Snapshot) {
    group_box(
        ui,
        RichText::new("Tests").size(16.0).strong(),
        4.0,
        8.0,
        |ui| {
            ui.horizontal(|ui| {
                let running = snap.net_local.running;
                if ui
                    .add_enabled(!running, egui::Button::new("Run local tests"))
                    .clicked()
                {
                    flag(ui.ctx(), LOCAL_TESTS);
                }
                if running {
                    ui.spinner();
                }
            });
            ui.weak(
            "Checks each connected adapter's link and address, pings its gateway and asks the DNS servers on your local network. Nothing is sent to the internet.",
        );
            results(ui, "local", &snap.net_local);
            ui.separator();
            ui.horizontal(|ui| {
                let running = snap.net_internet.running;
                if ui
                    .add_enabled(!running, egui::Button::new("Run internet tests"))
                    .clicked()
                {
                    flag(ui.ctx(), INTERNET_TESTS);
                }
                if running {
                    ui.spinner();
                    ui.weak("Takes about 5 seconds...");
                }
            });
            ui.add(
            egui::Label::new(
                RichText::new(format!(
                    "Runs only when you click. Contacts: {INTERNET_TARGETS}. Measures latency, jitter and packet loss; nothing about your PC is sent."
                ))
                .weak(),
            )
            .wrap(),
        );
            results(ui, "internet", &snap.net_internet);
        },
    );
}

fn tools(ui: &mut Ui, busy: bool) {
    group_box(
        ui,
        RichText::new("Repair tools").size(16.0).strong(),
        4.0,
        8.0,
        |ui| {
            ui.add_enabled_ui(!busy, |ui| {
                action_button(
                    ui,
                    "Flush DNS cache",
                    NetAction::FlushDns,
                    "Forget cached name lookups (resolvectl flush-caches)",
                );
            });
        },
    );
}
