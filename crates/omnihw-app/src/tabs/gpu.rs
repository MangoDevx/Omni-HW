//! GPU page: the selected adapter's identity, memory and limits, and its sensors.

use crate::app::OmniApp;
use crate::ui::{self, section};
use gpui_kit::{AnyElement, Context, IntoElement, ParentElement, SharedString, Styled, Window};
use omnihw_core::format_bytes;

#[derive(Default)]
pub struct ViewState {
    /// Index of the adapter shown.
    selected: usize,
}

pub fn show(app: &mut OmniApp, _window: &mut Window, cx: &mut Context<OmniApp>) -> AnyElement {
    let Some(info) = app.sampler.info() else {
        return ui::column().into_any_element();
    };
    if info.gpus.is_empty() {
        return ui::label("No hardware display adapter found.").into_any_element();
    }
    let selected = app.views.gpu.selected.min(info.gpus.len() - 1);
    let mut page = ui::column();
    if info.gpus.len() > 1 {
        page = page.child(
            ui::row()
                .pb_1p5()
                .child(ui::dropdown(
                    "adapter",
                    info.gpus[selected].name.clone(),
                    info.gpus
                        .iter()
                        .map(|g| SharedString::from(g.name.clone()))
                        .collect(),
                    Some(selected),
                    cx,
                    |this, i, _, _| this.views.gpu.selected = i,
                ))
                .child(ui::label("Adapter")),
        );
    }
    let snap = app.sampler.snapshot();
    let g = &info.gpus[selected];
    let chip = g.chip();
    let nv = g.nvml.as_ref();

    let board = match g.board_vendor() {
        Some(b) if b != g.vendor_name() => format!("{} ({b})", g.vendor_name()),
        _ => g.vendor_name().to_string(),
    };
    let mut card = vec![
        ("Name", g.name.clone()),
        ("GPU", chip.map(|c| c.chip.to_string()).unwrap_or_default()),
        (
            "Architecture",
            chip.map(|c| c.architecture.to_string()).unwrap_or_default(),
        ),
        (
            "Technology",
            chip.map(|c| c.process.to_string()).unwrap_or_default(),
        ),
        ("Vendor", board),
        ("Device ID", g.pci_ids()),
        ("Revision", format!("{:X}", g.revision)),
    ];
    if let Some(n) = nv {
        let link = |l: Option<(u32, u32)>| l.map(|(gen_, w)| format!("x{w} {gen_}.0"));
        if let Some(max) = link(n.pcie_max) {
            let cur = link(n.pcie_current).unwrap_or_default();
            card.push(("Bus Interface", format!("PCIe {max} @ {cur}")));
        }
        card.push(("Bus ID", n.bus_id.clone()));
        card.push(("BIOS Version", n.vbios.clone()));
    }
    let driver = match nv {
        Some(n) if !n.driver_version.is_empty() => {
            format!("{} ({})", n.driver_version, g.driver_version)
        }
        _ => g.driver_version.clone(),
    };
    card.push(("Driver Version", driver));
    if let Some((y, m, d)) = g.driver_date {
        card.push(("Driver Date", format!("{y}-{m:02}-{d:02}")));
    }
    let rebar = match g.rebar() {
        omnihw_core::gpu::ReBar::Enabled { bar_bytes } => {
            format!("On ({} BAR)", format_bytes(bar_bytes))
        }
        omnihw_core::gpu::ReBar::Disabled { bar_bytes } => {
            format!("Off ({} BAR)", format_bytes(bar_bytes))
        }
        omnihw_core::gpu::ReBar::Unknown => "Unknown (NVIDIA only so far)".into(),
    };
    card.push(("Resizable BAR", rebar));
    let displays = if g.displays.is_empty() {
        "None".to_string()
    } else {
        g.displays
            .iter()
            .map(|d| d.summary())
            .collect::<Vec<_>>()
            .join(", ")
    };
    card.push(("Displays", displays));
    page = page.child(section("Graphics Card", &card, cx));

    let mut mem = vec![
        ("Dedicated Memory", format_bytes(g.dedicated_vram)),
        ("Shared System Memory", format_bytes(g.shared_memory)),
    ];
    if let Some(n) = nv {
        if let Some(w) = n.memory_bus_width {
            mem.push(("Bus Width", format!("{w} bit")));
        }
        if let Some(c) = n.cuda_cores {
            mem.push(("CUDA Cores", c.to_string()));
        }
        if let Some(c) = n.max_core_clock_mhz {
            mem.push(("Max GPU Clock", format!("{c} MHz")));
        }
        if let Some(c) = n.max_mem_clock_mhz {
            mem.push(("Max Memory Clock", format!("{c} MHz")));
        }
        if let Some(p) = n.power_limit_w {
            mem.push(("Power Limit", format!("{p:.0} W")));
        }
    }
    page = page.child(section("Memory & Limits", &mem, cx));

    let prefix = g.sensor_prefix();
    let rows: Vec<_> = snap
        .table
        .entries()
        .iter()
        .filter(|t| t.reading.id.starts_with(&prefix))
        .collect();
    if rows.is_empty() {
        page = page.child(ui::label("Waiting for first sample..."));
    } else {
        page = page
            .child(ui::strong("Sensors"))
            .child(super::sensors::sensor_grid(
                "gpu-sensors",
                rows.into_iter(),
                cx,
            ));
    }
    if nv.is_none() && g.vendor_id != omnihw_core::gpu::VENDOR_NVIDIA {
        page = page.child(ui::space(6.0)).child(ui::weak(
            "Temperatures, clocks and power for AMD (ADLX) and Intel (IGCL) GPUs are planned.",
            cx,
        ));
    }
    page.into_any_element()
}
