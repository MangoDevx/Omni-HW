//! Quick disk speed self-test: sequential and 4K random read/write through a temporary file,
//! compared with what the drive and its link should deliver. This module picks the target,
//! sizes the test and judges the result; the I/O runs in [`crate::linux::diskio`].

use crate::checkup::PcieLink;
use crate::storage::{BusType, Drive, Volume, sata_gen_speed};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, AtomicU32, Ordering};
use std::time::Duration;

const MIB: u64 = 1 << 20;
const GIB: u64 = 1 << 30;
/// Free space left over on the volume after the test file is written.
const SPARE_BYTES: u64 = 2 * GIB;

/// Measured speeds, MB/s (10^6 bytes per second).
#[derive(Debug, Clone)]
pub struct DiskTestResult {
    pub seq_read_mbs: f64,
    pub seq_write_mbs: f64,
    /// 4K random, queue depth 1.
    pub rnd4k_read_mbs: f64,
    pub rnd4k_write_mbs: f64,
    /// 4K random read, queue depth 32.
    pub rnd4k_read_q32_mbs: f64,
    /// Size of the test file in MiB.
    pub file_mb: u64,
    /// Where the temporary file was written.
    pub path: String,
    pub expected: Option<DiskExpectation>,
}

/// Typical sequential read range for a drive, and why.
#[derive(Debug, Clone, PartialEq)]
pub struct DiskExpectation {
    /// (low, high) MB/s.
    pub seq_read_mbs: (f64, f64),
    /// e.g. "PCIe 4.0 x4 NVMe", "SATA 6 Gb/s SSD", "hard drive".
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Within (or above) the typical range.
    Good,
    /// Below the range but within 60% of it.
    Low,
    VeryLow,
}

impl DiskTestResult {
    /// Sequential read against the expected range; `None` when there is no expectation.
    pub fn verdict(&self) -> Option<Verdict> {
        let low = self.expected.as_ref()?.seq_read_mbs.0;
        Some(if self.seq_read_mbs >= low {
            Verdict::Good
        } else if self.seq_read_mbs >= 0.6 * low {
            Verdict::Low
        } else {
            Verdict::VeryLow
        })
    }
}

/// Which stage the test is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Writing the test file once so every later read hits real data.
    Preparing,
    SeqWrite,
    SeqRead,
    RandomReadQ32,
    RandomRead,
    RandomWrite,
    Done,
}

impl Phase {
    const ALL: [Phase; 7] = [
        Phase::Preparing,
        Phase::SeqWrite,
        Phase::SeqRead,
        Phase::RandomReadQ32,
        Phase::RandomRead,
        Phase::RandomWrite,
        Phase::Done,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Phase::Preparing => "Preparing test file",
            Phase::SeqWrite => "Sequential write",
            Phase::SeqRead => "Sequential read",
            Phase::RandomReadQ32 => "4K random read (QD32)",
            Phase::RandomRead => "4K random read",
            Phase::RandomWrite => "4K random write",
            Phase::Done => "Cleaning up",
        }
    }
}

/// Shared progress a running test updates and the UI reads.
#[derive(Debug, Default)]
pub struct Progress {
    phase: AtomicU8,
    /// Progress through the current phase, 0..=1000.
    done: AtomicU32,
}

impl Progress {
    pub(crate) fn start(&self, phase: Phase) {
        self.done.store(0, Ordering::Relaxed);
        self.phase.store(phase as u8, Ordering::Relaxed);
    }

    /// Fraction of the current phase done, 0..=1.
    pub(crate) fn set(&self, frac: f64) {
        self.done
            .store((frac.clamp(0.0, 1.0) * 1000.0) as u32, Ordering::Relaxed);
    }

    /// (phase, fraction of the whole test done) for the UI.
    pub fn read(&self) -> (Phase, f32) {
        let i = (self.phase.load(Ordering::Relaxed) as usize).min(Phase::ALL.len() - 1);
        let within = self.done.load(Ordering::Relaxed).min(1000) as f32 / 1000.0;
        let steps = (Phase::ALL.len() - 1) as f32;
        (Phase::ALL[i], ((i as f32 + within) / steps).min(1.0))
    }
}

fn is_hdd(drive: &Drive) -> bool {
    drive.bus != BusType::Nvme && drive.ssd == Some(false)
}

/// Test file size: 1 GiB, or 512 MiB on small drives and hard drives to keep it quick.
pub fn file_bytes(drive: &Drive) -> u64 {
    if drive.size_bytes < 64_000_000_000 || is_hdd(drive) {
        512 * MIB
    } else {
        GIB
    }
}

/// Free space a volume needs for the test.
pub fn needed_free_bytes(drive: &Drive) -> u64 {
    file_bytes(drive) + SPARE_BYTES
}

/// Longest time any one phase may run.
fn phase_cap(drive: &Drive) -> Duration {
    Duration::from_secs(if is_hdd(drive) { 5 } else { 3 })
}

/// The `PcieLink` reported for this NVMe drive (matched by model name).
pub fn nvme_link<'a>(drive: &Drive, links: &'a [PcieLink]) -> Option<&'a PcieLink> {
    if drive.bus != BusType::Nvme {
        return None;
    }
    let model = drive.model.trim().to_ascii_lowercase();
    let name = drive.name().to_ascii_lowercase();
    if model.is_empty() {
        return None;
    }
    links.iter().find(|l| {
        let d = l.device.trim().to_ascii_lowercase();
        !d.is_empty() && (d == model || d == name || d.contains(&model) || model.contains(&d))
    })
}

/// MB/s per lane by PCIe generation (after encoding overhead).
fn pcie_lane_mbs(generation: u32) -> Option<f64> {
    Some(match generation {
        1 => 250.0,
        2 => 500.0,
        3 => 985.0,
        4 => 1969.0,
        5 => 3938.0,
        _ => return None,
    })
}

/// Typical sequential read range for `drive`; `None` for USB, card readers, virtual, RAID and
/// unknown disks, where it depends on the port or enclosure.
pub fn expectation(drive: &Drive, nvme_link: Option<&PcieLink>) -> Option<DiskExpectation> {
    let range = |lo: f64, hi: f64, reason: String| DiskExpectation {
        seq_read_mbs: (lo, hi),
        reason,
    };
    match (drive.bus, drive.ssd) {
        (BusType::Nvme, _) => {
            let known = nvme_link.and_then(|l| {
                let (generation, lanes) = l.current;
                let per_lane = pcie_lane_mbs(generation)?;
                (lanes > 0).then_some((generation, lanes, per_lane * lanes as f64))
            });
            Some(match known {
                Some((generation, lanes, ceiling)) => range(
                    0.4 * ceiling,
                    0.95 * ceiling,
                    format!("PCIe {generation}.0 x{lanes} NVMe"),
                ),
                None => range(1500.0, 7500.0, "NVMe SSD".into()),
            })
        }
        (BusType::Sata | BusType::Ata, Some(true)) => {
            let generation = drive.sata_link.map_or(3, |l| l.current_gen);
            let (lo, hi) = match generation {
                1 => (100.0, 140.0),
                2 => (200.0, 280.0),
                _ => (400.0, 560.0),
            };
            let reason = if drive.sata_link.is_some() {
                format!("SATA {} SSD", sata_gen_speed(generation))
            } else {
                "SATA SSD".into()
            };
            Some(range(lo, hi, reason))
        }
        (BusType::Sata | BusType::Ata | BusType::Sas | BusType::Scsi, Some(false)) => {
            Some(range(80.0, 280.0, "hard drive".into()))
        }
        _ => None,
    }
}

/// File systems the test writes to (all support direct I/O).
const FILE_SYSTEMS: [&str; 9] = [
    "ext4", "ext3", "xfs", "btrfs", "f2fs", "jfs", "exfat", "vfat", "ntfs3",
];

/// The mount point holding `path`: the longest mount that is `path` or one of its parents.
fn mount_of<'a>(path: &Path, mounts: impl Iterator<Item = &'a str>) -> Option<&'a str> {
    mounts
        .filter(|m| path.starts_with(m))
        .max_by_key(|m| m.len())
}

/// A folder on `volume` the test file can go in: the home folder when it's on this volume,
/// else the mount point itself if it's writable.
fn target_dir(volume: &Volume) -> Option<PathBuf> {
    use std::os::unix::fs::MetadataExt;
    let mount = Path::new(&volume.mount);
    let dev = std::fs::metadata(mount).ok()?.dev();
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from)
        && std::fs::metadata(&home).is_ok_and(|m| m.dev() == dev)
    {
        return Some(home);
    }
    rustix::fs::access(mount, rustix::fs::Access::WRITE_OK)
        .is_ok()
        .then(|| mount.to_path_buf())
}

/// A volume on `drive` for the test file: mounted, a supported file system, room for the file
/// plus 2 GiB, and `writable`. Prefers the volume holding `home`.
fn pick_volume_on<'a>(
    drive: &'a Drive,
    home: Option<&Path>,
    writable: impl Fn(&Volume) -> bool,
) -> Option<&'a Volume> {
    let need = needed_free_bytes(drive);
    let ok: Vec<&Volume> = drive
        .volumes
        .iter()
        .filter(|v| {
            !v.mount.is_empty()
                && FILE_SYSTEMS
                    .iter()
                    .any(|f| f.eq_ignore_ascii_case(v.file_system.trim()))
                && v.free_bytes >= need
                && writable(v)
        })
        .collect();
    let home_mount = home.and_then(|h| mount_of(h, ok.iter().map(|v| v.mount.as_str())));
    ok.iter()
        .find(|v| Some(v.mount.as_str()) == home_mount)
        .or(ok.first())
        .copied()
}

/// A writable volume on `drive` for the test file. Prefers the one holding the home folder.
pub fn pick_volume(drive: &Drive) -> Option<&Volume> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    pick_volume_on(drive, home.as_deref(), |v| target_dir(v).is_some())
}

/// Run the test on `drive`, reporting progress as it goes. Blocks for roughly 10-30 seconds;
/// call it off the UI thread. The temporary file is deleted on every exit path.
pub fn run(
    drive: &Drive,
    expected: Option<DiskExpectation>,
    progress: &Progress,
) -> Result<DiskTestResult, String> {
    let bytes = file_bytes(drive);
    let Some(volume) = pick_volume(drive) else {
        return Err(format!(
            "No volume on this drive with enough free space (needs {:.1} GB free)",
            needed_free_bytes(drive) as f64 / 1e9
        ));
    };
    let dir =
        target_dir(volume).ok_or_else(|| format!("Can't write a test file to {}", volume.mount))?;
    let path = dir.join(format!(".omnihw-disktest-{}.tmp", std::process::id()));
    let speeds = crate::linux::diskio::measure(
        &path,
        &volume.mount,
        bytes,
        needed_free_bytes(drive),
        phase_cap(drive),
        progress,
    )?;
    progress.start(Phase::Done);
    Ok(DiskTestResult {
        seq_read_mbs: speeds.seq_read,
        seq_write_mbs: speeds.seq_write,
        rnd4k_read_mbs: speeds.rnd_read,
        rnd4k_write_mbs: speeds.rnd_write,
        rnd4k_read_q32_mbs: speeds.rnd_read_q32,
        file_mb: bytes / MIB,
        path: path.display().to_string(),
        expected,
    })
}

/// Raw MB/s of each phase.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Speeds {
    pub seq_write: f64,
    pub seq_read: f64,
    pub rnd_read_q32: f64,
    pub rnd_read: f64,
    pub rnd_write: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::SataLink;

    fn vol(mount: &str, fs: &str, free_gb: u64) -> Volume {
        Volume {
            mount: mount.into(),
            file_system: fs.into(),
            total_bytes: 1000 * GIB,
            free_bytes: free_gb * GIB,
            ..Default::default()
        }
    }

    fn drive(bus: BusType, ssd: Option<bool>) -> Drive {
        Drive {
            model: "Samsung SSD 990 PRO 2TB".into(),
            bus,
            ssd,
            size_bytes: 2000 * GIB,
            ..Default::default()
        }
    }

    fn link(current: (u32, u32)) -> PcieLink {
        PcieLink {
            device: "Samsung SSD 990 PRO 2TB".into(),
            current,
            max: (4, 4),
        }
    }

    #[test]
    fn nvme_expectations() {
        let d = drive(BusType::Nvme, Some(true));
        let e = expectation(&d, Some(&link((4, 4)))).unwrap();
        assert_eq!(e.reason, "PCIe 4.0 x4 NVMe");
        assert!((e.seq_read_mbs.0 - 0.4 * 7876.0).abs() < 1e-6);
        assert!((e.seq_read_mbs.1 - 0.95 * 7876.0).abs() < 1e-6);
        let e = expectation(&d, Some(&link((3, 2)))).unwrap();
        assert_eq!(e.reason, "PCIe 3.0 x2 NVMe");
        assert!((e.seq_read_mbs.1 - 0.95 * 1970.0).abs() < 1e-6);
        // Unknown or unreadable link.
        for l in [None, Some(link((0, 4))), Some(link((4, 0)))] {
            let e = expectation(&d, l.as_ref()).unwrap();
            assert_eq!(
                (e.seq_read_mbs, e.reason.as_str()),
                ((1500.0, 7500.0), "NVMe SSD")
            );
        }
    }

    #[test]
    fn sata_hdd_and_other_expectations() {
        let mut d = drive(BusType::Sata, Some(true));
        let e = expectation(&d, None).unwrap();
        assert_eq!(
            (e.seq_read_mbs, e.reason.as_str()),
            ((400.0, 560.0), "SATA SSD")
        );
        d.sata_link = Some(SataLink {
            current_gen: 2,
            max_gen: 3,
        });
        let e = expectation(&d, None).unwrap();
        assert_eq!(
            (e.seq_read_mbs, e.reason.as_str()),
            ((200.0, 280.0), "SATA 3 Gb/s SSD")
        );
        d.sata_link = Some(SataLink {
            current_gen: 1,
            max_gen: 1,
        });
        assert_eq!(expectation(&d, None).unwrap().seq_read_mbs, (100.0, 140.0));
        let e = expectation(&drive(BusType::Sata, Some(false)), None).unwrap();
        assert_eq!(
            (e.seq_read_mbs, e.reason.as_str()),
            ((80.0, 280.0), "hard drive")
        );
        for bus in [
            BusType::Usb,
            BusType::Sd,
            BusType::Mmc,
            BusType::Virtual,
            BusType::Spaces,
            BusType::Raid,
            BusType::Unknown,
        ] {
            assert_eq!(expectation(&drive(bus, Some(true)), None), None, "{bus:?}");
        }
        assert_eq!(expectation(&drive(BusType::Sata, None), None), None);
    }

    #[test]
    fn verdicts() {
        let result = |mbs: f64, expected: Option<(f64, f64)>| DiskTestResult {
            seq_read_mbs: mbs,
            seq_write_mbs: 0.0,
            rnd4k_read_mbs: 0.0,
            rnd4k_write_mbs: 0.0,
            rnd4k_read_q32_mbs: 0.0,
            file_mb: 1024,
            path: String::new(),
            expected: expected.map(|r| DiskExpectation {
                seq_read_mbs: r,
                reason: String::new(),
            }),
        };
        let r = Some((1000.0, 2000.0));
        assert_eq!(result(1000.0, r).verdict(), Some(Verdict::Good));
        assert_eq!(result(5000.0, r).verdict(), Some(Verdict::Good));
        assert_eq!(result(999.0, r).verdict(), Some(Verdict::Low));
        assert_eq!(result(600.0, r).verdict(), Some(Verdict::Low));
        assert_eq!(result(599.0, r).verdict(), Some(Verdict::VeryLow));
        assert_eq!(result(10.0, None).verdict(), None);
    }

    #[test]
    fn volume_choice() {
        let all = |_: &Volume| true;
        let home = Some(Path::new("/home/sam"));
        let mut d = drive(BusType::Nvme, Some(true));
        d.volumes = vec![vol("/mnt/data", "ext4", 100), vol("/", "ext4", 100)];
        assert_eq!(pick_volume_on(&d, home, all).unwrap().mount, "/");
        d.volumes.push(vol("/home", "btrfs", 100));
        assert_eq!(pick_volume_on(&d, home, all).unwrap().mount, "/home");
        assert_eq!(pick_volume_on(&d, None, all).unwrap().mount, "/mnt/data");
        // A read-only volume is skipped.
        assert_eq!(
            pick_volume_on(&d, home, |v| v.mount != "/home")
                .unwrap()
                .mount,
            "/"
        );
        // Needs file (1 GiB) + 2 GiB free.
        d.volumes = vec![vol("/mnt/data", "xfs", 3)];
        assert!(pick_volume_on(&d, home, all).is_some());
        d.volumes[0].free_bytes = 3 * GIB - 1;
        assert!(pick_volume_on(&d, home, all).is_none());
        // Unsupported file systems and unmounted volumes are skipped.
        d.volumes = vec![
            vol("/boot/efi", "squashfs", 100),
            vol("", "ext4", 100),
            vol("/media/usb", "exfat", 100),
        ];
        assert_eq!(pick_volume_on(&d, home, all).unwrap().mount, "/media/usb");
        d.volumes.pop();
        assert!(pick_volume_on(&d, home, all).is_none());
        assert_eq!(
            mount_of(Path::new("/home/sam"), ["/", "/home", "/ho"].into_iter()),
            Some("/home")
        );
    }

    #[test]
    fn sizes_and_links() {
        let mut d = drive(BusType::Nvme, Some(true));
        assert_eq!(file_bytes(&d), GIB);
        d.size_bytes = 32 * GIB;
        assert_eq!(file_bytes(&d), 512 * MIB);
        assert_eq!(file_bytes(&drive(BusType::Sata, Some(false))), 512 * MIB);
        let links = [
            PcieLink {
                device: "WD_BLACK SN850X 2000GB".into(),
                ..Default::default()
            },
            link((4, 4)),
        ];
        assert_eq!(nvme_link(&d, &links).map(|l| l.current), Some((4, 4)));
        assert!(nvme_link(&drive(BusType::Sata, Some(true)), &links).is_none());
    }
}
