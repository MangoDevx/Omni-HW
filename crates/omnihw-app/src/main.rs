// Hide the console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod autostart;
mod dismissed;
mod dump_ops;
mod elevate;
mod export;
mod folder_picker;
mod heat;
mod help;
mod nav;
mod net_ops;
mod notify;
mod pawnio;
mod power_ops;
mod repaint;
mod retro;
mod sampler;
mod sensor_service;
mod settings;
mod tabs;
mod theme;
mod tray;
mod ui;
mod update;

use gpui_kit::{AppContext, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size};

fn main() {
    // Elevated helper for a power change: do it and exit without a window.
    if let Some(code) = power_ops::run_helper() {
        std::process::exit(code);
    }
    // Elevated helper for a network change.
    if let Some(code) = net_ops::run_helper() {
        std::process::exit(code);
    }
    // Elevated helper that reads crash dumps.
    if let Some(code) = dump_ops::run_helper() {
        std::process::exit(code);
    }
    // The sensor service (started by Windows), and the elevated helper that sets it up.
    if let Some(code) = sensor_service::run_if_service() {
        std::process::exit(code);
    }
    if let Some(code) = pawnio::run_helper() {
        std::process::exit(code);
    }
    if elevate::relaunch_elevated_if_useful() {
        return;
    }
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(|cx| {
            gpui_kit::init(cx);
            app::bind_keys(cx);
            let [width, height] = initial_size();
            let bounds = Bounds::centered(None, size(px(width), px(height)), cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("Omni-HW".into()),
                    ..Default::default()
                }),
                window_min_size: Some(size(px(440.0), px(400.0))),
                ..Default::default()
            };
            let (window, app) = gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| app::OmniApp::new(window, cx))
            })
            .expect("open the main window");
            // Closing the window ends the app: remove the tray icon and save settings first.
            let _ = window.update(cx, |_, window, cx| {
                window.on_window_should_close(cx, move |_, cx| {
                    app.update(cx, |app, _| app.shutdown());
                    true
                });
            });
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            cx.activate(true);
        });
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
