use super::section;
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
                ("Manufacturer", b.manufacturer.clone()),
                ("Model", b.product.clone()),
                ("Version", b.version.clone()),
            ],
        );
    }
    if let Some(sys) = &s.system {
        section(
            ui,
            "System",
            &[
                ("Manufacturer", sys.manufacturer.clone()),
                ("Product", sys.product.clone()),
                ("Version", sys.version.clone()),
                ("Family", sys.family.clone()),
            ],
        );
    }
    if let Some(b) = &s.bios {
        section(
            ui,
            "BIOS",
            &[
                ("Brand", b.vendor.clone()),
                ("Version", b.version.clone()),
                ("Date", b.release_date.clone()),
            ],
        );
    }
    // Linux without root reads only /sys/class/dmi/id, which has no table version.
    let version = match s.version {
        (0, 0) => String::new(),
        (major, minor) => format!("{major}.{minor}"),
    };
    let mut misc = vec![("SMBIOS Version", version)];
    if let Some(p) = s.processors.first() {
        misc.push(("CPU Socket", p.socket.clone()));
    }
    section(ui, "Firmware", &misc);
}
