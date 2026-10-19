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
            ("Driver", driver_line(snap)),
            ("Samples taken", snap.samples.to_string()),
            (
                "Administrator",
                if crate::elevate::elevated() {
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

/// Omni-HW uses no driver itself; with extra sensors set up, the sensor service uses PawnIO.
fn driver_line(snap: &Snapshot) -> String {
    use omnihw_core::sensor_service::ServiceState;
    match snap.sensor_service.as_ref().map(|s| &s.state) {
        Some(ServiceState::Ready { sources, .. }) if !sources.is_empty() => format!(
            "None in Omni-HW itself; the sensor service reads {} through PawnIO",
            sources.join(", ")
        ),
        _ => "None (user-mode APIs only; extra sensors through PawnIO are optional, see Settings)"
            .into(),
    }
}
