//! Mainboard page: board, system, BIOS and firmware identity from SMBIOS.

use crate::app::OmniApp;
use crate::ui::{self, section};
use gpui_kit::{AnyElement, Context, IntoElement, ParentElement, Window};

pub fn show(app: &mut OmniApp, _window: &mut Window, cx: &mut Context<OmniApp>) -> AnyElement {
    let Some(s) = app.sampler.info().and_then(|i| i.smbios.as_ref()) else {
        return ui::label("SMBIOS table unavailable.").into_any_element();
    };
    let mut page = ui::column();
    if let Some(b) = &s.board {
        page = page.child(section(
            "Motherboard",
            &[
                ("Manufacturer", b.manufacturer.clone()),
                ("Model", b.product.clone()),
                ("Version", b.version.clone()),
            ],
            cx,
        ));
    }
    if let Some(sys) = &s.system {
        page = page.child(section(
            "System",
            &[
                ("Manufacturer", sys.manufacturer.clone()),
                ("Product", sys.product.clone()),
                ("Version", sys.version.clone()),
                ("Family", sys.family.clone()),
            ],
            cx,
        ));
    }
    if let Some(b) = &s.bios {
        page = page.child(section(
            "BIOS",
            &[
                ("Brand", b.vendor.clone()),
                ("Version", b.version.clone()),
                ("Date", b.release_date.clone()),
            ],
            cx,
        ));
    }
    let mut misc = vec![("SMBIOS Version", format!("{}.{}", s.version.0, s.version.1))];
    if let Some(p) = s.processors.first() {
        misc.push(("CPU Socket", p.socket.clone()));
    }
    page.child(section("Firmware", &misc, cx))
        .into_any_element()
}
