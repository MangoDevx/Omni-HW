// Hide the console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod autostart;
mod elevate;
mod export;
mod heat;
mod help;
mod icon;
mod notify;
mod power_ops;
mod sampler;
mod settings;
mod tabs;
mod tray;
mod update;

fn main() -> eframe::Result {
    // Elevated helper for a power change: do it and exit without a window.
    if let Some(code) = power_ops::run_helper() {
        std::process::exit(code);
    }
    if elevate::relaunch_elevated_if_useful() {
        return Ok(());
    }
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Omni-HW")
        .with_inner_size(initial_size())
        .with_min_inner_size([440.0, 400.0]);
    if let Some(icon) = icon::window_icon() {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "Omni-HW",
        options,
        Box::new(|cc| Ok(Box::new(app::OmniApp::new(cc)))),
    )
}

/// Preferred window size (logical points), shrunk to fit the screen's work area so the
/// Overview fits without scrolling on typical displays.
fn initial_size() -> [f32; 2] {
    const PREFERRED: [f32; 2] = [1080.0, 900.0];
    match work_area_points() {
        Some([w, h]) => [PREFERRED[0].min(w * 0.92), PREFERRED[1].min(h * 0.92)],
        None => PREFERRED,
    }
}

#[cfg(windows)]
fn work_area_points() -> Option<[f32; 2]> {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::UI::HiDpi::GetDpiForSystem;
    use windows::Win32::UI::WindowsAndMessaging::{
        SPI_GETWORKAREA, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
    };
    let mut r = RECT::default();
    // SAFETY: valid RECT out pointer.
    unsafe {
        SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            Some((&raw mut r).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    }
    .ok()?;
    // SAFETY: trivially safe.
    let scale = unsafe { GetDpiForSystem() }.max(96) as f32 / 96.0;
    Some([
        (r.right - r.left) as f32 / scale,
        (r.bottom - r.top) as f32 / scale,
    ])
}

#[cfg(not(windows))]
fn work_area_points() -> Option<[f32; 2]> {
    None
}
