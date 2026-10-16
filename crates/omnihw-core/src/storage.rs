//! Storage data model and parsers for `IOCTL_STORAGE_QUERY_PROPERTY` results
//! (device descriptor, NVMe SMART / health log, temperature descriptor).

use crate::le::{u16_at, u32_at, u128_at};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Drive {
    /// `\\.\PhysicalDriveN` number.
    pub index: u32,
    pub model: String,
    pub vendor: String,
    pub firmware: String,
    pub serial: String,
    pub bus: BusType,
    pub size_bytes: u64,
    /// `None` if Windows doesn't report seek penalty.
    pub ssd: Option<bool>,
    pub removable: bool,
    pub volumes: Vec<Volume>,
    /// NVMe health log, if readable (usually requires administrator rights).
    pub nvme_health: Option<NvmeHealth>,
    /// ATA SMART attributes for SATA drives, if readable (usually requires administrator rights).
    pub ata_smart: Option<crate::smart::AtaSmart>,
    /// Partition table summary, if readable.
    pub layout: Option<DiskLayout>,
    /// SATA link speed from IDENTIFY DEVICE (administrator rights).
    pub sata_link: Option<SataLink>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartitionStyle {
    Mbr,
    Gpt,
    /// Not initialized: no partition table at all.
    Raw,
}

impl PartitionStyle {
    pub fn from_raw(v: u32) -> Option<PartitionStyle> {
        Some(match v {
            0 => PartitionStyle::Mbr,
            1 => PartitionStyle::Gpt,
            2 => PartitionStyle::Raw,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiskLayout {
    pub style: PartitionStyle,
    /// Partitions with a non-zero size.
    pub partitions: u32,
    /// Largest stretch of space not covered by any partition.
    pub unallocated_bytes: u64,
}

/// Largest gap in `[0, disk_size)` not covered by the `(offset, length)` partitions.
pub fn largest_gap(disk_size: u64, partitions: &[(u64, u64)]) -> u64 {
    let mut parts: Vec<(u64, u64)> = partitions.iter().copied().filter(|p| p.1 > 0).collect();
    parts.sort_unstable();
    let (mut pos, mut best) = (0u64, 0u64);
    for (off, len) in parts {
        best = best.max(off.saturating_sub(pos));
        pos = pos.max(off.saturating_add(len));
    }
    best.max(disk_size.saturating_sub(pos))
}

/// SATA link generation: 1 = 1.5 Gb/s, 2 = 3 Gb/s, 3 = 6 Gb/s.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SataLink {
    pub current_gen: u8,
    pub max_gen: u8,
}

pub fn sata_gen_speed(generation: u8) -> &'static str {
    match generation {
        1 => "1.5 Gb/s",
        2 => "3 Gb/s",
        3 => "6 Gb/s",
        _ => "unknown speed",
    }
}

/// SATA link speeds from an ATA IDENTIFY DEVICE block: word 76 lists supported generations,
/// word 77 bits 1-3 the negotiated one.
pub fn parse_sata_link(identify: &[u8]) -> Option<SataLink> {
    let caps = u16_at(identify, 76 * 2)?;
    let cur = u16_at(identify, 77 * 2)?;
    if caps == 0 || caps == 0xFFFF {
        return None;
    }
    let max_gen = (1..=3u8).rev().find(|g| caps >> g & 1 == 1)?;
    let current_gen = ((cur >> 1) & 0b111) as u8;
    (1..=3).contains(&current_gen).then_some(SataLink {
        current_gen,
        max_gen,
    })
}

impl Drive {
    pub fn kind(&self) -> &'static str {
        match (self.bus, self.ssd) {
            (BusType::Nvme, _) => "NVMe SSD",
            (_, Some(true)) => "SSD",
            (_, Some(false)) => "HDD",
            _ => "Disk",
        }
    }

    /// Prefix for this drive's sensor ids.
    pub fn sensor_prefix(&self) -> String {
        format!("disk/{}", self.index)
    }

    /// Display name, e.g. "Samsung SSD 980 PRO 1TB".
    pub fn name(&self) -> String {
        let model = self.model.trim();
        let vendor = self.vendor.trim();
        if vendor.is_empty()
            || model
                .to_ascii_lowercase()
                .starts_with(&vendor.to_ascii_lowercase())
        {
            model.to_string()
        } else {
            format!("{vendor} {model}")
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Volume {
    /// e.g. "C:"
    pub mount: String,
    pub label: String,
    pub file_system: String,
    pub total_bytes: u64,
    pub free_bytes: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum BusType {
    #[default]
    Unknown,
    Scsi,
    Atapi,
    Ata,
    Usb,
    Raid,
    Sas,
    Sata,
    Sd,
    Mmc,
    Virtual,
    Spaces,
    Nvme,
    Ufs,
    Other(u32),
}

impl BusType {
    pub fn from_raw(v: u32) -> BusType {
        match v {
            0 => BusType::Unknown,
            1 => BusType::Scsi,
            2 => BusType::Atapi,
            3 => BusType::Ata,
            7 => BusType::Usb,
            8 => BusType::Raid,
            0xA => BusType::Sas,
            0xB => BusType::Sata,
            0xC => BusType::Sd,
            0xD => BusType::Mmc,
            0xE | 0xF => BusType::Virtual,
            0x10 => BusType::Spaces,
            0x11 => BusType::Nvme,
            0x13 => BusType::Ufs,
            o => BusType::Other(o),
        }
    }

    /// Whether ATA SMART can usually be read on this bus: SATA/ATA drives, also behind
    /// RAID/SCSI miniports. USB bridges, virtual disks and card readers rarely pass it through,
    /// and NVMe has its own health log.
    pub fn smart_capable(self) -> bool {
        !matches!(
            self,
            BusType::Nvme
                | BusType::Usb
                | BusType::Virtual
                | BusType::Sd
                | BusType::Mmc
                | BusType::Spaces
        )
    }

    pub fn name(self) -> String {
        match self {
            BusType::Unknown => "Unknown".into(),
            BusType::Scsi => "SCSI".into(),
            BusType::Atapi => "ATAPI".into(),
            BusType::Ata => "ATA".into(),
            BusType::Usb => "USB".into(),
            BusType::Raid => "RAID".into(),
            BusType::Sas => "SAS".into(),
            BusType::Sata => "SATA".into(),
            BusType::Sd => "SD".into(),
            BusType::Mmc => "MMC".into(),
            BusType::Virtual => "Virtual".into(),
            BusType::Spaces => "Storage Spaces".into(),
            BusType::Nvme => "NVMe".into(),
            BusType::Ufs => "UFS".into(),
            BusType::Other(v) => format!("Bus {v}"),
        }
    }
}

/// NUL-terminated ASCII string at `offset` (0 = absent).
fn str_at(b: &[u8], offset: u32) -> String {
    let o = offset as usize;
    if o == 0 || o >= b.len() {
        return String::new();
    }
    let end = b[o..]
        .iter()
        .position(|&c| c == 0)
        .map_or(b.len(), |e| o + e);
    String::from_utf8_lossy(&b[o..end]).trim().to_string()
}

/// Fields from `STORAGE_DEVICE_DESCRIPTOR`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeviceDescriptor {
    pub vendor: String,
    pub product: String,
    pub revision: String,
    pub serial: String,
    pub bus: BusType,
    pub removable: bool,
}

pub fn parse_device_descriptor(b: &[u8]) -> Option<DeviceDescriptor> {
    let size = (u32_at(b, 4)? as usize).min(b.len());
    let b = &b[..size];
    Some(DeviceDescriptor {
        removable: *b.get(10)? != 0,
        vendor: str_at(b, u32_at(b, 12)?),
        product: str_at(b, u32_at(b, 16)?),
        revision: str_at(b, u32_at(b, 20)?),
        serial: str_at(b, u32_at(b, 24)?),
        bus: BusType::from_raw(u32_at(b, 28)?),
    })
}

/// Temperatures (°C) from `STORAGE_TEMPERATURE_DATA_DESCRIPTOR`. The first entry is the composite/drive temperature.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TemperatureData {
    pub critical: i16,
    pub warning: i16,
    pub sensors: Vec<i16>,
}

pub fn parse_temperature_descriptor(b: &[u8]) -> Option<TemperatureData> {
    let critical = u16_at(b, 8)? as i16;
    let warning = u16_at(b, 10)? as i16;
    let count = u16_at(b, 12)? as usize;
    let sensors = (0..count.min(16))
        .filter_map(|i| u16_at(b, 24 + i * 16 + 2).map(|t| t as i16))
        .collect();
    Some(TemperatureData {
        critical,
        warning,
        sensors,
    })
}

/// NVMe SMART / Health Information log page (log id 02h).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NvmeHealth {
    pub critical_warning: u8,
    pub temperature_c: i32,
    pub available_spare_pct: u8,
    pub spare_threshold_pct: u8,
    /// Vendor estimate of life used; may exceed 100.
    pub percentage_used: u8,
    pub data_read_bytes: u128,
    pub data_written_bytes: u128,
    pub power_cycles: u128,
    pub power_on_hours: u128,
    pub unsafe_shutdowns: u128,
    pub media_errors: u128,
    pub error_log_entries: u128,
    pub sensor_temps_c: Vec<i32>,
}

impl NvmeHealth {
    /// Health estimate 0..=100 (100 - percentage used, floored at 0).
    pub fn health_pct(&self) -> u8 {
        100u8.saturating_sub(self.percentage_used)
    }

    /// Human-readable critical warning flags.
    pub fn warnings(&self) -> Vec<&'static str> {
        const FLAGS: [&str; 6] = [
            "spare below threshold",
            "temperature out of range",
            "reliability degraded",
            "read-only",
            "volatile backup failed",
            "persistent memory read-only",
        ];
        FLAGS
            .iter()
            .enumerate()
            .filter(|(i, _)| self.critical_warning >> i & 1 == 1)
            .map(|(_, f)| *f)
            .collect()
    }
}

fn kelvin(k: u16) -> i32 {
    k as i32 - 273
}

pub fn parse_nvme_health(b: &[u8]) -> Option<NvmeHealth> {
    if b.len() < 512 {
        return None;
    }
    // Data units are thousands of 512-byte units.
    let units = |o| u128_at(b, o).map(|v| v * 512_000);
    Some(NvmeHealth {
        critical_warning: b[0],
        temperature_c: kelvin(u16_at(b, 1)?),
        available_spare_pct: b[3],
        spare_threshold_pct: b[4],
        percentage_used: b[5],
        data_read_bytes: units(32)?,
        data_written_bytes: units(48)?,
        power_cycles: u128_at(b, 112)?,
        power_on_hours: u128_at(b, 128)?,
        unsafe_shutdowns: u128_at(b, 144)?,
        media_errors: u128_at(b, 160)?,
        error_log_entries: u128_at(b, 176)?,
        sensor_temps_c: (0..8)
            .filter_map(|i| u16_at(b, 200 + i * 2))
            .filter(|&k| k != 0)
            .map(kelvin)
            .collect(),
    })
}

/// Parse a `PhysicalDisk` PDH instance like "0 C: D:" into the disk number.
pub fn parse_disk_instance(name: &str) -> Option<u32> {
    name.split_whitespace().next()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor() -> Vec<u8> {
        let mut b = vec![0u8; 128];
        b[4..8].copy_from_slice(&128u32.to_le_bytes());
        b[10] = 0;
        let mut put = |field: usize, off: u32, s: &str| {
            b[field..field + 4].copy_from_slice(&off.to_le_bytes());
            b[off as usize..off as usize + s.len()].copy_from_slice(s.as_bytes());
        };
        put(12, 40, "");
        put(16, 48, "Samsung SSD 980 PRO 1TB");
        put(20, 80, "5B2QGXA7");
        put(24, 96, "S5GXNF0R123456      ");
        b[28..32].copy_from_slice(&0x11u32.to_le_bytes());
        b
    }

    #[test]
    fn device_descriptor() {
        let d = parse_device_descriptor(&descriptor()).unwrap();
        assert_eq!(d.product, "Samsung SSD 980 PRO 1TB");
        assert_eq!(d.revision, "5B2QGXA7");
        assert_eq!(d.serial, "S5GXNF0R123456");
        assert_eq!(d.vendor, "");
        assert_eq!(d.bus, BusType::Nvme);
        assert!(!d.removable);
        assert!(parse_device_descriptor(&[0; 8]).is_none());
    }

    #[test]
    fn drive_names() {
        let mut d = Drive {
            model: "Samsung SSD 980".into(),
            vendor: "Samsung".into(),
            ..Default::default()
        };
        assert_eq!(d.name(), "Samsung SSD 980");
        d.vendor = "WDC".into();
        d.model = "WD40EFRX".into();
        assert_eq!(d.name(), "WDC WD40EFRX");
        d.bus = BusType::Nvme;
        assert_eq!(d.kind(), "NVMe SSD");
    }

    #[test]
    fn nvme_health() {
        let mut b = vec![0u8; 512];
        b[0] = 0b0000_0101;
        b[1..3].copy_from_slice(&(273u16 + 41).to_le_bytes());
        b[3] = 100;
        b[4] = 10;
        b[5] = 3;
        b[32..48].copy_from_slice(&1000u128.to_le_bytes());
        b[48..64].copy_from_slice(&2000u128.to_le_bytes());
        b[112..128].copy_from_slice(&850u128.to_le_bytes());
        b[128..144].copy_from_slice(&12345u128.to_le_bytes());
        b[144..160].copy_from_slice(&42u128.to_le_bytes());
        b[200..202].copy_from_slice(&(273u16 + 50).to_le_bytes());
        let h = parse_nvme_health(&b).unwrap();
        assert_eq!(h.temperature_c, 41);
        assert_eq!(h.health_pct(), 97);
        assert_eq!(h.data_read_bytes, 512_000_000);
        assert_eq!(h.data_written_bytes, 1_024_000_000);
        assert_eq!(
            (h.power_cycles, h.power_on_hours, h.unsafe_shutdowns),
            (850, 12345, 42)
        );
        assert_eq!(h.sensor_temps_c, vec![50]);
        assert_eq!(
            h.warnings(),
            vec!["spare below threshold", "reliability degraded"]
        );
        assert!(parse_nvme_health(&b[..100]).is_none());
    }

    #[test]
    fn temperature_descriptor() {
        let mut b = vec![0u8; 24 + 32];
        b[8..10].copy_from_slice(&84i16.to_le_bytes());
        b[10..12].copy_from_slice(&75i16.to_le_bytes());
        b[12..14].copy_from_slice(&2u16.to_le_bytes());
        b[26..28].copy_from_slice(&38i16.to_le_bytes());
        b[42..44].copy_from_slice(&45i16.to_le_bytes());
        let t = parse_temperature_descriptor(&b).unwrap();
        assert_eq!((t.critical, t.warning), (84, 75));
        assert_eq!(t.sensors, vec![38, 45]);
    }

    #[test]
    fn gaps_and_links() {
        const G: u64 = 1 << 30;
        assert_eq!(largest_gap(100 * G, &[]), 100 * G);
        assert_eq!(largest_gap(100 * G, &[(G, 50 * G)]), 49 * G);
        assert_eq!(
            largest_gap(100 * G, &[(0, 10 * G), (5 * G, 90 * G), (0, 0)]),
            5 * G
        );
        let mut id = vec![0u8; 512];
        id[152..154].copy_from_slice(&0b1110u16.to_le_bytes());
        id[154..156].copy_from_slice(&(2u16 << 1).to_le_bytes());
        assert_eq!(
            parse_sata_link(&id),
            Some(SataLink {
                current_gen: 2,
                max_gen: 3
            })
        );
        id[152..154].copy_from_slice(&0u16.to_le_bytes());
        assert_eq!(parse_sata_link(&id), None);
    }

    #[test]
    fn disk_instances() {
        assert_eq!(parse_disk_instance("0 C: D:"), Some(0));
        assert_eq!(parse_disk_instance("12"), Some(12));
        assert_eq!(parse_disk_instance("_Total"), None);
    }
}
