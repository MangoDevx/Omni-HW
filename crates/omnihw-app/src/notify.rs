//! Desktop notifications for fired alerts, through the freedesktop notification service on
//! D-Bus, plus an attention request (taskbar highlight) when the window isn't focused.

#[derive(Default)]
pub struct Notifier;

impl Notifier {
    pub fn show(&mut self, ctx: &egui::Context, title: &str, message: &str) {
        let (title, message) = (title.to_string(), message.to_string());
        // The D-Bus call can block for a while when no notification daemon runs; keep it off
        // the sampler thread.
        let _ = std::thread::Builder::new()
            .name("omnihw-notify".into())
            .spawn(move || {
                let _ = notify_rust::Notification::new()
                    .appname("Omni-HW")
                    .summary(&title)
                    .body(&message)
                    .icon("omnihw")
                    .show();
            });
        if ctx.input(|i| i.viewport().focused) != Some(true) {
            ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(
                egui::UserAttentionType::Informational,
            ));
        }
    }
}
