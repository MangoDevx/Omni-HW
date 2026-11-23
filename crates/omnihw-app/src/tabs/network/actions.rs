//! Network changes requested by the page: the editor dialog for changes that need input first
//! (rename, IP, DNS, MTU), the confirmation every change goes through, and the buttons that
//! ask for them. Confirmed changes run on a worker thread ([`Sampler::net_action`]), outside
//! the snapshot lock; the result arrives as a status bar message.
//!
//! [`Sampler::net_action`]: crate::sampler::Sampler::net_action

use super::ViewState;
use crate::app::OmniApp;
use crate::elevate::elevated;
use crate::heat::ORANGE;
use crate::ui;
use crate::ui::LineSized;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::dialog::Dialog;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::radio::Radio;
use gpui_kit::component::{Disableable, WindowExt};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    App, AppContext, Context, ElementId, Entity, InteractiveElement, ParentElement, Styled,
    WeakEntity, Window, rems,
};
use omnihw_core::network::{MAX_DNS, MTU_RANGE, NetAction, parse_ipv4_list, parse_prefix};

/// A change that needs input first, edited in a dialog.
#[derive(Clone, Debug, PartialEq)]
pub enum Editor {
    Rename {
        guid: String,
        name: String,
        text: String,
    },
    Ip {
        if_index: u32,
        name: String,
        dhcp: bool,
        /// The adapter is connected: Windows only applies a fixed address then.
        connected: bool,
        ip: String,
        mask: String,
        gateway: String,
    },
    Dns {
        if_index: u32,
        name: String,
        auto: bool,
        text: String,
    },
    Mtu {
        if_index: u32,
        name: String,
        text: String,
    },
}

impl Editor {
    fn title(&self) -> String {
        match self {
            Editor::Rename { name, .. } => format!("Rename \"{name}\""),
            Editor::Ip { name, .. } => format!("IPv4 address of {name}"),
            Editor::Dns { name, .. } => format!("DNS servers of {name}"),
            Editor::Mtu { name, .. } => format!("MTU of {name}"),
        }
    }

    /// The editor's text, in the order of the dialog's text fields.
    fn texts(&self) -> [&str; 3] {
        match self {
            Editor::Rename { text, .. } | Editor::Dns { text, .. } | Editor::Mtu { text, .. } => {
                [text, "", ""]
            }
            Editor::Ip {
                ip, mask, gateway, ..
            } => [ip, mask, gateway],
        }
    }

    /// This editor with the text the user typed into the dialog's fields.
    fn with_texts(mut self, typed: [String; 3]) -> Self {
        let [a, b, c] = typed;
        match &mut self {
            Editor::Rename { text, .. } | Editor::Dns { text, .. } | Editor::Mtu { text, .. } => {
                *text = a
            }
            Editor::Ip {
                ip, mask, gateway, ..
            } => {
                *ip = a;
                *mask = b;
                *gateway = c;
            }
        }
        self
    }

    /// The action this editor describes, or why it's not valid yet.
    fn action(&self) -> Result<NetAction, String> {
        let a = match self.clone() {
            Editor::Rename { guid, name, text } => NetAction::Rename {
                guid,
                name,
                new_name: text.trim().to_string(),
            },
            Editor::Ip {
                if_index,
                name,
                dhcp: true,
                ..
            } => NetAction::Dhcp { if_index, name },
            Editor::Ip {
                if_index,
                name,
                ip,
                mask,
                gateway,
                ..
            } => NetAction::StaticIp {
                if_index,
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
            Editor::Dns {
                if_index,
                name,
                auto,
                text,
            } => NetAction::Dns {
                if_index,
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
            Editor::Mtu {
                if_index,
                name,
                text,
            } => NetAction::Mtu {
                if_index,
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

/// Ask for `action`: a confirmation dialog first, then the change runs on a worker thread.
pub(super) fn request(action: NetAction, window: &mut Window, cx: &mut Context<OmniApp>) {
    let mut text = action.confirm_text().to_string();
    if action.needs_restart() {
        text.push_str("\n\nRestart required: Windows must restart for this to take effect.");
    }
    if action.needs_admin() && !elevated() {
        text.push_str("\n\nWindows will ask for administrator permission.");
    }
    ui::confirm(
        action.label(),
        text,
        "Apply",
        window,
        cx,
        move |this, _, _| this.sampler.net_action(action.clone()),
    );
}

pub(super) fn admin_hint(a: &NetAction) -> &'static str {
    if a.needs_admin() && !elevated() {
        "Asks for administrator permission"
    } else {
        "Applies the change after you confirm"
    }
}

/// A button that requests `action` (disabled while another change runs).
pub(super) fn action_button(
    id: impl Into<ElementId>,
    text: &'static str,
    action: NetAction,
    hover: &str,
    busy: bool,
    cx: &mut Context<OmniApp>,
) -> Button {
    let hint = admin_hint(&action);
    Button::new(id)
        .line_sized()
        .caption(text)
        .tooltip(format!("{hover}\n{hint}"))
        .disabled(busy)
        .on_click(cx.listener(move |_, _, window, cx| request(action.clone(), window, cx)))
}

/// Open the editor dialog for `editor`, its text fields filled from it.
pub(super) fn edit(
    app: &mut OmniApp,
    editor: Editor,
    window: &mut Window,
    cx: &mut Context<OmniApp>,
) {
    let fields = app
        .views
        .network
        .fields
        .get_or_insert_with(|| std::array::from_fn(|_| cx.new(|cx| InputState::new(window, cx))))
        .clone();
    for (field, text) in fields.iter().zip(editor.texts()) {
        let text = text.to_string();
        field.update(cx, |f, cx| f.set_value(text, window, cx));
    }
    app.views.network.editor = Some(editor);
    let weak = cx.weak_entity();
    window.open_dialog(cx, move |dialog, _, cx| editor_dialog(dialog, &weak, cx));
}

/// The open editor with what has been typed so far.
fn current(view: &ViewState, cx: &App) -> Option<(Editor, [Entity<InputState>; 3])> {
    let fields = view.fields.clone()?;
    let typed = fields.each_ref().map(|f| f.read(cx).value().to_string());
    let mut ed = view.editor.clone()?.with_texts(typed);
    // Windows only applies a fixed address to a connected adapter.
    if let Editor::Ip {
        dhcp,
        connected: false,
        ..
    } = &mut ed
    {
        *dhcp = true;
    }
    Some((ed, fields))
}

/// Change the open editor (a radio button was picked).
fn update_editor(app: &WeakEntity<OmniApp>, cx: &mut App, change: impl FnOnce(&mut Editor)) {
    let _ = app.update(cx, |this, cx| {
        if let Some(ed) = &mut this.views.network.editor {
            change(ed);
        }
        cx.notify();
    });
}

/// The editor dialog: built again on every redraw from the page's [`ViewState`], so it shows
/// whether the input is valid as the user types.
fn editor_dialog(dialog: Dialog, app: &WeakEntity<OmniApp>, cx: &mut App) -> Dialog {
    let Some(entity) = app.upgrade() else {
        return dialog;
    };
    let Some((ed, fields)) = current(&entity.read(cx).views.network, cx) else {
        return dialog;
    };
    let check = ed.action();
    let mut body = ui::column().gap_2();
    match &ed {
        Editor::Rename { .. } => {
            body = body
                .child(ui::label("New name"))
                .child(Input::new(&fields[0]));
        }
        Editor::Ip {
            dhcp, connected, ..
        } => {
            let dhcp = *dhcp;
            let (auto, manual) = (app.clone(), app.clone());
            body = body
                .child(
                    Radio::new("net-ip-dhcp")
                        .label("Get an IP address automatically (DHCP)")
                        .checked(dhcp)
                        .on_click(move |_, _, cx| {
                            update_editor(&auto, cx, |ed| {
                                if let Editor::Ip { dhcp, .. } = ed {
                                    *dhcp = true;
                                }
                            })
                        }),
                )
                .child(
                    Radio::new("net-ip-static")
                        .label("Use this IP address:")
                        .checked(!dhcp)
                        .disabled(!*connected)
                        .when(!*connected, |r| {
                            r.tooltip(
                                "Connect the adapter first: Windows only applies a fixed address to a connected adapter.",
                            )
                        })
                        .on_click(move |_, _, cx| {
                            update_editor(&manual, cx, |ed| {
                                if let Editor::Ip { dhcp, .. } = ed {
                                    *dhcp = false;
                                }
                            })
                        }),
                );
            for (k, (name, field)) in ["IP address", "Subnet mask", "Default gateway"]
                .into_iter()
                .zip(fields.iter())
                .enumerate()
            {
                body = body.child(
                    ui::row()
                        .id(("net-ip-field", k))
                        .child(ui::label(name).flex_none().w(rems(9.0)))
                        .child(Input::new(field).disabled(dhcp).flex_1()),
                );
            }
            body = body.child(ui::weak(
                if dhcp {
                    "DNS servers are also set back to automatic."
                } else {
                    "IPv4 only. Set DNS servers separately (DNS button)."
                },
                cx,
            ));
        }
        Editor::Dns { auto, .. } => {
            let (to_auto, to_manual) = (app.clone(), app.clone());
            body = body
                .child(
                    Radio::new("net-dns-auto")
                        .label("Get DNS servers automatically")
                        .checked(*auto)
                        .on_click(move |_, _, cx| {
                            update_editor(&to_auto, cx, |ed| {
                                if let Editor::Dns { auto, .. } = ed {
                                    *auto = true;
                                }
                            })
                        }),
                )
                .child(
                    Radio::new("net-dns-manual")
                        .label("Use these DNS servers (IPv4):")
                        .checked(!*auto)
                        .on_click(move |_, _, cx| {
                            update_editor(&to_manual, cx, |ed| {
                                if let Editor::Dns { auto, .. } = ed {
                                    *auto = false;
                                }
                            })
                        }),
                )
                .child(Input::new(&fields[0]).disabled(*auto))
                .child(ui::weak(
                    format!("Up to {MAX_DNS}, separated by commas, e.g. 1.1.1.1, 8.8.8.8"),
                    cx,
                ));
        }
        Editor::Mtu { .. } => {
            body = body
                .child(ui::label(format!(
                    "MTU in bytes ({}-{}; 1500 is standard)",
                    MTU_RANGE.start(),
                    MTU_RANGE.end()
                )))
                .child(Input::new(&fields[0]));
        }
    }
    if let Err(e) = &check {
        body = body.child(ui::colored(e.clone(), ORANGE, cx));
    }
    let (ok, cancel) = (app.clone(), app.clone());
    body = body.child(
        ui::row()
            .pt_1()
            .child(
                Button::new("net-editor-continue")
                    .line_sized()
                    .primary()
                    .caption("Continue")
                    .disabled(check.is_err())
                    .on_click(move |_, window, cx| {
                        let _ = ok.update(cx, |this, cx| finish(this, window, cx));
                    }),
            )
            .child(
                Button::new("net-editor-cancel")
                    .line_sized()
                    .caption("Cancel")
                    .on_click(move |_, window, cx| {
                        let _ = cancel.update(cx, |this, cx| {
                            this.views.network.editor = None;
                            cx.notify();
                        });
                        window.close_dialog(cx);
                    }),
            ),
    );
    dialog.title(ui::strong(ed.title())).child(body)
}

/// Continue was clicked: close the editor and confirm the change it describes.
fn finish(app: &mut OmniApp, window: &mut Window, cx: &mut Context<OmniApp>) {
    let Some((ed, _)) = current(&app.views.network, cx) else {
        return;
    };
    let Ok(action) = ed.action() else { return };
    app.views.network.editor = None;
    window.close_dialog(cx);
    request(action, window, cx);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editors_build_valid_actions() {
        let ip = |dhcp: bool, ip: &str, mask: &str, gw: &str| Editor::Ip {
            if_index: 7,
            name: "Ethernet".into(),
            dhcp,
            connected: true,
            ip: ip.into(),
            mask: mask.into(),
            gateway: gw.into(),
        };
        assert!(matches!(
            ip(true, "", "", "").action(),
            Ok(NetAction::Dhcp { if_index: 7, .. })
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
            if_index: 7,
            name: "Wi-Fi".into(),
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
            if_index: 7,
            name: "Ethernet".into(),
            text: t.into(),
        };
        assert!(mtu("1500").action().is_ok());
        assert!(mtu("100").action().is_err());
        assert!(mtu("abc").action().is_err());

        let rename = |t: &str| Editor::Rename {
            guid: "{4D36E972-E325-11CE-BFC1-08002BE10318}".into(),
            name: "Ethernet".into(),
            text: t.into(),
        };
        assert!(rename("Desk LAN").action().is_ok());
        assert!(rename("a:b").action().is_err());
        assert!(rename("   ").action().is_err());
    }

    #[test]
    fn typed_text_goes_to_the_right_fields() {
        let ip = Editor::Ip {
            if_index: 7,
            name: "Ethernet".into(),
            dhcp: false,
            connected: true,
            ip: String::new(),
            mask: String::new(),
            gateway: String::new(),
        };
        let typed = ["10.0.0.5".to_string(), "24".into(), "10.0.0.1".into()];
        let ip = ip.with_texts(typed);
        assert_eq!(ip.texts(), ["10.0.0.5", "24", "10.0.0.1"]);
        assert!(ip.action().is_ok());
        let mtu = Editor::Mtu {
            if_index: 7,
            name: "Ethernet".into(),
            text: "1500".into(),
        };
        assert_eq!(mtu.texts(), ["1500", "", ""]);
    }
}
