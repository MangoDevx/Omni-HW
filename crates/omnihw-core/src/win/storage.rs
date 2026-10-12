//! Physical drives via `\\.\PhysicalDriveN` storage IOCTLs, volumes, and live disk sensors.

use super::ffi::*;
use super::pdh::{Counter, Query, read_array};
use crate::sensors::{Reading, SensorBackend, Unit};
use crate::smart::{AtaSmart, parse_ata_smart};
use crate::storage::*;
use std::ffi::c_void;
use std::ptr;

const PROP_DEVICE: u32 = 0;
const PROP_SEEK_PENALTY: u32 = 7;
const PROP_TEMPERATURE: u32 = 11;
const PROP_DEVICE_PROTOCOL_SPECIFIC: u32 = 50;
const PROTOCOL_TYPE_NVME: u32 = 3;
const NVME_DATA_TYPE_LOG_PAGE: u32 = 2;
const NVME_LOG_HEALTH: u32 = 2;
const PROTOCOL_SPECIFIC_DATA_SIZE: usize = 40;

/// Owned Win32 handle.
struct Handle(HANDLE);

// SAFETY: device handles may be used from any thread.
unsafe impl Send for Handle {}

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: valid handle owned by us.
        unsafe { CloseHandle(self.0) };
    }
}

fn open(path: &str, access: u32) -> Option<Handle> {
    let w = wide(path);
    // SAFETY: NUL-terminated path; no template.
    let h = unsafe {
        CreateFileW(
            w.as_ptr(),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            ptr::null_mut(),
            OPEN_EXISTING,
            0,
            ptr::null_mut(),
        )
    };
    (h != INVALID_HANDLE_VALUE && !h.is_null()).then_some(Handle(h))
}

fn ioctl(h: &Handle, code: u32, input: &mut [u8], out: &mut [u8]) -> Option<usize> {
    let mut got = 0;
    // SAFETY: buffers are valid for the given lengths.
    let ok = unsafe {
        DeviceIoControl(
            h.0,
            code,
            input.as_mut_ptr() as *mut c_void,
            input.len() as u32,
            out.as_mut_ptr() as *mut c_void,
            out.len() as u32,
            &mut got,
            ptr::null_mut(),
        )
    };
    (ok != 0).then_some(got as usize)
}

/// `IOCTL_STORAGE_QUERY_PROPERTY` with a standard query for `property`.
fn query_property(h: &Handle, property: u32, out_len: usize) -> Option<Vec<u8>> {
    let mut q = [0u8; 12];
    q[0..4].copy_from_slice(&property.to_le_bytes());
    let mut out = vec![0u8; out_len];
    let n = ioctl(h, IOCTL_STORAGE_QUERY_PROPERTY, &mut q, &mut out)?;
    out.truncate(n);
    Some(out)
}

fn temperature(h: &Handle) -> Option<TemperatureData> {
    parse_temperature_descriptor(&query_property(h, PROP_TEMPERATURE, 512)?)
}

fn nvme_health(h: &Handle) -> Option<NvmeHealth> {
    let mut q = vec![0u8; 8 + PROTOCOL_SPECIFIC_DATA_SIZE + 512];
    let put = |q: &mut [u8], o: usize, v: u32| q[o..o + 4].copy_from_slice(&v.to_le_bytes());
    put(&mut q, 0, PROP_DEVICE_PROTOCOL_SPECIFIC);
    // STORAGE_PROTOCOL_SPECIFIC_DATA starts at offset 8.
    put(&mut q, 8, PROTOCOL_TYPE_NVME);
    put(&mut q, 12, NVME_DATA_TYPE_LOG_PAGE);
    put(&mut q, 16, NVME_LOG_HEALTH);
    put(&mut q, 24, PROTOCOL_SPECIFIC_DATA_SIZE as u32);
    put(&mut q, 28, 512);
    let mut out = vec![0u8; q.len()];
    let n = ioctl(h, IOCTL_STORAGE_QUERY_PROPERTY, &mut q, &mut out)?;
    // STORAGE_PROTOCOL_DATA_DESCRIPTOR: Version, Size, then the protocol data header.
    let data_offset = u32::from_le_bytes(out.get(24..28)?.try_into().ok()?) as usize;
    let data_len = u32::from_le_bytes(out.get(28..32)?.try_into().ok()?) as usize;
    let start = 8 + data_offset;
    parse_nvme_health(out.get(start..(start + data_len).min(n))?)
}

/// `SMART_RCV_DRIVE_DATA`: CTL_CODE(IOCTL_DISK_BASE, 0x22, METHOD_BUFFERED, read|write access).
const SMART_RCV_DRIVE_DATA: u32 = 0x0007_C088;
/// SENDCMDOUTPARAMS header (cBufferSize + DRIVERSTATUS) before the 512-byte sector.
const SENDCMDOUT_HEADER: usize = 16;

/// ATA SMART READ DATA through the legacy SMART IOCTL. Needs a read/write handle, which
/// Windows only grants to administrators for physical drives.
fn ata_smart(h: &Handle, drive: u32) -> Option<AtaSmart> {
    // SENDCMDINPARAMS (packed): cBufferSize u32, IDEREGS[8], bDriveNumber, reserved[3],
    // dwReserved[4]; the trailing 1-byte buffer is not sent.
    let mut input = [0u8; 32];
    input[0..4].copy_from_slice(&512u32.to_le_bytes());
    input[4] = 0xD0; // features: SMART READ DATA
    input[5] = 1; // sector count
    input[6] = 1; // sector number
    input[7] = 0x4F; // cylinder low (SMART signature)
    input[8] = 0xC2; // cylinder high (SMART signature)
    input[9] = 0xA0 | (((drive & 1) as u8) << 4); // drive/head
    input[10] = 0xB0; // command: SMART
    input[12] = drive as u8;
    let mut out = [0u8; SENDCMDOUT_HEADER + 512];
    let n = ioctl(h, SMART_RCV_DRIVE_DATA, &mut input, &mut out)?;
    parse_ata_smart(out.get(SENDCMDOUT_HEADER..n)?)
}

fn disk_size(h: &Handle) -> Option<u64> {
    let mut out = [0u8; 256];
    ioctl(h, IOCTL_DISK_GET_DRIVE_GEOMETRY_EX, &mut [], &mut out)?;
    Some(u64::from_le_bytes(out[24..32].try_into().ok()?))
}

fn utf16_z(b: &[u16]) -> String {
    String::from_utf16_lossy(&b[..b.iter().position(|&c| c == 0).unwrap_or(b.len())])
}

/// Mounted drive-letter volumes with the physical disk number they live on.
fn volumes() -> Vec<(u32, Volume)> {
    let mut out = Vec::new();
    // SAFETY: trivially safe.
    let mask = unsafe { GetLogicalDrives() };
    for i in 0..26u32 {
        if mask >> i & 1 == 0 {
            continue;
        }
        let letter = (b'A' + i as u8) as char;
        let Some(h) = open(&format!(r"\\.\{letter}:"), 0) else {
            continue;
        };
        let mut num = [0u8; 12];
        if ioctl(&h, IOCTL_STORAGE_GET_DEVICE_NUMBER, &mut [], &mut num).is_none() {
            continue;
        }
        let disk = u32::from_le_bytes(num[4..8].try_into().unwrap());
        let root = wide(&format!(r"{letter}:\"));
        let (mut avail, mut total, mut free) = (0u64, 0u64, 0u64);
        // SAFETY: valid root path and out pointers.
        unsafe { GetDiskFreeSpaceExW(root.as_ptr(), &mut avail, &mut total, &mut free) };
        let mut label = [0u16; 128];
        let mut fs = [0u16; 32];
        // SAFETY: buffer lengths passed explicitly.
        unsafe {
            GetVolumeInformationW(
                root.as_ptr(),
                label.as_mut_ptr(),
                label.len() as u32,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                fs.as_mut_ptr(),
                fs.len() as u32,
            )
        };
        out.push((
            disk,
            Volume {
                mount: format!("{letter}:"),
                label: utf16_z(&label),
                file_system: utf16_z(&fs),
                total_bytes: total,
                free_bytes: free,
            },
        ));
    }
    out
}

/// All physical drives with their volumes. NVMe health is read when permitted.
pub fn drives() -> Vec<Drive> {
    let vols = volumes();
    let mut out = Vec::new();
    let mut misses = 0;
    for index in 0..64u32 {
        let path = format!(r"\\.\PhysicalDrive{index}");
        let Some(h) = open(&path, 0) else {
            // Drive numbers can have gaps; stop after a run of misses.
            misses += 1;
            if misses > 8 {
                break;
            }
            continue;
        };
        misses = 0;
        let Some(desc) =
            query_property(&h, PROP_DEVICE, 1024).and_then(|b| parse_device_descriptor(&b))
        else {
            continue;
        };
        let ssd = query_property(&h, PROP_SEEK_PENALTY, 12).and_then(|b| b.get(8).map(|&v| v == 0));
        let nvme_health = if desc.bus == BusType::Nvme {
            // The protocol-specific query generally needs read/write access (administrator).
            open(&path, GENERIC_READ | GENERIC_WRITE)
                .and_then(|rw| nvme_health(&rw))
                .or_else(|| nvme_health(&h))
        } else {
            None
        };
        let ata_smart = if desc.bus.smart_capable() {
            open(&path, GENERIC_READ | GENERIC_WRITE).and_then(|rw| ata_smart(&rw, index))
        } else {
            None
        };
        out.push(Drive {
            index,
            model: desc.product,
            vendor: desc.vendor,
            firmware: desc.revision,
            serial: desc.serial,
            bus: desc.bus,
            size_bytes: disk_size(&h).unwrap_or(0),
            ssd,
            removable: desc.removable,
            volumes: vols
                .iter()
                .filter(|(d, _)| *d == index)
                .map(|(_, v)| v.clone())
                .collect(),
            nvme_health,
            ata_smart,
        });
    }
    out
}

/// Drive temperatures plus PDH read/write throughput and activity.
pub struct StorageBackend {
    /// (handle, is NVMe, sensor prefix, group) per drive, for temperature queries.
    handles: Vec<(Handle, bool, String, String)>,
    /// (disk number, sensor prefix, group), for matching PDH instances.
    by_index: Vec<(u32, String, String)>,
    query: Option<Query>,
    read: Option<Counter>,
    write: Option<Counter>,
    idle: Option<Counter>,
    buf: Vec<u64>,
}

impl StorageBackend {
    pub fn new(drives: &[Drive]) -> Self {
        let group = |d: &Drive| format!("Drive: {}", d.name());
        let handles = drives
            .iter()
            .filter_map(|d| {
                let h = open(&format!(r"\\.\PhysicalDrive{}", d.index), 0)?;
                Some((h, d.bus == BusType::Nvme, d.sensor_prefix(), group(d)))
            })
            .collect();
        let by_index = drives
            .iter()
            .map(|d| (d.index, d.sensor_prefix(), group(d)))
            .collect();
        let query = Query::open().ok();
        let add = |p: &str| query.as_ref().and_then(|q| q.add(p));
        let read = add(r"\PhysicalDisk(*)\Disk Read Bytes/sec");
        let write = add(r"\PhysicalDisk(*)\Disk Write Bytes/sec");
        let idle = add(r"\PhysicalDisk(*)\% Idle Time");
        if let Some(q) = &query {
            let _ = q.collect();
        }
        StorageBackend {
            handles,
            by_index,
            query,
            read,
            write,
            idle,
            buf: Vec::new(),
        }
    }
}

impl SensorBackend for StorageBackend {
    fn name(&self) -> &str {
        "Storage"
    }

    fn poll(&mut self, out: &mut Vec<Reading>) -> Result<(), String> {
        for (h, nvme, prefix, group) in &self.handles {
            // Some NVMe drivers report no sensors through the generic property; use the health log instead.
            let t = temperature(h)
                .and_then(|t| t.sensors.first().copied())
                .map(i32::from)
                .or_else(|| {
                    nvme.then(|| nvme_health(h).map(|n| n.temperature_c))
                        .flatten()
                });
            if let Some(t) = t.filter(|&t| t > 0) {
                out.push(Reading {
                    id: format!("{prefix}/temp"),
                    label: "Drive Temperature".into(),
                    group: group.clone(),
                    unit: Unit::Celsius,
                    value: t as f64,
                });
            }
        }
        let Some(q) = &self.query else { return Ok(()) };
        q.collect()?;
        let specs = [
            (&self.idle, "activity", "Drive Activity", Unit::Percent),
            (&self.read, "read", "Read Rate", Unit::MegabytesPerSec),
            (&self.write, "write", "Write Rate", Unit::MegabytesPerSec),
        ];
        for (counter, id, label, unit) in specs {
            let Some(c) = counter else { continue };
            for (name, v) in read_array(c, PDH_FMT_DOUBLE | PDH_FMT_NOCAP100, &mut self.buf) {
                let Some(disk) = parse_disk_instance(&name) else {
                    continue;
                };
                let Some((_, prefix, group)) = self.by_index.iter().find(|(i, _, _)| *i == disk)
                else {
                    continue;
                };
                let value = match unit {
                    Unit::Percent => (100.0 - v).clamp(0.0, 100.0),
                    _ => v / 1_048_576.0,
                };
                out.push(Reading {
                    id: format!("{prefix}/{id}"),
                    label: label.into(),
                    group: group.clone(),
                    unit,
                    value,
                });
            }
        }
        Ok(())
    }
}

/// Whether any physical drive is on a bus where ATA SMART could be read (with admin rights).
/// Cheap: only the device descriptor is queried.
pub fn has_smart_capable_drive() -> bool {
    let mut misses = 0;
    for index in 0..64u32 {
        let Some(h) = open(&format!(r"\\.\PhysicalDrive{index}"), 0) else {
            misses += 1;
            if misses > 8 {
                break;
            }
            continue;
        };
        misses = 0;
        if query_property(&h, PROP_DEVICE, 1024)
            .and_then(|b| parse_device_descriptor(&b))
            .is_some_and(|d| d.bus.smart_capable())
        {
            return true;
        }
    }
    false
}
