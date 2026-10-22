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
            (
                "Data sources",
                "The kernel's sysfs and procfs (hwmon, DRM, power supply, block devices), and NVIDIA's NVML when installed".into(),
            ),
            ("Samples taken", snap.samples.to_string()),
            (
                "Root",
                if crate::elevate::elevated() {
                    "Yes".into()
                } else {
                    "No (memory modules, CPU power and drive health need root; see Settings)".into()
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
