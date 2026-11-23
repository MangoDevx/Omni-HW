//! Network page: every adapter Network Connections shows, with its link, addresses, Wi-Fi and
//! driver details; changes (enable/disable, rename, IP, DNS, MTU, release/renew, driver
//! properties, DNS flush and stack resets); and the local and internet tests. The page only
//! reads the snapshot; its buttons ask for changes (after an editor and a confirmation dialog)
//! and tests, which the sampler runs on worker threads.

mod actions;

use crate::app::OmniApp;
use crate::heat::{BLUE, GREEN, ORANGE, RED, rgb8};
use crate::sampler::{Snapshot, TestRun};
use crate::ui;
use actions::{Editor, action_button};
use gpui_kit::component::button::Button;
use gpui_kit::component::input::InputState;
use gpui_kit::component::separator::Separator;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme, Disableable, Sizable, h_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AnyElement, App, Context, Div, ElementId, Entity, InteractiveElement, IntoElement,
    ParentElement, Rgba, SharedString, Styled, Window, div, rems,
};
use omnihw_core::checkup::format_link_speed;
use omnihw_core::network::{
    Adapter, INTERNET_TARGETS, IpEntry, NetAction, Status, Verdict, format_lease,
};

/// Neutral grey for adapters that are off or in an unknown state.
const GRAY: Rgba = rgb8(160, 160, 160);

/// Selections, dialog state and edit buffers of this page.
#[derive(Default)]
pub struct ViewState {
    /// The local tests already ran by themselves this session.
    local_auto_ran: bool,
    /// The change being edited in the editor dialog (rename, IP, DNS, MTU). Its text fields
    /// live in `fields` while the dialog is open.
    editor: Option<Editor>,
    /// The editor dialog's text fields, created the first time a dialog opens: the new name,
    /// DNS servers or MTU use the first; the IP editor uses all three (address, mask,
    /// gateway).
    fields: Option<[Entity<InputState>; 3]>,
}

pub fn show(app: &mut OmniApp, window: &mut Window, cx: &mut Context<OmniApp>) -> AnyElement {
    let snap = app.sampler.snapshot();
    let busy = snap.net_busy;
    let mut title = ui::row().child(ui::page_title("Network"));
    if busy {
        title = title
            .child(Spinner::new().small())
            .child(ui::weak("Working...", cx));
    } else {
        title = title.child(
            Button::new("net-refresh")
                .small()
                .label("Refresh")
                .tooltip("Read the network adapters again")
                .on_click(cx.listener(|this, _, _, _| this.sampler.refresh_network())),
        );
    }
    let mut page = ui::column().child(title);
    let Some(n) = &snap.network else {
        return page
            .child(
                ui::row()
                    .child(Spinner::new().small())
                    .child(ui::label("Reading network adapters...")),
            )
            .into_any_element();
    };

    // The local tests only contact the local network, so they run once by themselves the
    // first time the page shows adapters (after this redraw, with no snapshot lock held).
    if !app.views.network.local_auto_ran {
        app.views.network.local_auto_ran = true;
        cx.defer_in(window, |this, _, _| this.sampler.run_local_tests());
    }

    page = page
        .child(ui::weak(
            "Details refresh every few seconds while this page is open. Changes ask for confirmation, and Windows asks for administrator permission.",
            cx,
        ))
        .child(ui::space(8.0));
    if n.adapters.is_empty() {
        page = page.child(ui::label("No network adapters found."));
    }
    for (i, a) in n.adapters.iter().enumerate() {
        page = page.child(adapter_card(app, i, a, busy, cx));
    }
    page.child(tests(&snap, cx))
        .child(tools(busy, cx))
        .into_any_element()
}

fn status_color(s: Status) -> Rgba {
    match s {
        Status::Up => GREEN,
        Status::Disconnected => ORANGE,
        Status::Disabled | Status::Unknown => GRAY,
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
            w.ssid.clone().unwrap_or_else(|| {
                "(hidden: turn on location access for apps in Windows Settings to see it)".into()
            }),
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

/// One adapter: its name and state, its details, the change buttons and its advanced driver
/// properties. While a change runs (`busy`) the controls are disabled.
fn adapter_card(
    app: &OmniApp,
    i: usize,
    a: &Adapter,
    busy: bool,
    cx: &mut Context<OmniApp>,
) -> impl IntoElement {
    let mut header = ui::row()
        .child(ui::strong(a.name.clone()).text_lg())
        .child(ui::pill(a.status.name(), status_color(a.status), true, cx));
    if a.apipa_only() && a.status == Status::Up {
        header = header.child(ui::pill("No DHCP address", ORANGE, true, cx));
    }
    ui::group_box(cx)
        .id(("net-card", i))
        .child(header)
        .child(ui::key_values(&rows(a)))
        .child(ui::space(4.0))
        .child(changes(i, a, busy, cx))
        .children(advanced(app, i, a, busy, cx))
}

fn changes(i: usize, a: &Adapter, busy: bool, cx: &mut Context<OmniApp>) -> Div {
    let idx = a.if_index;
    let mut row = ui::wrap_row();
    row = if a.status == Status::Disabled {
        row.child(action_button(
            ("net-enable", i),
            "Enable",
            NetAction::Enable {
                guid: a.guid.clone(),
                name: a.name.clone(),
            },
            "Turn this adapter back on",
            busy,
            cx,
        ))
    } else {
        row.child(action_button(
            ("net-disable", i),
            "Disable",
            NetAction::Disable {
                guid: a.guid.clone(),
                name: a.name.clone(),
            },
            "Turn this adapter off",
            busy,
            cx,
        ))
    };
    row = row.child(editor_button(
        ("net-rename", i),
        "Rename...",
        None,
        Editor::Rename {
            guid: a.guid.clone(),
            name: a.name.clone(),
            text: a.name.clone(),
        },
        busy,
        cx,
    ));
    if a.status == Status::Disabled || idx == 0 {
        return row;
    }
    let first = a
        .ipv4
        .iter()
        .find(|e| !omnihw_core::network::is_apipa(e.addr));
    let gw = a.gateways.iter().find(|g| g.is_ipv4());
    row = row.child(editor_button(
        ("net-ip", i),
        "IP address...",
        Some("Get an IPv4 address automatically (DHCP) or set one by hand"),
        Editor::Ip {
            if_index: idx,
            name: a.name.clone(),
            dhcp: a.dhcp_enabled,
            connected: a.status == Status::Up,
            ip: first.map(|e| e.addr.to_string()).unwrap_or_default(),
            mask: first
                .map(|e| omnihw_core::network::prefix_mask(e.prefix).to_string())
                .unwrap_or_else(|| "255.255.255.0".into()),
            gateway: gw.map(|g| g.to_string()).unwrap_or_default(),
        },
        busy,
        cx,
    ));
    let v4: Vec<String> = a
        .dns
        .iter()
        .filter(|d| d.is_ipv4())
        .map(|d| d.to_string())
        .collect();
    row = row.child(editor_button(
        ("net-dns", i),
        "DNS...",
        Some("Get DNS servers automatically or choose your own"),
        Editor::Dns {
            if_index: idx,
            name: a.name.clone(),
            auto: !a.dns_static,
            text: v4.join(", "),
        },
        busy,
        cx,
    ));
    row = row.child(editor_button(
        ("net-mtu", i),
        "MTU...",
        None,
        Editor::Mtu {
            if_index: idx,
            name: a.name.clone(),
            text: a.mtu.unwrap_or(1500).to_string(),
        },
        busy,
        cx,
    ));
    if a.dhcp_enabled && a.status == Status::Up {
        row = row
            .child(action_button(
                ("net-release", i),
                "Release",
                NetAction::Release {
                    if_index: idx,
                    name: a.name.clone(),
                },
                "Give up the DHCP address (ipconfig /release)",
                busy,
                cx,
            ))
            .child(action_button(
                ("net-renew", i),
                "Renew",
                NetAction::Renew {
                    if_index: idx,
                    name: a.name.clone(),
                },
                "Ask the DHCP server for an address again (ipconfig /renew)",
                busy,
                cx,
            ));
    }
    row
}

/// A button that opens the editor dialog for `editor`.
fn editor_button(
    id: impl Into<ElementId>,
    text: &'static str,
    hover: Option<&'static str>,
    editor: Editor,
    busy: bool,
    cx: &mut Context<OmniApp>,
) -> Button {
    Button::new(id)
        .label(text)
        .disabled(busy)
        .when_some(hover, |b, h| b.tooltip(h))
        .on_click(cx.listener(move |this, _, window, cx| {
            actions::edit(this, editor.clone(), window, cx);
        }))
}

/// The collapsible list of advanced driver properties (closed by default); `None` when the
/// adapter has none.
fn advanced(
    app: &OmniApp,
    i: usize,
    a: &Adapter,
    busy: bool,
    cx: &mut Context<OmniApp>,
) -> Option<Div> {
    if a.advanced.is_empty() {
        return None;
    }
    let key = format!("network/advanced/{}", a.guid);
    let open = ui::is_open(app, &key, false);
    let mut section = ui::column().child(ui::collapse_header(
        key,
        ui::strong(format!("Advanced driver properties ({})", a.advanced.len())),
        open,
        cx,
    ));
    if !open {
        return Some(section);
    }
    let mut cells: Vec<AnyElement> = Vec::with_capacity(a.advanced.len() * 2);
    for (j, p) in a.advanced.iter().enumerate() {
        cells.push(
            ui::with_tip(
                ("net-adv-name", j),
                ui::label(p.display_name.clone()),
                p.keyword.clone(),
            )
            .into_any_element(),
        );
        let id = ElementId::NamedInteger(format!("net-adv-{i}").into(), j as u64);
        let value = if p.choices.is_empty() {
            ui::strong(p.display_value.clone()).into_any_element()
        } else if busy {
            Button::new(id)
                .small()
                .label(p.display_value.clone())
                .dropdown_caret(true)
                .disabled(true)
                .into_any_element()
        } else {
            let options = p
                .choices
                .iter()
                .map(|(_, display)| SharedString::from(display.clone()))
                .collect();
            let selected = p.choices.iter().position(|(v, _)| *v == p.value);
            let (guid, name) = (a.guid.clone(), a.name.clone());
            let prop = p.clone();
            ui::dropdown(
                id,
                p.display_value.clone(),
                options,
                selected,
                cx,
                move |_, k, window, cx| {
                    let Some((value, display)) = prop.choices.get(k).cloned() else {
                        return;
                    };
                    if value == prop.value {
                        return;
                    }
                    actions::request(
                        NetAction::Advanced {
                            guid: guid.clone(),
                            name: name.clone(),
                            keyword: prop.keyword.clone(),
                            value,
                            display_name: prop.display_name.clone(),
                            display_value: display,
                        },
                        window,
                        cx,
                    );
                },
            )
        };
        cells.push(value);
    }
    section = section.child(
        ui::column()
            .pl_4()
            .child(ui::weak(
                "Changing one restarts the adapter, so the connection drops for a few seconds. Values without a list can be changed in Device Manager.",
                cx,
            ))
            .child(striped_grid(2, cells, cx)),
    );
    Some(section)
}

/// A grid of `cols` columns as wide as their widest cell, with every other row tinted, and no
/// headings (a [`ui::Table`] without its heading row).
fn striped_grid(cols: usize, cells: Vec<AnyElement>, cx: &App) -> Div {
    let stripe = cx.theme().muted;
    let cells = cells.into_iter().enumerate().map(move |(k, cell)| {
        div()
            .id(k)
            .px_2()
            .py_0p5()
            .when((k / cols) % 2 == 1, |c| c.bg(stripe))
            .child(cell)
    });
    ui::hug(
        div()
            .grid()
            .grid_cols_max_content(cols as u16)
            .children(cells),
    )
}

fn verdict_pill(v: Verdict, cx: &App) -> Div {
    let (text, color) = match v {
        Verdict::Pass => ("OK", GREEN),
        Verdict::Warn => ("CHECK", ORANGE),
        Verdict::Fail => ("FAIL", RED),
        Verdict::Skip => ("-", BLUE),
    };
    ui::pill(text, color, true, cx)
}

/// The results of a test run, grouped under their headings; `None` before the first run.
fn results(id: &'static str, run: &TestRun, cx: &App) -> Option<Div> {
    let (groups, at) = run.results.as_ref()?;
    let mut col = ui::column().child(ui::weak(format!("Last run {}", ui::ago(*at)), cx));
    if groups.is_empty() {
        col = col.child(ui::label("No connected adapters to test."));
    }
    for (i, (heading, lines)) in groups.iter().enumerate() {
        let mut group = ui::column().id((id, i));
        if !heading.is_empty() {
            group = group.child(ui::strong(heading.clone()));
        }
        for (j, l) in lines.iter().enumerate() {
            group = group.child(
                h_flex()
                    .id(j)
                    .items_start()
                    .gap_2()
                    .child(
                        div()
                            .flex_none()
                            .w(rems(4.5))
                            .child(verdict_pill(l.verdict, cx)),
                    )
                    .child(ui::label(l.name.clone()).flex_none().w(rems(14.0)))
                    .child(ui::label(l.detail.clone()).flex_1().min_w_0()),
            );
        }
        col = col.child(group.pb_1());
    }
    Some(col)
}

fn tests(snap: &Snapshot, cx: &mut Context<OmniApp>) -> Div {
    let local = snap.net_local.running;
    let internet = snap.net_internet.running;
    ui::group_box(cx)
        .child(ui::strong("Tests").text_lg())
        .child(
            ui::row()
                .child(
                    Button::new("net-local-tests")
                        .label("Run local tests")
                        .disabled(local)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.sampler.run_local_tests();
                            cx.notify();
                        })),
                )
                .when(local, |r| r.child(Spinner::new().small())),
        )
        .child(ui::weak(
            "Checks each connected adapter's link and address, pings its gateway and asks the DNS servers on your local network. Nothing is sent to the internet.",
            cx,
        ))
        .children(results("local", &snap.net_local, cx))
        .child(Separator::horizontal())
        .child(
            ui::row()
                .child(
                    Button::new("net-internet-tests")
                        .label("Run internet tests")
                        .disabled(internet)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.sampler.run_internet_tests();
                            cx.notify();
                        })),
                )
                .when(internet, |r| {
                    r.child(Spinner::new().small())
                        .child(ui::weak("Takes about 5 seconds...", cx))
                }),
        )
        .child(ui::weak(
            format!(
                "Runs only when you click. Contacts: {INTERNET_TARGETS}. Measures latency, jitter and packet loss; nothing about your PC is sent."
            ),
            cx,
        ))
        .children(results("internet", &snap.net_internet, cx))
}

fn tools(busy: bool, cx: &mut Context<OmniApp>) -> Div {
    ui::group_box(cx)
        .child(ui::strong("Repair tools").text_lg())
        .child(
            ui::wrap_row()
                .child(action_button(
                    "net-flush-dns",
                    "Flush DNS cache",
                    NetAction::FlushDns,
                    "Forget cached name lookups (ipconfig /flushdns)",
                    busy,
                    cx,
                ))
                .child(action_button(
                    "net-reset-winsock",
                    "Reset Winsock",
                    NetAction::ResetWinsock,
                    "netsh winsock reset. Restart required.",
                    busy,
                    cx,
                ))
                .child(action_button(
                    "net-reset-tcpip",
                    "Reset TCP/IP",
                    NetAction::ResetTcpIp,
                    "netsh int ip reset. Restart required.",
                    busy,
                    cx,
                )),
        )
        .child(ui::weak(
            "The resets fix \"connected but no internet\" problems left by VPN or security software; Windows must restart afterwards.",
            cx,
        ))
}

#[cfg(test)]
mod tests {
    use crate::app::Tab;
    use crate::sampler::Snapshot;
    use crate::settings::Settings;
    use crate::tabs::page_tests::{assert_shows, go, machines, open, page, texts};
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{AnyWindowHandle, AppContext, Entity, TestAppContext};
    use omnihw_core::network::{
        Adapter, AdvancedProperty, IpEntry, Media, NetworkInfo, Status, Wifi,
    };

    /// Two adapters: a connected Ethernet port with an advanced property, and a disabled
    /// Wi-Fi card.
    fn network() -> NetworkInfo {
        let ip = |s: &str, prefix| IpEntry {
            addr: s.parse().unwrap(),
            prefix,
            duplicate: false,
            dhcp: true,
            lease_secs: Some(86_400),
        };
        NetworkInfo {
            adapters: vec![
                Adapter {
                    guid: "{11111111-2222-3333-4444-555555555555}".into(),
                    if_index: 7,
                    name: "Ethernet".into(),
                    description: "Intel(R) Ethernet Controller I225-V".into(),
                    media: Media::Ethernet,
                    physical: true,
                    status: Status::Up,
                    mac: "D8-BB-C1-12-34-56".into(),
                    speed_bps: Some(1_000_000_000),
                    max_speed_bps: Some(2_500_000_000),
                    mtu: Some(1500),
                    ipv4: vec![ip("192.168.1.20", 24)],
                    gateways: vec!["192.168.1.1".parse().unwrap()],
                    dns: vec!["192.168.1.1".parse().unwrap()],
                    dhcp_enabled: true,
                    dhcp_server: Some("192.168.1.1".parse().unwrap()),
                    advanced: vec![AdvancedProperty {
                        keyword: "*SpeedDuplex".into(),
                        display_name: "Speed & Duplex".into(),
                        display_value: "Auto Negotiation".into(),
                        value: "0".into(),
                        choices: vec![
                            ("0".into(), "Auto Negotiation".into()),
                            ("6".into(), "1.0 Gbps Full Duplex".into()),
                        ],
                    }],
                    ..Default::default()
                },
                Adapter {
                    guid: "{66666666-7777-8888-9999-000000000000}".into(),
                    name: "Wi-Fi".into(),
                    description: "Intel(R) Wi-Fi 6E AX211 160MHz".into(),
                    media: Media::WiFi,
                    physical: true,
                    status: Status::Disabled,
                    mac: "AC-12-03-AB-CD-EF".into(),
                    wifi: Some(Wifi::default()),
                    ..Default::default()
                },
            ],
            detailed: true,
        }
    }

    /// The Network page with [`network`]'s adapters. The local tests are marked as already
    /// run, so nothing pings anything.
    fn network_page(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<crate::app::OmniApp>) {
        let info = machines()[0].clone();
        let snap = Snapshot {
            network: Some(network()),
            ..Default::default()
        };
        let (window, app) = open(cx, info, snap, Settings::default());
        cx.update(|cx| app.update(cx, |app, _| app.views.network.local_auto_ran = true));
        go(cx, &app, Tab::Network);
        (window, app)
    }

    #[gpui_kit::test]
    fn draws_without_data(cx: &mut TestAppContext) {
        for info in machines() {
            let shown = page(cx, &info, Snapshot::default(), Tab::Network);
            assert_shows(&shown, &["Network", "Reading network adapters..."]);
        }
    }

    #[gpui_kit::test]
    fn shows_adapters_and_tests(cx: &mut TestAppContext) {
        let (window, app) = network_page(cx);
        let shown = texts(cx, window);
        assert_shows(
            &shown,
            &[
                "Ethernet",
                "Wi-Fi",
                "Intel(R) Ethernet Controller I225-V",
                "Connected",
                "Disabled",
                "192.168.1.20/24",
                "On (server 192.168.1.1)",
                "adapter supports",
                "Advanced driver properties (1)",
                "Run local tests",
                "Run internet tests",
                "Flush DNS cache",
                "Rename...",
                "Enable",
                "Disable",
            ],
        );
        // Closed by default.
        assert!(!shown.iter().any(|t| t == "Speed & Duplex"));
        // Nothing was started.
        let snap_idle = cx.read(|cx| {
            let snap = app.read(cx).sampler.snapshot();
            !snap.net_local.running && !snap.net_internet.running && !snap.lan.running
        });
        assert!(snap_idle);
    }

    #[gpui_kit::test]
    fn advanced_properties_expand(cx: &mut TestAppContext) {
        let (window, _app) = network_page(cx);
        texts(cx, window);
        cx.update_window(window, |_, window, cx| {
            window.click(
                gpui_kit::ElementId::Name(
                    "network/advanced/{11111111-2222-3333-4444-555555555555}".into(),
                ),
                cx,
            );
        })
        .unwrap();
        assert_shows(
            &texts(cx, window),
            &["Speed & Duplex", "Auto Negotiation", "Device Manager"],
        );
    }

    #[gpui_kit::test]
    fn rename_dialog_opens_and_cancels(cx: &mut TestAppContext) {
        let (window, app) = network_page(cx);
        texts(cx, window);
        cx.update_window(window, |_, window, cx| {
            window.click(("net-rename", 0usize), cx)
        })
        .unwrap();
        assert_shows(&texts(cx, window), &["Rename \"Ethernet\"", "New name"]);
        assert!(cx.read(|cx| app.read(cx).views.network.editor.is_some()));
        cx.update_window(window, |_, window, cx| {
            window.click("net-editor-cancel", cx)
        })
        .unwrap();
        texts(cx, window);
        assert!(cx.read(|cx| app.read(cx).views.network.editor.is_none()));
        // Nothing was changed.
        assert!(cx.read(|cx| !app.read(cx).sampler.snapshot().net_busy));
    }
}
