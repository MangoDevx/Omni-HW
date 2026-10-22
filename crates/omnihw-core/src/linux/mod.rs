//! Linux data sources: sysfs, procfs and udev's database.
//!
//! Everything reads files under a [`Root`] (`/` on a real system), so the readers are
//! unit-tested against fake trees.

pub mod block;
pub mod dmi;
pub mod drm;
pub mod nvme;
pub mod os;
pub mod pciids;
pub mod power_supply;
pub mod sensors;
pub mod topology;

use std::path::{Path, PathBuf};

/// The filesystem root the readers look under.
#[derive(Debug, Clone)]
pub struct Root(PathBuf);

impl Root {
    /// The real system root, `/`.
    pub fn system() -> Root {
        Root(PathBuf::from("/"))
    }

    /// A fake root, for tests.
    pub fn at(path: impl Into<PathBuf>) -> Root {
        Root(path.into())
    }

    /// `rel` (absolute or relative) resolved under this root.
    pub fn path(&self, rel: &str) -> PathBuf {
        self.0.join(rel.trim_start_matches('/'))
    }

    pub fn exists(&self, rel: &str) -> bool {
        self.path(rel).exists()
    }

    /// File contents with surrounding whitespace trimmed; `None` if unreadable.
    pub fn read(&self, rel: &str) -> Option<String> {
        let s = std::fs::read_to_string(self.path(rel)).ok()?;
        Some(s.trim().to_string())
    }

    /// Like [`Root::read`], but `None` for empty files too.
    pub fn read_nonempty(&self, rel: &str) -> Option<String> {
        self.read(rel).filter(|s| !s.is_empty())
    }

    pub fn read_bytes(&self, rel: &str) -> Option<Vec<u8>> {
        std::fs::read(self.path(rel)).ok()
    }

    pub fn read_u64(&self, rel: &str) -> Option<u64> {
        parse_int(&self.read(rel)?)
    }

    pub fn read_i64(&self, rel: &str) -> Option<i64> {
        self.read(rel)?.parse().ok()
    }

    /// Entry names of a directory, in natural order ("cpu2" before "cpu10").
    pub fn list(&self, rel: &str) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.path(rel))
            .map(|d| {
                d.filter_map(|e| e.ok())
                    .filter_map(|e| e.file_name().into_string().ok())
                    .collect()
            })
            .unwrap_or_default();
        names.sort_by(|a, b| natural_cmp(a, b));
        names
    }

    /// File name of a symlink's target, e.g. the driver behind `device/driver`.
    pub fn link_name(&self, rel: &str) -> Option<String> {
        let target = std::fs::read_link(self.path(rel)).ok()?;
        Some(target.file_name()?.to_string_lossy().into_owned())
    }
}

impl AsRef<Path> for Root {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

/// Decimal or `0x` hex integer.
pub fn parse_int(s: &str) -> Option<u64> {
    let s = s.trim();
    match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        Some(h) => u64::from_str_radix(h, 16).ok(),
        None => s.parse().ok(),
    }
}

/// A kernel CPU list such as "0-3,8,10-11".
pub fn parse_cpu_list(s: &str) -> Vec<u32> {
    let mut out = Vec::new();
    for part in s.trim().split(',').filter(|p| !p.is_empty()) {
        match part.split_once('-') {
            Some((a, b)) => {
                if let (Ok(a), Ok(b)) = (a.trim().parse::<u32>(), b.trim().parse::<u32>()) {
                    out.extend(a..=b.min(a.saturating_add(4096)));
                }
            }
            None => out.extend(part.trim().parse::<u32>().ok()),
        }
    }
    out
}

/// `KEY=value` lines (uevent files, os-release); surrounding quotes are removed.
pub fn parse_key_values(s: &str) -> Vec<(String, String)> {
    s.lines()
        .filter_map(|l| {
            let (k, v) = l.trim().split_once('=')?;
            let v = v.trim();
            let v = v
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .or_else(|| v.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
                .unwrap_or(v);
            Some((k.trim().to_string(), v.to_string()))
        })
        .collect()
}

/// Value of `key` in `KEY=value` text.
pub fn key_value(s: &str, key: &str) -> Option<String> {
    parse_key_values(s)
        .into_iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v)
}

/// Compare strings with embedded numbers numerically.
pub fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    fn split(s: &str) -> (&str, Option<u64>) {
        let digits = s.len() - s.trim_end_matches(|c: char| c.is_ascii_digit()).len();
        let (head, tail) = s.split_at(s.len() - digits);
        (head, tail.parse().ok())
    }
    let (ha, na) = split(a);
    let (hb, nb) = split(b);
    ha.cmp(hb).then(na.cmp(&nb)).then(a.cmp(b))
}

/// The Linux implementations behind `crate::win`'s cross-platform API.
pub mod imp {
    use super::Root;
    use crate::win::{Battery, MemoryStatus, OsInfo, PowerPlan};

    /// DRM adapters, with NVML details (and NVML's names and VRAM) on NVIDIA cards.
    pub fn gpu_adapters() -> Vec<crate::gpu::GpuAdapter> {
        let mut gpus = super::drm::gpu_adapters(&Root::system());
        crate::win::nvml::attach_details(&mut gpus);
        for g in &mut gpus {
            if let Some(n) = &g.nvml {
                if !n.name.is_empty() {
                    g.name = n.name.clone();
                }
                if g.dedicated_vram == 0 {
                    g.dedicated_vram = n.memory_total;
                }
                if !n.driver_version.is_empty() {
                    g.driver_version = n.driver_version.clone();
                }
            }
        }
        gpus
    }
    pub fn drives() -> Vec<crate::storage::Drive> {
        super::block::drives(&Root::system())
    }
    /// SATA SMART needs root and an ATA passthrough ioctl, not wired up yet.
    pub fn has_smart_capable_drive() -> bool {
        false
    }
    /// Running as root (effective uid 0).
    pub fn is_elevated() -> bool {
        super::os::is_root(&Root::system())
    }
    pub fn os_info() -> Option<OsInfo> {
        super::os::os_info(&Root::system())
    }
    pub fn uptime_secs() -> Option<u64> {
        super::os::uptime_secs(&Root::system())
    }
    pub fn battery() -> Option<Battery> {
        super::power_supply::battery(&Root::system())
    }
    pub fn microcode_revision() -> Option<u32> {
        super::os::microcode(&Root::system())
    }
    /// Linux has no Windows-style power plans.
    pub fn power_plan() -> Option<PowerPlan> {
        None
    }
    pub fn smbios_buffer() -> Option<Vec<u8>> {
        super::dmi::raw_smbios(&Root::system())
    }
    pub fn nominal_mhz() -> Option<u32> {
        super::os::nominal_mhz(&Root::system())
    }
    pub fn memory_status() -> Option<MemoryStatus> {
        super::os::memory_status(&Root::system())
    }
    pub fn battery_health() -> Option<crate::battery::BatteryHealth> {
        super::power_supply::battery_health(&Root::system())
    }
}

/// A throwaway directory tree standing in for `/`.
#[cfg(test)]
pub(crate) struct Fixture {
    dir: tempfile::TempDir,
}

#[cfg(test)]
impl Fixture {
    pub fn new() -> Fixture {
        Fixture {
            dir: tempfile::tempdir().expect("temp dir"),
        }
    }

    pub fn root(&self) -> Root {
        Root::at(self.dir.path())
    }

    /// Write `contents` to `rel`, creating parent directories.
    pub fn file(&self, rel: &str, contents: impl AsRef<[u8]>) -> &Fixture {
        let p = self.root().path(rel);
        std::fs::create_dir_all(p.parent().expect("parent")).expect("mkdir");
        std::fs::write(p, contents).expect("write");
        self
    }

    pub fn dir(&self, rel: &str) -> &Fixture {
        std::fs::create_dir_all(self.root().path(rel)).expect("mkdir");
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_lists() {
        assert_eq!(parse_cpu_list("0-3,8,10-11\n"), vec![0, 1, 2, 3, 8, 10, 11]);
        assert_eq!(parse_cpu_list("5"), vec![5]);
        assert_eq!(parse_cpu_list(""), Vec::<u32>::new());
    }

    #[test]
    fn ints() {
        assert_eq!(parse_int("0x1f"), Some(31));
        assert_eq!(parse_int(" 42\n"), Some(42));
        assert_eq!(parse_int("x"), None);
    }

    #[test]
    fn key_values() {
        let s = "NAME=\"Ubuntu\"\nVERSION_ID='24.04'\nID=ubuntu\n# comment\n";
        assert_eq!(key_value(s, "NAME").as_deref(), Some("Ubuntu"));
        assert_eq!(key_value(s, "VERSION_ID").as_deref(), Some("24.04"));
        assert_eq!(key_value(s, "ID").as_deref(), Some("ubuntu"));
        assert_eq!(key_value(s, "MISSING"), None);
    }

    #[test]
    fn natural_order() {
        let mut v = vec!["cpu10", "cpu2", "cpu1", "cpufreq"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, vec!["cpu1", "cpu2", "cpu10", "cpufreq"]);
    }

    #[test]
    fn root_reads() {
        let f = Fixture::new();
        f.file("proc/x", " 12 \n")
            .dir("sys/a/cpu10")
            .dir("sys/a/cpu2");
        let r = f.root();
        assert_eq!(r.read_u64("/proc/x"), Some(12));
        assert_eq!(r.list("sys/a"), vec!["cpu2", "cpu10"]);
        assert_eq!(r.read("nope"), None);
    }
}
