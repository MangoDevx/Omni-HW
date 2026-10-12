use super::{or_dash, section};
use omnihw_core::SystemInfo;

pub fn show(ui: &mut egui::Ui, info: &SystemInfo) {
    let Some(s) = &info.smbios else {
        ui.label("SMBIOS table unavailable.");
        return;
    };
    if let Some(b) = &s.board {
        section(
            ui,
            "Motherboard",
            &[
                ("Manufacturer", or_dash(&b.manufacturer)),
                ("Model", or_dash(&b.product)),
                ("Version", or_dash(&b.version)),
            ],
        );
    }
    if let Some(sys) = &s.system {
        section(
            ui,
            "System",
            &[
                ("Manufacturer", or_dash(&sys.manufacturer)),
                ("Product", or_dash(&sys.product)),
                ("Version", or_dash(&sys.version)),
                ("Family", or_dash(&sys.family)),
            ],
        );
    }
    if let Some(b) = &s.bios {
        section(
            ui,
            "BIOS",
            &[
                ("Brand", or_dash(&b.vendor)),
                ("Version", or_dash(&b.version)),
                ("Date", or_dash(&b.release_date)),
            ],
        );
    }
    let mut misc = vec![("SMBIOS Version", format!("{}.{}", s.version.0, s.version.1))];
    if let Some(p) = s.processors.first() {
        misc.push(("CPU Socket", or_dash(&p.socket)));
    }
    section(ui, "Firmware", &misc);
}
