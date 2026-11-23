//! About page: version, licence and anything that went wrong while reading hardware.

use crate::app::OmniApp;
use crate::sampler::Snapshot;
use crate::ui::{self, section};
use gpui_kit::{AnyElement, Context, IntoElement, ParentElement, Window};

pub fn show(app: &mut OmniApp, _window: &mut Window, cx: &mut Context<OmniApp>) -> AnyElement {
    let Some(info) = app.sampler.info() else {
        return ui::column().into_any_element();
    };
    let snap = app.sampler.snapshot();
    let mut page = ui::column().child(section(
        "Omni-HW",
        &[
            ("Version", env!("CARGO_PKG_VERSION").into()),
            ("License", "MIT".into()),
            ("Driver", driver_line(&snap)),
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
        cx,
    ));
    let problems: Vec<&String> = info.warnings.iter().chain(&snap.errors).collect();
    if !problems.is_empty() {
        page = page
            .child(ui::strong("Warnings"))
            .children(problems.into_iter().map(|p| ui::label(p.clone())));
    }
    page.into_any_element()
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
