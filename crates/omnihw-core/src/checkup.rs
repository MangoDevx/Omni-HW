//! Data for the setup checks of the Health Check: firmware security state, wired network link
//! speeds and NVMe PCIe links.
//!
//! Gathered once at startup on the background thread (see [`crate::platform::checkup`]); the checks
//! in [`crate::diagnostics`] only read it.

/// Everything the setup checks look at. Empty/`None` means "couldn't tell".
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Checkup {
    pub firmware: Firmware,
    /// Connected wired network adapters.
    pub nics: Vec<NicLink>,
    /// PCIe links of NVMe drives.
    pub nvme_links: Vec<PcieLink>,
}

/// Boot mode and platform security state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Firmware {
    /// Booted through UEFI (false = legacy BIOS / CSM).
    pub uefi: Option<bool>,
    pub secure_boot: Option<bool>,
    pub tpm: Tpm,
}

/// TPM presence and version.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Tpm {
    #[default]
    Unknown,
    Missing,
    V12,
    V20,
}

/// A connected wired network adapter.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NicLink {
    pub name: String,
    /// Negotiated link speed in bits per second.
    pub speed_bps: u64,
}

/// "100 Mbps", "1 Gbps", "2.5 Gbps".
pub fn format_link_speed(bps: u64) -> String {
    if bps >= 1_000_000_000 {
        let g = bps as f64 / 1e9;
        if g.fract() == 0.0 {
            format!("{g:.0} Gbps")
        } else {
            format!("{g:.1} Gbps")
        }
    } else {
        format!("{} Mbps", bps / 1_000_000)
    }
}

/// A PCIe link as (generation, lanes).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PcieLink {
    /// Drive model.
    pub device: String,
    pub current: (u32, u32),
    pub max: (u32, u32),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_speeds() {
        assert_eq!(format_link_speed(100_000_000), "100 Mbps");
        assert_eq!(format_link_speed(1_000_000_000), "1 Gbps");
        assert_eq!(format_link_speed(2_500_000_000), "2.5 Gbps");
    }
}
