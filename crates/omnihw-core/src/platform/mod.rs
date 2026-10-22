//! Platform data sources behind one API, implemented by the readers in [`crate::linux`].

pub mod events_viewer;
pub mod nvml;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OsInfo {
    /// The distribution, e.g. "Ubuntu 24.04.1 LTS".
    pub product: String,
    pub computer_name: String,
    /// Kernel release, e.g. "6.8.0-45-generic".
    pub kernel: String,
    /// Which family of distributions it belongs to, for package-manager commands in fix steps.
    pub distro: Distro,
}

/// A family of distributions sharing a package manager.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Distro {
    /// Ubuntu and its derivatives (Mint, Pop!_OS, Zorin, elementary).
    Ubuntu,
    Debian,
    /// Arch and its derivatives (CachyOS, EndeavourOS, Manjaro).
    Arch,
    /// Fedora, RHEL and their rebuilds.
    Fedora,
    /// openSUSE and SUSE Linux Enterprise.
    Suse,
    #[default]
    Other,
}

impl Distro {
    /// From os-release's `ID` and `ID_LIKE` (a space-separated list of parents).
    pub fn from_ids(id: &str, id_like: &str) -> Distro {
        let ids: Vec<&str> = std::iter::once(id)
            .chain(id_like.split_whitespace())
            .collect();
        let has = |names: &[&str]| ids.iter().any(|i| names.contains(i));
        if has(&["ubuntu"]) {
            Distro::Ubuntu
        } else if has(&["debian"]) {
            Distro::Debian
        } else if has(&["arch", "archlinux"]) {
            Distro::Arch
        } else if has(&["fedora", "rhel", "centos"]) {
            Distro::Fedora
        } else if has(&["suse", "opensuse", "sles"]) || id.starts_with("opensuse") {
            Distro::Suse
        } else {
            Distro::Other
        }
    }

    /// The command that installs `pkg` (the package's name on this family).
    pub fn install(self, pkg: &str) -> String {
        match self {
            Distro::Ubuntu | Distro::Debian => format!("sudo apt install {pkg}"),
            Distro::Arch => format!("sudo pacman -S {pkg}"),
            Distro::Fedora => format!("sudo dnf install {pkg}"),
            Distro::Suse => format!("sudo zypper install {pkg}"),
            Distro::Other => format!("install the {pkg} package"),
        }
    }

    /// The command that installs every update.
    pub fn upgrade(self) -> &'static str {
        match self {
            Distro::Ubuntu | Distro::Debian => "sudo apt update && sudo apt full-upgrade",
            Distro::Arch => "sudo pacman -Syu",
            Distro::Fedora => "sudo dnf upgrade --refresh",
            Distro::Suse => "sudo zypper dup on Tumbleweed, sudo zypper update on Leap",
            Distro::Other => "your distribution's updater",
        }
    }

    /// The command that empties the package manager's download cache.
    pub fn clean_cache(self) -> &'static str {
        match self {
            Distro::Ubuntu | Distro::Debian => "sudo apt clean",
            Distro::Arch => "sudo pacman -Sc",
            Distro::Fedora => "sudo dnf clean all",
            Distro::Suse => "sudo zypper clean",
            Distro::Other => "your package manager's clean command",
        }
    }
}

impl OsInfo {
    /// "Ubuntu 24.04.1 LTS (kernel 6.8.0-45-generic)".
    pub fn summary(&self) -> String {
        if self.kernel.is_empty() {
            return self.product.clone();
        }
        format!("{} (kernel {})", self.product, self.kernel)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Battery {
    pub percent: u8,
    pub on_ac: bool,
    pub charging: bool,
    pub seconds_left: Option<u32>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MemoryStatus {
    pub load_percent: u32,
    pub total_bytes: u64,
    pub available_bytes: u64,
}

use crate::linux::imp;

/// Battery design vs full-charge capacity (None on desktops).
pub use imp::battery_health;
/// Physical drives with volumes (and NVMe health when permitted).
pub use imp::drives;
/// Display adapters (DRM), with NVML details on NVIDIA cards.
pub use imp::gpu_adapters;
/// Whether this process runs as root.
pub use imp::is_elevated;
pub use imp::memory_status;
/// Nominal (base) clock in MHz.
pub use imp::nominal_mhz;
/// Raw SMBIOS table wrapped as RawSMBIOSData (root only).
pub use imp::smbios_buffer;
pub use imp::{battery, microcode_revision, os_info, uptime_secs};

/// Data collectors for the setup checks of the Health Check.
pub mod checkup;

#[cfg(test)]
mod tests {
    use super::Distro;

    #[test]
    fn distro_from_os_release_ids() {
        assert_eq!(Distro::from_ids("ubuntu", "debian"), Distro::Ubuntu);
        assert_eq!(
            Distro::from_ids("linuxmint", "ubuntu debian"),
            Distro::Ubuntu
        );
        assert_eq!(Distro::from_ids("debian", ""), Distro::Debian);
        assert_eq!(Distro::from_ids("cachyos", "arch"), Distro::Arch);
        assert_eq!(Distro::from_ids("arch", ""), Distro::Arch);
        assert_eq!(Distro::from_ids("fedora", ""), Distro::Fedora);
        assert_eq!(
            Distro::from_ids("rocky", "rhel centos fedora"),
            Distro::Fedora
        );
        assert_eq!(
            Distro::from_ids("opensuse-tumbleweed", "opensuse suse"),
            Distro::Suse
        );
        assert_eq!(Distro::from_ids("nixos", ""), Distro::Other);
        assert_eq!(Distro::Arch.install("iw"), "sudo pacman -S iw");
    }
}
