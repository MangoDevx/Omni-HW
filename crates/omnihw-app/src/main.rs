mod app;
mod autostart;
mod dismissed;
mod dump_ops;
mod elevate;
mod export;
mod folder_picker;
mod heat;
mod help;
mod icon;
mod nav;
mod net_ops;
mod notify;
mod power_ops;
mod retro;
mod sampler;
mod settings;
mod tabs;
mod update;

fn main() -> eframe::Result {
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
    if elevate::relaunch_elevated_if_useful() {
        return Ok(());
    }
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Omni-HW")
        // Wayland app id / X11 WM_CLASS, matching packaging/linux/omnihw.desktop.
        .with_app_id("omnihw")
        // Window managers shrink it to fit smaller screens.
        .with_inner_size([1080.0, 900.0])
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
