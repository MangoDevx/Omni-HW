//! LAN Security Report page: scans the connected adapters' own private subnets for devices and
//! common weaknesses, then lists ranked findings and a device inventory. The page only reads
//! the snapshot and asks for a scan; the scan runs on the sampler's worker thread (see
//! `Sampler::run_lan_scan`). It stays on the local network and changes nothing on any device —
//! each finding explains the issue and links to fix steps the user opens in their own browser.

use crate::app::{OmniApp, Tab};
use crate::heat::{BLUE, GREEN, ORANGE, RED, rgb8};
use crate::sampler::Snapshot;
use crate::ui;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::link::Link;
use gpui_kit::component::progress::Progress;
use gpui_kit::component::separator::Separator;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme, Disableable, Sizable, h_flex};
use gpui_kit::{
    AnyElement, App, Context, Div, FontWeight, InteractiveElement, IntoElement, ParentElement,
    Rgba, Styled, Window, rems,
};
use omnihw_core::lansec::{Finding, Host, LanScan, Phase, Risk, Severity, SubnetScan};
use std::time::Duration;

/// Neutral grey for open ports that are normal to see.
const GRAY: Rgba = rgb8(160, 160, 160);
/// How often the progress bar is redrawn while a scan runs.
const PROGRESS_EVERY: Duration = Duration::from_millis(200);

/// What this page remembers between redraws. Nothing yet: which device rows are open is kept
/// with the other collapsible sections (see [`ui::is_open`]).
#[derive(Default)]
pub struct ViewState {}

pub fn show(app: &mut OmniApp, _window: &mut Window, cx: &mut Context<OmniApp>) -> AnyElement {
    let mut page = ui::column()
        .child(ui::page_title("LAN Security Report"))
        .child(ui::weak(
            "Scans your own local network for devices and common security problems — \
             clear-text logins, exposed admin pages, unauthenticated databases and more. \
             It only looks (no passwords are tried and nothing is changed), stays on your \
             local network, and runs only when you click.",
            cx,
        ))
        .child(ui::space(8.0));

    let snap = app.sampler.snapshot();
    let Some(net) = &snap.network else {
        return page
            .child(
                ui::row()
                    .child(Spinner::new().small())
                    .child(ui::label("Reading network adapters...")),
            )
            .into_any_element();
    };
    let scannable = net
        .adapters
        .iter()
        .any(|a| a.status == omnihw_core::network::Status::Up);

    let mut controls = ui::row();
    if snap.lan.running {
        controls = controls
            .child(Spinner::new())
            .child(ui::label("Scanning..."));
    } else {
        controls = controls.child(
            Button::new("lan-scan")
                .primary()
                .label("Scan my network")
                .disabled(!scannable)
                .tooltip(if scannable {
                    "Discover devices and probe common ports on your local subnet"
                } else {
                    "Connect to a network first"
                })
                .on_click(cx.listener(|this, _, _, cx| {
                    this.sampler.run_lan_scan();
                    follow_progress(cx);
                    cx.notify();
                })),
        );
    }
    if let Some((_, at)) = &snap.lan.results {
        controls = controls.child(ui::weak(format!("Last run {}", ui::ago(*at)), cx));
    }
    page = page.child(controls);

    if snap.lan.running {
        page = page.child(progress(&snap));
    }

    if let Some((scan, _)) = &snap.lan.results {
        page = page
            .child(ui::space(8.0))
            .child(summary(scan, cx))
            .child(findings(&scan.findings, cx))
            .children(devices(app, &scan.subnets, cx));
    } else if !snap.lan.running {
        page = page.child(ui::space(8.0)).child(ui::weak(
            "No scan yet. A scan of a typical home network takes well under a minute.",
            cx,
        ));
    }
    page.into_any_element()
}

/// Keep the progress bar moving while the scan's worker thread runs: the scan only wakes the
/// UI when it finishes, so redraw the page every [`PROGRESS_EVERY`] until then. Started by the
/// Scan button; it stops by itself when the scan is done.
fn follow_progress(cx: &mut Context<OmniApp>) {
    cx.spawn(async move |this, cx| {
        loop {
            cx.background_executor().timer(PROGRESS_EVERY).await;
            let running = this.update(cx, |this, cx| {
                let running = this.sampler.snapshot().lan.running;
                if this.tab == Tab::LanSecurity {
                    cx.notify();
                }
                running
            });
            if !matches!(running, Ok(true)) {
                break;
            }
        }
    })
    .detach();
}

fn progress(snap: &Snapshot) -> Div {
    let (phase, done, total) = snap.lan.progress.read();
    let (label, frac) = match phase {
        Phase::Discovering => ("Finding devices", ratio(done, total)),
        Phase::Scanning => ("Checking ports", ratio(done, total)),
        Phase::Done => ("Finishing", 1.0),
    };
    ui::row()
        .child(
            Progress::new("lan-progress")
                .value(frac * 100.0)
                .w(rems(22.5)),
        )
        .child(ui::label(format!("{label} ({done}/{total})")))
}

fn ratio(done: u32, total: u32) -> f32 {
    if total == 0 {
        0.0
    } else {
        (done as f32 / total as f32).clamp(0.0, 1.0)
    }
}

/// A group box with a large bold title.
#[track_caller]
fn titled_box(title: impl Into<gpui_kit::SharedString>, cx: &App) -> Div {
    ui::group_box(cx).child(ui::strong(title).text_lg())
}

fn summary(scan: &LanScan, cx: &App) -> Div {
    let (high, warn, info) = scan.counts();
    let mut row = ui::wrap_row().child(ui::label(format!("{} devices found.", scan.total_hosts())));
    if high == 0 && warn == 0 {
        row = row.child(ui::pill("No problems found", GREEN, false, cx));
    } else {
        if high > 0 {
            row = row.child(ui::pill(format!("{high} risk"), RED, false, cx));
        }
        if warn > 0 {
            row = row.child(ui::pill(format!("{warn} to check"), ORANGE, false, cx));
        }
    }
    if info > 0 {
        row = row.child(ui::pill(ui::plural(info, "note"), BLUE, false, cx));
    }
    titled_box("Summary", cx).child(row)
}

fn severity_color(s: Severity) -> Rgba {
    match s {
        Severity::High => RED,
        Severity::Warn => ORANGE,
        Severity::Info => BLUE,
    }
}

fn findings(findings: &[Finding], cx: &App) -> Div {
    let mut b = titled_box("Findings", cx);
    if findings.is_empty() {
        return b.child(ui::label(
            "Nothing stood out. Your devices aren't exposing the risky services this check looks for.",
        ));
    }
    for (i, f) in findings.iter().enumerate() {
        let mut item = ui::column().id(("finding", i));
        if i > 0 {
            item = item.child(Separator::horizontal());
        }
        let mut head = ui::wrap_row()
            .child(ui::pill(
                f.severity.name(),
                severity_color(f.severity),
                true,
                cx,
            ))
            .child(ui::strong(f.title.clone()));
        if !f.host.is_empty() {
            head = head.child(ui::weak(format!("— {}", f.host), cx));
        }
        item = item.child(head).child(ui::label(f.detail.clone()));
        if !f.help_url.is_empty() {
            // Opens in the user's own browser; the app itself makes no request.
            item = item.child(
                Link::new(("fix-link", i))
                    .href(f.help_url)
                    .child(ui::label("How to fix this").text_color(cx.theme().link)),
            );
        }
        b = b.child(item.pb_0p5());
    }
    b
}

fn risk_color(r: Risk) -> Rgba {
    match r {
        Risk::High => RED,
        Risk::Medium => ORANGE,
        Risk::Low => BLUE,
        Risk::Info => GRAY,
    }
}

fn devices(app: &OmniApp, subnets: &[SubnetScan], cx: &mut Context<OmniApp>) -> Vec<AnyElement> {
    subnets
        .iter()
        .enumerate()
        .map(|(s, subnet)| {
            let mut b = titled_box(
                format!("Devices on {} ({})", subnet.adapter, subnet.cidr),
                cx,
            )
            .id(("subnet", s));
            if subnet.hosts.is_empty() {
                b = b.child(ui::label("No devices answered on this network."));
            } else {
                for (h, host) in subnet.hosts.iter().enumerate() {
                    b = b.child(host_row(app, &subnet.cidr, h, host, cx));
                }
            }
            b.into_any_element()
        })
        .collect()
}

/// A device: a collapsible heading (address, role, name, maker, open ports) over its MAC
/// address and open ports.
fn host_row(
    app: &OmniApp,
    cidr: &str,
    h: usize,
    host: &Host,
    cx: &mut Context<OmniApp>,
) -> impl IntoElement {
    let key = format!("lansec/host/{cidr}/{}", host.ip);
    let open = ui::is_open(app, &key, false);
    let open_ports = host.ports.len();
    let mut title =
        ui::row().child(ui::mono(host.ip.to_string(), cx).font_weight(FontWeight::SEMIBOLD));
    if host.is_gateway {
        title = title.child(ui::pill("Router", GREEN, true, cx));
    }
    if host.is_self {
        title = title.child(ui::pill("This PC", BLUE, true, cx));
    }
    if !host.hostname.is_empty() {
        title = title.child(ui::label(host.hostname.clone()));
    }
    if !host.vendor.is_empty() {
        title = title.child(ui::weak(host.vendor.clone(), cx));
    }
    if open_ports == 0 {
        title = title.child(ui::weak("no open ports", cx));
    } else {
        let worst = host
            .ports
            .iter()
            .map(|p| p.risk)
            .max()
            .unwrap_or(Risk::Info);
        title = title.child(ui::pill(
            ui::plural(open_ports, "open port"),
            risk_color(worst),
            true,
            cx,
        ));
    }
    let mut row = ui::column()
        .id(("host", h))
        .gap_0()
        .child(ui::collapse_header(key, title, open, cx));
    if open {
        let mut body = ui::column().pl_4().pb_1();
        if !host.mac.is_empty() {
            body = body.child(detail_row(
                "mac",
                ui::label("MAC address"),
                ui::mono(host.mac.clone(), cx),
            ));
        }
        if host.ports.is_empty() {
            body = body.child(detail_row(
                "none",
                ui::label("Open ports"),
                ui::label("none"),
            ));
        }
        for (p, port) in host.ports.iter().enumerate() {
            body = body.child(detail_row(
                ("port", p),
                ui::row()
                    .gap_1()
                    .child(ui::pill(
                        port.port.to_string(),
                        risk_color(port.risk),
                        true,
                        cx,
                    ))
                    .child(ui::label(port.service)),
                ui::label(if port.banner.is_empty() {
                    "open".to_string()
                } else {
                    port.banner.clone()
                }),
            ));
        }
        row = row.child(body);
    }
    row
}

/// A label/value row with the label in a fixed-width column and the value wrapping.
fn detail_row(
    id: impl Into<gpui_kit::ElementId>,
    left: impl IntoElement,
    right: impl IntoElement,
) -> impl IntoElement {
    h_flex()
        .id(id)
        .items_start()
        .gap_4()
        .child(div_fixed(left))
        .child(gpui_kit::div().flex_1().min_w_0().child(right))
}

fn div_fixed(child: impl IntoElement) -> Div {
    gpui_kit::div().flex_none().w(rems(12.5)).child(child)
}

#[cfg(test)]
mod tests {
    use crate::app::Tab;
    use crate::sampler::{LanScanState, Snapshot};
    use crate::tabs::page_tests::{assert_shows, go, machines, open, page, texts};
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{AppContext, TestAppContext};
    use omnihw_core::lansec::{Finding, Host, LanScan, OpenPort, Risk, Severity, SubnetScan};
    use omnihw_core::network::{Adapter, NetworkInfo, Status};
    use std::time::SystemTime;

    #[test]
    fn progress_ratio() {
        assert_eq!(super::ratio(0, 0), 0.0);
        assert_eq!(super::ratio(5, 10), 0.5);
        assert_eq!(super::ratio(12, 10), 1.0);
    }

    #[gpui_kit::test]
    fn draws_without_data(cx: &mut TestAppContext) {
        for info in machines() {
            let shown = page(cx, &info, Snapshot::default(), Tab::LanSecurity);
            assert_shows(
                &shown,
                &["LAN Security Report", "Reading network adapters..."],
            );
        }
    }

    fn scanned() -> Snapshot {
        let host = |ip: &str, gateway: bool| Host {
            ip: ip.parse().unwrap(),
            mac: "D8-BB-C1-12-34-56".into(),
            vendor: "ASUSTek".into(),
            hostname: if gateway {
                "router.lan".into()
            } else {
                String::new()
            },
            is_self: false,
            is_gateway: gateway,
            ports: if gateway {
                vec![OpenPort {
                    port: 23,
                    service: "Telnet",
                    risk: Risk::High,
                    banner: "BusyBox telnetd".into(),
                }]
            } else {
                Vec::new()
            },
        };
        Snapshot {
            network: Some(NetworkInfo {
                adapters: vec![Adapter {
                    name: "Ethernet".into(),
                    status: Status::Up,
                    ..Default::default()
                }],
                detailed: true,
            }),
            lan: LanScanState {
                running: false,
                results: Some((
                    LanScan {
                        subnets: vec![SubnetScan {
                            adapter: "Ethernet".into(),
                            cidr: "192.168.1.0/24".into(),
                            self_ip: "192.168.1.20".parse().unwrap(),
                            gateway: Some("192.168.1.1".parse().unwrap()),
                            hosts: vec![host("192.168.1.1", true), host("192.168.1.30", false)],
                        }],
                        findings: vec![Finding {
                            severity: Severity::High,
                            host: "192.168.1.1".into(),
                            title: "Telnet is open".into(),
                            detail: "Telnet sends passwords in clear text.".into(),
                            help_url: "https://example.invalid/telnet",
                        }],
                    },
                    SystemTime::now(),
                )),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[gpui_kit::test]
    fn shows_report_and_expands_a_device(cx: &mut TestAppContext) {
        let info = machines()[0].clone();
        let (window, app) = open(cx, info, scanned(), Default::default());
        go(cx, &app, Tab::LanSecurity);
        let shown = texts(cx, window);
        assert_shows(
            &shown,
            &[
                "Scan my network",
                "Last run just now",
                "2 devices found.",
                "1 risk",
                "Telnet is open",
                "How to fix this",
                "Devices on Ethernet (192.168.1.0/24)",
                "192.168.1.30",
                "router.lan",
                "1 open port",
                "no open ports",
            ],
        );
        // Device rows start closed.
        assert!(!shown.iter().any(|t| t == "BusyBox telnetd"));
        cx.update_window(window, |_, window, cx| {
            window.click(
                gpui_kit::ElementId::Name("lansec/host/192.168.1.0/24/192.168.1.1".into()),
                cx,
            );
        })
        .unwrap();
        assert_shows(
            &texts(cx, window),
            &["MAC address", "Telnet", "BusyBox telnetd"],
        );
        // No scan was started.
        assert!(cx.read(|cx| !app.read(cx).sampler.snapshot().lan.running));
    }
}
