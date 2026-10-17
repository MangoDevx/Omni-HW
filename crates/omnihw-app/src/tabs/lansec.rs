//! LAN Security Report page: scans the connected adapters' own private subnets for devices and
//! common weaknesses, then lists ranked findings and a device inventory. The page only reads
//! the snapshot and asks for a scan; the scan runs on the sampler's worker thread (see
//! `Sampler::run_lan_scan`). It stays on the local network and changes nothing on any device —
//! each finding explains the issue and links to fix steps the user opens in their own browser.

use super::{group_box, page_title, pill, temp};
use crate::heat::{BLUE, GREEN, ORANGE, RED};
use crate::sampler::{Sampler, Snapshot};
use egui::collapsing_header::CollapsingState;
use egui::{Color32, Id, RichText, Ui};
use omnihw_core::lansec::{Finding, Host, Phase, Risk, Severity, SubnetScan};
use std::time::{Duration, SystemTime};

/// egui-memory flag: the user asked for a scan this frame.
const SCAN: &str = "lan_scan";

pub fn show(ui: &mut Ui, snap: &Snapshot) {
    page_title(ui, "LAN Security Report");
    ui.add(
        egui::Label::new(
            RichText::new(
                "Scans your own local network for devices and common security problems — \
                 clear-text logins, exposed admin pages, unauthenticated databases and more. \
                 It only looks (no passwords are tried and nothing is changed), stays on your \
                 local network, and runs only when you click.",
            )
            .weak(),
        )
        .wrap(),
    );
    ui.add_space(8.0);

    let Some(net) = &snap.network else {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label("Reading network adapters...");
        });
        return;
    };
    let scannable = net
        .adapters
        .iter()
        .any(|a| a.status == omnihw_core::network::Status::Up);

    ui.horizontal(|ui| {
        if snap.lan.running {
            ui.add(egui::Spinner::new());
            ui.label("Scanning...");
        } else if ui
            .add_enabled(scannable, egui::Button::new(RichText::new("Scan my network").strong()))
            .on_hover_text("Discover devices and probe common ports on your local subnet")
            .on_disabled_hover_text("Connect to a network first")
            .clicked()
        {
            temp::set(ui.ctx(), Id::new(SCAN), true);
        }
        if let Some((_, at)) = &snap.lan.results {
            ui.weak(format!("Last run {}", ago(*at)));
        }
    });

    if snap.lan.running {
        progress(ui, snap);
        // Keep the progress bar animating while the worker thread runs.
        ui.ctx().request_repaint_after(Duration::from_millis(200));
    }

    if let Some((scan, _)) = &snap.lan.results {
        ui.add_space(8.0);
        summary(ui, scan);
        findings(ui, &scan.findings);
        devices(ui, &scan.subnets);
    } else if !snap.lan.running {
        ui.add_space(8.0);
        ui.weak("No scan yet. A scan of a typical home network takes well under a minute.");
    }
}

fn progress(ui: &mut Ui, snap: &Snapshot) {
    let (phase, done, total) = snap.lan.progress.read();
    let (label, frac) = match phase {
        Phase::Discovering => ("Finding devices", ratio(done, total)),
        Phase::Scanning => ("Checking ports", ratio(done, total)),
        Phase::Done => ("Finishing", 1.0),
    };
    ui.add(
        egui::ProgressBar::new(frac)
            .text(format!("{label} ({done}/{total})"))
            .desired_width(360.0),
    );
}

fn ratio(done: u32, total: u32) -> f32 {
    if total == 0 {
        0.0
    } else {
        (done as f32 / total as f32).clamp(0.0, 1.0)
    }
}

fn summary(ui: &mut Ui, scan: &omnihw_core::lansec::LanScan) {
    let (high, warn, info) = scan.counts();
    group_box(
        ui,
        RichText::new("Summary").size(16.0).strong(),
        4.0,
        8.0,
        |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(format!("{} devices found.", scan.total_hosts()));
                if high == 0 && warn == 0 {
                    pill(ui, "No problems found", GREEN, false);
                } else {
                    if high > 0 {
                        pill(ui, format!("{high} risk"), RED, false);
                    }
                    if warn > 0 {
                        pill(ui, format!("{warn} to check"), ORANGE, false);
                    }
                }
                if info > 0 {
                    pill(ui, format!("{info} note{}", plural(info)), BLUE, false);
                }
            });
        },
    );
}

fn severity_color(s: Severity) -> Color32 {
    match s {
        Severity::High => RED,
        Severity::Warn => ORANGE,
        Severity::Info => BLUE,
    }
}

fn findings(ui: &mut Ui, findings: &[Finding]) {
    group_box(
        ui,
        RichText::new("Findings").size(16.0).strong(),
        4.0,
        8.0,
        |ui| {
            if findings.is_empty() {
                ui.label("Nothing stood out. Your devices aren't exposing the risky services this check looks for.");
                return;
            }
            for (i, f) in findings.iter().enumerate() {
                if i > 0 {
                    ui.separator();
                }
                ui.horizontal_wrapped(|ui| {
                    pill(ui, f.severity.name(), severity_color(f.severity), true);
                    ui.label(RichText::new(&f.title).strong());
                    if !f.host.is_empty() {
                        ui.weak(format!("— {}", f.host));
                    }
                });
                ui.add(egui::Label::new(&f.detail).wrap());
                if !f.help_url.is_empty() {
                    ui.hyperlink_to("How to fix this", f.help_url);
                }
                ui.add_space(2.0);
            }
        },
    );
}

fn risk_color(r: Risk) -> Color32 {
    match r {
        Risk::High => RED,
        Risk::Medium => ORANGE,
        Risk::Low => BLUE,
        Risk::Info => Color32::GRAY,
    }
}

fn devices(ui: &mut Ui, subnets: &[SubnetScan]) {
    for subnet in subnets {
        group_box(
            ui,
            RichText::new(format!("Devices on {} ({})", subnet.adapter, subnet.cidr))
                .size(16.0)
                .strong(),
            4.0,
            8.0,
            |ui| {
                if subnet.hosts.is_empty() {
                    ui.label("No devices answered on this network.");
                    return;
                }
                for host in &subnet.hosts {
                    host_row(ui, host);
                }
            },
        );
    }
}

fn host_row(ui: &mut Ui, host: &Host) {
    let id = ui.make_persistent_id(("lan_host", host.ip));
    let open_ports = host.ports.len();
    CollapsingState::load_with_default_open(ui.ctx(), id, false)
        .show_header(ui, |ui| {
            ui.label(RichText::new(host.ip.to_string()).strong().monospace());
            if host.is_gateway {
                pill(ui, "Router", GREEN, true);
            }
            if host.is_self {
                pill(ui, "This PC", BLUE, true);
            }
            if !host.hostname.is_empty() {
                ui.label(&host.hostname);
            }
            if !host.vendor.is_empty() {
                ui.weak(&host.vendor);
            }
            if open_ports == 0 {
                ui.weak("no open ports");
            } else {
                let worst = host.ports.iter().map(|p| p.risk).max().unwrap_or(Risk::Info);
                pill(
                    ui,
                    format!("{open_ports} open port{}", plural(open_ports)),
                    risk_color(worst),
                    true,
                );
            }
        })
        .body(|ui| {
            egui::Grid::new(("lan_host_rows", host.ip))
                .num_columns(2)
                .spacing([16.0, 3.0])
                .show(ui, |ui| {
                    if !host.mac.is_empty() {
                        ui.label("MAC address");
                        ui.label(RichText::new(&host.mac).monospace());
                        ui.end_row();
                    }
                    if host.ports.is_empty() {
                        ui.label("Open ports");
                        ui.label("none");
                        ui.end_row();
                    }
                    for p in &host.ports {
                        ui.horizontal(|ui| {
                            pill(ui, p.port.to_string(), risk_color(p.risk), true);
                            ui.label(p.service);
                        });
                        ui.add(
                            egui::Label::new(if p.banner.is_empty() {
                                "open".to_string()
                            } else {
                                p.banner.clone()
                            })
                            .wrap(),
                        );
                        ui.end_row();
                    }
                });
        });
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

fn ago(t: SystemTime) -> String {
    let s = t.elapsed().map(|d| d.as_secs()).unwrap_or(0);
    if s < 60 {
        "just now".into()
    } else {
        format!("{} min ago", s / 60)
    }
}

/// Start a scan if the button asked for one. Call once per frame.
pub fn dispatch(ctx: &egui::Context, sampler: &Sampler) {
    if temp::take::<bool>(ctx, Id::new(SCAN)).unwrap_or(false) {
        sampler.run_lan_scan();
    }
}
