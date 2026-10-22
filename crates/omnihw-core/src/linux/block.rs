//! Physical drives from `/sys/block`, with udev's database (`/run/udev/data`) for serials,
//! labels and partition tables, and `/proc/self/mounts` for volumes.

use super::Root;
use crate::checkup::PcieLink;
use crate::storage::{BusType, DiskLayout, Drive, PartitionStyle, Volume, largest_gap};

const BLOCK: &str = "/sys/block";

/// Virtual and optical devices that aren't drives.
const SKIP: [&str; 9] = ["loop", "ram", "zram", "dm-", "md", "sr", "fd", "nbd", "zd"];

/// `E:KEY=value` properties from a udev database entry.
pub fn udev_props(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|l| {
            let (k, v) = l.strip_prefix("E:")?.split_once('=')?;
            Some((k.to_string(), v.to_string()))
        })
        .collect()
}

/// udev properties of the block device whose sysfs dir is `sys` (via its `dev` major:minor).
fn udev(root: &Root, sys: &str) -> Vec<(String, String)> {
    root.read(&format!("{sys}/dev"))
        .and_then(|dev| root.read(&format!("/run/udev/data/b{dev}")))
        .map(|t| udev_props(&t))
        .unwrap_or_default()
}

fn prop<'a>(props: &'a [(String, String)], key: &str) -> Option<&'a str> {
    props
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
        .filter(|v| !v.is_empty())
}

/// Undo the `\x20`-style escapes in udev's `*_ENC` values.
pub fn unescape_hex(s: &str) -> String {
    let mut out = Vec::with_capacity(s.len());
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\'
            && b.get(i + 1) == Some(&b'x')
            && let Some(v) = s
                .get(i + 2..i + 4)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 4;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).trim().to_string()
}

/// Undo the octal escapes (`\040` = space) in `/proc/self/mounts` fields.
pub fn unescape_octal(s: &str) -> String {
    let mut out = Vec::with_capacity(s.len());
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\'
            && let Some(v) = s
                .get(i + 1..i + 4)
                .and_then(|o| u8::from_str_radix(o, 8).ok())
        {
            out.push(v);
            i += 4;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// (device, mount point, file system) from `/proc/self/mounts`.
pub fn parse_mounts(text: &str) -> Vec<(String, String, String)> {
    text.lines()
        .filter_map(|l| {
            let mut f = l.split_whitespace();
            let dev = f.next()?;
            let mount = f.next()?;
            let fs = f.next()?;
            dev.starts_with("/dev/")
                .then(|| (unescape_octal(dev), unescape_octal(mount), fs.to_string()))
        })
        .collect()
}

pub fn bus_type(name: &str, props: &[(String, String)]) -> BusType {
    if name.starts_with("nvme") {
        return BusType::Nvme;
    }
    if name.starts_with("mmcblk") {
        return BusType::Mmc;
    }
    if name.starts_with("vd") || name.starts_with("xvd") {
        return BusType::Virtual;
    }
    match prop(props, "ID_BUS") {
        Some("ata") if prop(props, "ID_ATA_SATA") == Some("1") => BusType::Sata,
        Some("ata") => BusType::Ata,
        Some("usb") => BusType::Usb,
        Some("scsi") => BusType::Scsi,
        _ => BusType::Unknown,
    }
}

/// Free and total bytes of the file system mounted at `path`.
fn space(path: &str) -> Option<(u64, u64)> {
    let s = rustix::fs::statvfs(path).ok()?;
    Some((
        s.f_bavail.saturating_mul(s.f_frsize),
        s.f_blocks.saturating_mul(s.f_frsize),
    ))
}

/// Device nodes that can carry `part`'s file system: the partition itself plus any
/// device-mapper devices stacked on it (LUKS, LVM), as `/dev/dm-N` and `/dev/mapper/<name>`.
fn device_nodes(root: &Root, sys: &str, part: &str) -> Vec<String> {
    let mut out = vec![format!("/dev/{part}")];
    for holder in root.list(&format!("{sys}/holders")) {
        out.push(format!("/dev/{holder}"));
        if let Some(n) = root.read_nonempty(&format!("{BLOCK}/{holder}/dm/name")) {
            out.push(format!("/dev/mapper/{n}"));
        }
    }
    out
}

fn volumes(
    root: &Root,
    disk: &str,
    parts: &[String],
    mounts: &[(String, String, String)],
) -> Vec<Volume> {
    let mut out = Vec::new();
    let whole = std::iter::once((disk.to_string(), format!("{BLOCK}/{disk}")));
    let partitions = parts
        .iter()
        .map(|p| (p.clone(), format!("{BLOCK}/{disk}/{p}")));
    for (part, sys) in whole.chain(partitions) {
        let nodes = device_nodes(root, &sys, &part);
        // The first mount of a device is its main one (later ones are bind/subvolume mounts).
        let Some((_, mount, fs)) = mounts.iter().find(|(d, _, _)| nodes.contains(d)) else {
            continue;
        };
        let props = udev(root, &sys);
        let label = prop(&props, "ID_FS_LABEL_ENC")
            .map(unescape_hex)
            .or_else(|| prop(&props, "ID_FS_LABEL").map(str::to_string))
            .unwrap_or_default();
        let (free_bytes, total_bytes) = space(mount).unwrap_or_else(|| {
            let size = root.read_u64(&format!("{sys}/size")).unwrap_or(0) * 512;
            (0, size)
        });
        out.push(Volume {
            mount: mount.clone(),
            label,
            file_system: fs.clone(),
            total_bytes,
            free_bytes,
        });
    }
    out
}

fn layout(
    root: &Root,
    disk: &str,
    parts: &[String],
    size_bytes: u64,
    props: &[(String, String)],
) -> Option<DiskLayout> {
    let style = match prop(props, "ID_PART_TABLE_TYPE") {
        Some("gpt") => PartitionStyle::Gpt,
        Some("dos") => PartitionStyle::Mbr,
        Some(_) => return None,
        // A file system, LVM or LUKS straight on the disk (no partition table) is in use.
        None if prop(props, "ID_FS_TYPE").is_some() => return None,
        None if parts.is_empty() && !props.is_empty() => PartitionStyle::Raw,
        None => return None,
    };
    let extents: Vec<(u64, u64)> = parts
        .iter()
        .filter_map(|p| {
            let base = format!("{BLOCK}/{disk}/{p}");
            Some((
                root.read_u64(&format!("{base}/start"))? * 512,
                root.read_u64(&format!("{base}/size"))? * 512,
            ))
        })
        .collect();
    Some(DiskLayout {
        style,
        partitions: extents.iter().filter(|e| e.1 > 0).count() as u32,
        unallocated_bytes: largest_gap(size_bytes, &extents),
    })
}

pub fn drives(root: &Root) -> Vec<Drive> {
    let mounts = root
        .read("/proc/self/mounts")
        .map(|m| parse_mounts(&m))
        .unwrap_or_default();
    let mut out = Vec::new();
    for name in root.list(BLOCK) {
        if SKIP.iter().any(|s| name.starts_with(s)) {
            continue;
        }
        let sys = format!("{BLOCK}/{name}");
        let size_bytes = root.read_u64(&format!("{sys}/size")).unwrap_or(0) * 512;
        if !root.exists(&format!("{sys}/device")) || size_bytes == 0 {
            continue;
        }
        let props = udev(root, &sys);
        let dev = |f: &str| root.read_nonempty(&format!("{sys}/device/{f}"));
        let model = dev("model")
            .or_else(|| prop(&props, "ID_MODEL_ENC").map(unescape_hex))
            .or_else(|| prop(&props, "ID_MODEL").map(|m| m.replace('_', " ")))
            .unwrap_or_default();
        // SCSI-translated SATA drives report the vendor "ATA", which says nothing.
        let vendor = dev("vendor")
            .filter(|v| v != "ATA")
            .or_else(|| prop(&props, "ID_VENDOR").map(|v| v.replace('_', " ")))
            .filter(|v| v != "ATA")
            .unwrap_or_default();
        let parts: Vec<String> = root
            .list(&sys)
            .into_iter()
            .filter(|p| p.starts_with(&name) && root.exists(&format!("{sys}/{p}/partition")))
            .collect();
        let bus = bus_type(&name, &props);
        out.push(Drive {
            index: out.len() as u32,
            model: model.trim().to_string(),
            vendor: vendor.trim().to_string(),
            firmware: dev("firmware_rev")
                .or_else(|| dev("rev"))
                .or_else(|| prop(&props, "ID_REVISION").map(str::to_string))
                .unwrap_or_default(),
            serial: dev("serial")
                .or_else(|| prop(&props, "ID_SERIAL_SHORT").map(str::to_string))
                .unwrap_or_default(),
            bus,
            size_bytes,
            ssd: match root.read(&format!("{sys}/queue/rotational")).as_deref() {
                Some("0") => Some(true),
                Some("1") => Some(false),
                _ => None,
            },
            removable: root.read(&format!("{sys}/removable")).as_deref() == Some("1")
                || bus == BusType::Usb,
            // Needs root; `None` otherwise, like on Windows without administrator rights.
            nvme_health: if bus == BusType::Nvme {
                super::nvme::controller_of(&name).and_then(super::nvme::smart_log)
            } else {
                None
            },
            volumes: volumes(root, &name, &parts, &mounts),
            layout: layout(root, &name, &parts, size_bytes, &props),
            ..Default::default()
        });
    }
    out
}

/// PCIe generation for a sysfs link speed such as "16.0 GT/s PCIe".
pub fn pcie_gen(speed: &str) -> Option<u32> {
    let gts: f64 = speed.split_whitespace().next()?.parse().ok()?;
    Some(match gts {
        g if g < 4.0 => 1,
        g if g < 7.0 => 2,
        g if g < 12.0 => 3,
        g if g < 24.0 => 4,
        g if g < 48.0 => 5,
        _ => 6,
    })
}

/// PCIe links of NVMe controllers (`/sys/class/nvme/nvmeN/device`), as (generation, lanes).
pub fn nvme_links(root: &Root) -> Vec<PcieLink> {
    const NVME: &str = "/sys/class/nvme";
    root.list(NVME)
        .into_iter()
        .filter_map(|c| {
            let p = format!("{NVME}/{c}");
            let link = |kind: &str| {
                let speed = root.read(&format!("{p}/device/{kind}_link_speed"))?;
                let width = root.read_u64(&format!("{p}/device/{kind}_link_width"))?;
                Some((pcie_gen(&speed)?, width as u32))
            };
            Some(PcieLink {
                device: root.read_nonempty(&format!("{p}/model"))?,
                current: link("current")?,
                max: link("max")?,
            })
        })
        .collect()
}

/// sysfs block device names of the drives, in [`Drive::index`] order (for live sensors).
pub fn drive_names(root: &Root) -> Vec<String> {
    root.list(BLOCK)
        .into_iter()
        .filter(|name| {
            let sys = format!("{BLOCK}/{name}");
            !SKIP.iter().any(|s| name.starts_with(s))
                && root.exists(&format!("{sys}/device"))
                && root.read_u64(&format!("{sys}/size")).unwrap_or(0) > 0
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::linux::Fixture;

    #[test]
    fn pcie_links() {
        assert_eq!(pcie_gen("16.0 GT/s PCIe"), Some(4));
        assert_eq!(pcie_gen("2.5 GT/s PCIe"), Some(1));
        assert_eq!(pcie_gen("Unknown"), None);
        let f = Fixture::new();
        let d = "sys/class/nvme/nvme0";
        f.file(&format!("{d}/model"), "WD_BLACK SN850X 2000GB   \n")
            .file(&format!("{d}/device/current_link_speed"), "8.0 GT/s PCIe\n")
            .file(&format!("{d}/device/current_link_width"), "4\n")
            .file(&format!("{d}/device/max_link_speed"), "16.0 GT/s PCIe\n")
            .file(&format!("{d}/device/max_link_width"), "4\n");
        let l = nvme_links(&f.root());
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].device, "WD_BLACK SN850X 2000GB");
        assert_eq!((l[0].current, l[0].max), ((3, 4), (4, 4)));
    }

    #[test]
    fn escapes() {
        assert_eq!(unescape_hex("Samsung\\x20SSD\\x20980"), "Samsung SSD 980");
        assert_eq!(unescape_octal("/media/My\\040Disk"), "/media/My Disk");
    }

    #[test]
    fn mounts() {
        let m = parse_mounts(
            "proc /proc proc rw 0 0\n/dev/nvme0n1p2 / ext4 rw 0 0\n/dev/sda1 /mnt/My\\040Data ntfs3 rw 0 0\n",
        );
        assert_eq!(m.len(), 2);
        assert_eq!(m[1].1, "/mnt/My Data");
    }

    fn fixture() -> Fixture {
        let f = Fixture::new();
        let nvme = "sys/block/nvme0n1";
        f.file(&format!("{nvme}/size"), "1953525168")
            .file(&format!("{nvme}/dev"), "259:0")
            .file(&format!("{nvme}/queue/rotational"), "0")
            .file(&format!("{nvme}/removable"), "0")
            .file(&format!("{nvme}/device/model"), "Samsung SSD 980 PRO 1TB          ")
            .file(&format!("{nvme}/device/firmware_rev"), "5B2QGXA7")
            .file(&format!("{nvme}/device/serial"), "S5GXNX0T123456")
            .file(&format!("{nvme}/nvme0n1p1/partition"), "1")
            .file(&format!("{nvme}/nvme0n1p1/start"), "2048")
            .file(&format!("{nvme}/nvme0n1p1/size"), "1048576")
            .file(&format!("{nvme}/nvme0n1p1/dev"), "259:1")
            .file(&format!("{nvme}/nvme0n1p2/partition"), "2")
            .file(&format!("{nvme}/nvme0n1p2/start"), "1050624")
            .file(&format!("{nvme}/nvme0n1p2/size"), "1000000000")
            .file(&format!("{nvme}/nvme0n1p2/dev"), "259:2")
            .dir(&format!("{nvme}/nvme0n1p2/holders/dm-0"))
            .file("sys/block/dm-0/dm/name", "cryptroot")
            .file("sys/block/dm-0/size", "999000000")
            .file("run/udev/data/b259:0", "E:ID_PART_TABLE_TYPE=gpt\n")
            .file("run/udev/data/b259:2", "E:ID_FS_LABEL_ENC=my\\x20root\n")
            .file(
                "proc/self/mounts",
                "/dev/nvme0n1p1 /boot/efi vfat rw 0 0\n/dev/mapper/cryptroot / btrfs rw 0 0\n/dev/mapper/cryptroot /home btrfs rw 0 0\n",
            )
            .file("sys/block/sda/size", "976773168")
            .file("sys/block/sda/dev", "8:0")
            .file("sys/block/sda/queue/rotational", "1")
            .file("sys/block/sda/device/vendor", "ATA     ")
            .file("sys/block/sda/device/model", "ST500DM002-1BD14")
            .file(
                "run/udev/data/b8:0",
                "S:disk/by-id/x\nE:ID_BUS=ata\nE:ID_ATA_SATA=1\nE:ID_SERIAL_SHORT=Z3T0ABCD\nE:ID_REVISION=KC48\n",
            )
            .file("sys/block/loop0/size", "100")
            .dir("sys/block/loop0/device");
        f
    }

    #[test]
    fn drive_list() {
        let f = fixture();
        let d = drives(&f.root());
        assert_eq!(d.len(), 2);
        let n = &d[0];
        assert_eq!(n.model, "Samsung SSD 980 PRO 1TB");
        assert_eq!(
            (n.bus, n.ssd, n.removable),
            (BusType::Nvme, Some(true), false)
        );
        assert_eq!(n.size_bytes, 1953525168 * 512);
        assert_eq!(n.serial, "S5GXNX0T123456");
        assert_eq!(n.volumes.len(), 2);
        assert_eq!(n.volumes[0].mount, "/boot/efi");
        assert_eq!(n.volumes[1].mount, "/");
        assert_eq!(n.volumes[1].label, "my root");
        assert_eq!(n.volumes[1].file_system, "btrfs");
        let l = n.layout.unwrap();
        assert_eq!((l.style, l.partitions), (PartitionStyle::Gpt, 2));
        let s = &d[1];
        assert_eq!(s.index, 1);
        assert_eq!((s.bus, s.ssd), (BusType::Sata, Some(false)));
        assert_eq!(s.vendor, "");
        assert_eq!(
            (s.serial.as_str(), s.firmware.as_str()),
            ("Z3T0ABCD", "KC48")
        );
        assert_eq!(drive_names(&f.root()), vec!["nvme0n1", "sda"]);
    }
}
