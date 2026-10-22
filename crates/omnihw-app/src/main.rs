mod app;
mod autostart;
mod dismissed;
mod elevate;
mod export;
mod folder_picker;
mod heat;
mod help;
mod icon;
mod nav;
mod notify;
mod retro;
mod sampler;
mod settings;
mod tabs;
mod update;

fn main() -> eframe::Result {
    // The root helper (started through pkexec): print the root-only data and exit.
    if let Some(code) = elevate::run_helper() {
        std::process::exit(code);
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
