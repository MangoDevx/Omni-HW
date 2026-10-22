//! Network changes and tests requested by the page: kept in egui memory during the frame, then
//! (after the editor and confirmation dialogs) run on worker threads by [`dispatch`], outside
//! the snapshot lock.

use crate::sampler::Sampler;
use crate::tabs::temp;
use egui::{Id, RichText, Ui};
use omnihw_core::network::{MAX_DNS, MTU_RANGE, NetAction, parse_ipv4_list, parse_prefix};

const PENDING: &str = "net_pending";
const CONFIRM: &str = "net_confirm";
const EDITOR: &str = "net_editor";
pub(super) const REFRESH: &str = "net_refresh";
pub(super) const LOCAL_TESTS: &str = "net_local_tests";
pub(super) const INTERNET_TESTS: &str = "net_internet_tests";
/// Local tests already ran automatically this session.
const AUTO_RAN: &str = "net_auto_ran";

/// A change that needs input first, edited in a dialog.
#[derive(Clone, Debug, PartialEq)]
pub enum Editor {
    Ip {
        name: String,
        dhcp: bool,
        ip: String,
        mask: String,
        gateway: String,
    },
    Dns {
        name: String,
        auto: bool,
        text: String,
    },
    Mtu {
        name: String,
        text: String,
    },
}

impl Editor {
    fn title(&self) -> String {
        match self {
            Editor::Ip { name, .. } => format!("IPv4 address of {name}"),
            Editor::Dns { name, .. } => format!("DNS servers of {name}"),
            Editor::Mtu { name, .. } => format!("MTU of {name}"),
        }
    }

    /// The action this editor describes, or why it's not valid yet.
    fn action(&self) -> Result<NetAction, String> {
        let a = match self.clone() {
            Editor::Ip {
                name, dhcp: true, ..
            } => NetAction::Dhcp { name },
            Editor::Ip {
                name,
                ip,
                mask,
                gateway,
                ..
            } => NetAction::StaticIp {
                name,
                ip: ip
                    .trim()
                    .parse()
                    .map_err(|_| format!("\"{}\" is not an IPv4 address", ip.trim()))?,
                prefix: parse_prefix(&mask).ok_or_else(|| {
                    "Enter the subnet mask as 255.255.255.0 or a prefix length such as 24"
                        .to_string()
                })?,
                gateway: match gateway.trim() {
                    "" => None,
                    g => Some(
                        g.parse()
                            .map_err(|_| format!("\"{g}\" is not an IPv4 address"))?,
                    ),
                },
            },
            Editor::Dns { name, auto, text } => NetAction::Dns {
                name,
                servers: if auto {
                    Vec::new()
                } else {
                    let list = parse_ipv4_list(&text)?;
                    if list.is_empty() {
                        return Err("Enter at least one DNS server, or choose automatic".into());
                    }
                    list
                },
            },
            Editor::Mtu { name, text } => NetAction::Mtu {
                name,
                mtu: text.trim().parse().map_err(|_| {
                    format!(
                        "Enter a number between {} and {}",
                        MTU_RANGE.start(),
                        MTU_RANGE.end()
                    )
                })?,
            },
        };
        a.validate()?;
        Ok(a)
    }
}

/// Ask for `action` (it is confirmed first).
pub fn request(ctx: &egui::Context, action: NetAction) {
    temp::set(ctx, Id::new(PENDING), Some(action));
}

/// Open an editor dialog.
pub fn edit(ctx: &egui::Context, editor: Editor) {
    temp::set(ctx, Id::new(EDITOR), Some(editor));
}

pub(super) fn flag(ctx: &egui::Context, key: &str) {
    temp::set(ctx, Id::new(key), true);
}

fn take_flag(ctx: &egui::Context, key: &str) -> bool {
    temp::take::<bool>(ctx, Id::new(key)).unwrap_or(false)
}

/// A button that requests `action`.
pub(super) fn action_button(ui: &mut Ui, text: &str, action: NetAction, hover: &str) {
    if ui
        .button(text)
        .on_hover_text(format!("{hover}\nApplies the change after you confirm"))
        .clicked()
    {
        request(ui.ctx(), action);
    }
}

/// The editor dialog. Returns the action once the user clicks Continue on valid input.
fn editor_dialog(ctx: &egui::Context, mut ed: Editor) -> Option<NetAction> {
    let mut close = false;
    let mut result = None;
    let check = ed.action();
    egui::Modal::new(Id::new("net_editor_modal")).show(ctx, |ui| {
        ui.set_max_width(440.0);
        ui.label(RichText::new(ed.title()).size(16.0).strong());
        ui.add_space(6.0);
        match &mut ed {
            Editor::Ip {
                dhcp,
                ip,
                mask,
                gateway,
                ..
            } => {
                ui.radio_value(dhcp, true, "Get an IP address automatically (DHCP)");
                ui.radio_value(dhcp, false, "Use this IP address:");
                ui.add_enabled_ui(!*dhcp, |ui| {
                    egui::Grid::new("net_ip_grid")
                        .num_columns(2)
                        .spacing([12.0, 4.0])
                        .show(ui, |ui| {
                            ui.label("IP address");
                            ui.text_edit_singleline(ip);
                            ui.end_row();
                            ui.label("Subnet mask");
                            ui.text_edit_singleline(mask);
                            ui.end_row();
                            ui.label("Default gateway");
                            ui.text_edit_singleline(gateway);
                            ui.end_row();
                        });
                });
                ui.add_space(4.0);
                ui.weak(if *dhcp {
                    "DNS servers are also set back to automatic."
                } else {
                    "IPv4 only. Set DNS servers separately (DNS button)."
                });
            }
            Editor::Dns { auto, text, .. } => {
                ui.radio_value(auto, true, "Get DNS servers automatically");
                ui.radio_value(auto, false, "Use these DNS servers (IPv4):");
                ui.add_enabled_ui(!*auto, |ui| {
                    ui.text_edit_singleline(text);
                    ui.weak(format!(
                        "Up to {MAX_DNS}, separated by commas, e.g. 1.1.1.1, 8.8.8.8"
                    ));
                });
            }
            Editor::Mtu { text, .. } => {
                ui.label(format!(
                    "MTU in bytes ({}-{}; 1500 is standard)",
                    MTU_RANGE.start(),
                    MTU_RANGE.end()
                ));
                ui.text_edit_singleline(text);
            }
        }
        ui.add_space(6.0);
        if let Err(e) = &check {
            ui.colored_label(crate::heat::ORANGE, e);
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    check.is_ok(),
                    egui::Button::new(RichText::new("Continue").strong()),
                )
                .clicked()
            {
                result = check.clone().ok();
                close = true;
            }
            if ui.button("Cancel").clicked() {
                close = true;
            }
        });
    });
    if close {
        temp::take::<Option<Editor>>(ctx, Id::new(EDITOR));
    } else {
        temp::set(ctx, Id::new(EDITOR), Some(ed));
    }
    result
}

/// Run requested changes and tests, and show the editor and confirmation dialogs. Call once per
/// frame, outside the snapshot lock. Returns a status message when a change finished.
pub fn dispatch(ctx: &egui::Context, sampler: &Sampler, page_open: bool) -> Option<String> {
    if take_flag(ctx, REFRESH) {
        sampler.refresh_network();
    }
    if take_flag(ctx, LOCAL_TESTS) {
        sampler.run_local_tests();
    }
    if take_flag(ctx, INTERNET_TESTS) {
        sampler.run_internet_tests();
    }
    // The local tests only contact the local network, so they run once by themselves the
    // first time the page shows adapters.
    if page_open && !temp::get::<bool>(ctx, Id::new(AUTO_RAN)).unwrap_or(false) {
        let ready = sampler.snapshot().network.is_some();
        if ready {
            temp::set(ctx, Id::new(AUTO_RAN), true);
            sampler.run_local_tests();
        }
    }
    if let Some(ed) = temp::get::<Option<Editor>>(ctx, Id::new(EDITOR)).flatten()
        && let Some(a) = editor_dialog(ctx, ed)
    {
        request(ctx, a);
    }
    if let Some(a) = temp::take::<Option<NetAction>>(ctx, Id::new(PENDING)).flatten() {
        temp::set(ctx, Id::new(CONFIRM), Some(a));
    }
    if let Some(a) = temp::get::<Option<NetAction>>(ctx, Id::new(CONFIRM)).flatten() {
        let mut close = None;
        egui::Modal::new(Id::new("net_confirm_modal")).show(ctx, |ui| {
            ui.set_max_width(440.0);
            ui.label(RichText::new(a.label()).size(16.0).strong());
            ui.add_space(6.0);
            ui.add(egui::Label::new(a.confirm_text()).wrap());
            if a.interface().is_some() {
                ui.add_space(4.0);
                ui.weak("The change is saved in the interface's NetworkManager connection.");
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button(RichText::new("Apply").strong()).clicked() {
                    close = Some(true);
                }
                if ui.button("Cancel").clicked() {
                    close = Some(false);
                }
            });
        });
        if let Some(apply) = close {
            temp::take::<Option<NetAction>>(ctx, Id::new(CONFIRM));
            if apply {
                sampler.net_action(a);
            }
        }
    }
    sampler.snapshot().net_msg.take()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editors_build_valid_actions() {
        let ip = |dhcp: bool, ip: &str, mask: &str, gw: &str| Editor::Ip {
            name: "enp5s0".into(),
            dhcp,
            ip: ip.into(),
            mask: mask.into(),
            gateway: gw.into(),
        };
        assert!(matches!(
            ip(true, "", "", "").action(),
            Ok(NetAction::Dhcp { .. })
        ));
        let Ok(NetAction::StaticIp {
            prefix, gateway, ..
        }) = ip(false, " 192.168.1.50 ", "255.255.255.0", "192.168.1.1").action()
        else {
            panic!("static IP should parse")
        };
        assert_eq!(prefix, 24);
        assert_eq!(gateway, Some("192.168.1.1".parse().unwrap()));
        assert!(ip(false, "192.168.1.50", "24", "").action().is_ok());
        assert!(ip(false, "192.168.1.500", "24", "").action().is_err());
        assert!(
            ip(false, "192.168.1.50", "255.0.255.0", "")
                .action()
                .is_err()
        );
        assert!(
            ip(false, "192.168.1.50", "24", "10.0.0.1")
                .action()
                .is_err()
        );

        let dns = |auto: bool, text: &str| Editor::Dns {
            name: "wlp6s0".into(),
            auto,
            text: text.into(),
        };
        assert!(
            matches!(dns(true, "junk").action(), Ok(NetAction::Dns { servers, .. }) if servers.is_empty())
        );
        assert!(
            matches!(dns(false, "1.1.1.1, 8.8.8.8").action(), Ok(NetAction::Dns { servers, .. }) if servers.len() == 2)
        );
        assert!(dns(false, "").action().is_err());
        assert!(dns(false, "1.1.1.1, x").action().is_err());

        let mtu = |t: &str| Editor::Mtu {
            name: "enp5s0".into(),
            text: t.into(),
        };
        assert!(mtu("1500").action().is_ok());
        assert!(mtu("100").action().is_err());
        assert!(mtu("abc").action().is_err());
    }
}
