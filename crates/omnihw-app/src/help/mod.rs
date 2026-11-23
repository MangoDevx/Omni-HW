//! In-app help. PENDING PORT: the egui version is under
//! `git show main:crates/omnihw-app/src/help/` (mod.rs, start.rs, hardware.rs, health.rs,
//! monitoring.rs).

use gpui_kit::App;

/// Help window state.
#[derive(Default)]
pub struct HelpState {}

impl HelpState {
    /// Whether the help window is open (the sidebar highlights its row).
    pub fn is_open(&self) -> bool {
        false
    }

    /// Open the help window, or close it if it is open.
    pub fn toggle(&mut self, _cx: &mut App) {}
}
