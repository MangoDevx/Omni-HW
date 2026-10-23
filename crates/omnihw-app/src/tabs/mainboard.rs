use super::section;
use omnihw_core::SystemInfo;
use omnihw_core::smbios::is_placeholder;

/// A section with only the rows that have a real value, left out when none has.
fn known(ui: &mut egui::Ui, title: &str, rows: &[(&str, &str)]) {
    let rows: Vec<(&str, String)> = rows
        .iter()
        .filter(|(_, v)| !is_placeholder(v))
        .map(|(k, v)| (*k, v.trim().to_string()))
        .collect();
    if !rows.is_empty() {
        section(ui, title, &rows);
    }
}

pub fn show(ui: &mut egui::Ui, info: &SystemInfo) {
    let Some(s) = &info.smbios else {
        ui.label("SMBIOS table unavailable.");
        return;
    };
    if let Some(b) = &s.board {
        known(
            ui,
            "Motherboard",
            &[
                ("Manufacturer", &b.manufacturer),
                ("Model", &b.product),
                ("Version", &b.version),
            ],
        );
    }
    if let Some(sys) = &s.system {
        known(
            ui,
            "System",
            &[
                ("Manufacturer", &sys.manufacturer),
                ("Product", &sys.product),
                ("Version", &sys.version),
                ("Family", &sys.family),
            ],
        );
    }
    if let Some(b) = &s.bios {
        known(
            ui,
            "BIOS",
            &[
                ("Brand", &b.vendor),
                ("Version", &b.version),
                ("Date", &b.release_date),
            ],
        );
    }
    // Linux without root reads only /sys/class/dmi/id, which has no table version.
    let version = match s.version {
        (0, 0) => String::new(),
        (major, minor) => format!("{major}.{minor}"),
    };
    let socket = s
        .processors
        .first()
        .map(|p| p.socket.as_str())
        .unwrap_or("");
    known(
        ui,
        "Firmware",
        &[("SMBIOS Version", &version), ("CPU Socket", socket)],
    );
}
