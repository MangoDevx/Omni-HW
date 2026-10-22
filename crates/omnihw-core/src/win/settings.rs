//! Windows configuration the Health Check looked at (pending restarts, VBS, GPU scheduling,
//! Game Mode, pagefile). Linux has none of it, so every field stays "couldn't tell".

/// Settings relevant to the Health Check. `None` means "couldn't tell".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WindowsSettings {
    /// Why a restart is pending (empty when none is).
    pub pending_reboot: Vec<&'static str>,
    /// Memory Integrity (HVCI) is running right now.
    pub hvci_running: Option<bool>,
    /// Virtualization-based security is turned on in policy/registry.
    pub vbs_configured: Option<bool>,
    /// Hardware-accelerated GPU scheduling; `None` when the GPU/driver doesn't offer it.
    pub hags: Option<bool>,
    pub game_mode: Option<bool>,
    /// No pagefile on any drive.
    pub pagefile_disabled: Option<bool>,
}

pub fn windows_settings() -> WindowsSettings {
    WindowsSettings::default()
}
