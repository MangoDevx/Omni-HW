use super::section;
use crate::sampler::Snapshot;
use omnihw_core::SystemInfo;

pub fn show(ui: &mut egui::Ui, info: &SystemInfo, snap: &Snapshot) {
    section(
        ui,
        "Omni-HW",
        &[
            ("Version", env!("CARGO_PKG_VERSION").into()),
            ("License", "MIT".into()),
            ("Driver", "None (user-mode APIs only)".into()),
            ("Samples taken", snap.samples.to_string()),
            (
                "Administrator",
                if omnihw_core::win::is_elevated() {
                    "Yes".into()
                } else {
                    "No (SATA drive health needs it; Omni-HW asks via UAC at start when SATA drives are present)".into()
                },
            ),
        ],
    );
    let problems: Vec<&String> = info.warnings.iter().chain(&snap.errors).collect();
    if !problems.is_empty() {
        ui.label(egui::RichText::new("Warnings").strong());
        for p in problems {
            ui.label(p);
        }
    }
}
