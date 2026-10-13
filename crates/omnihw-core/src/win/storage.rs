//! Physical drives via `\\.\PhysicalDriveN` storage IOCTLs, volumes, and live disk sensors.

use super::pdh::{Counter, PDH_FMT_DOUBLE_NOCAP100, Query, read_array};
use crate::sensors::{Reading, SensorBackend, Unit};
use crate::smart::{AtaSmart, parse_ata_smart};
use crate::storage::*;
use std::mem::offset_of;
use std::ptr;
use windows::Win32::Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, GetDiskFreeSpaceExW,
    GetLogicalDrives, GetVolumeInformationW, OPEN_EXISTING,
};
use windows::Win32::Storage::Nvme::NVME_LOG_PAGE_HEALTH_INFO;
use windows::Win32::System::IO::DeviceIoControl;
use windows::Win32::System::Ioctl::{
    DISK_GEOMETRY_EX, IDEREGS, IOCTL_DISK_GET_DRIVE_GEOMETRY_EX, IOCTL_STORAGE_GET_DEVICE_NUMBER,
    IOCTL_STORAGE_QUERY_PROPERTY, NVMeDataTypeLogPage, PropertyStandardQuery, ProtocolTypeNvme,
    READ_ATTRIBUTES, SENDCMDINPARAMS, SENDCMDOUTPARAMS, SMART_CMD, SMART_CYL_HI, SMART_CYL_LOW,
    SMART_RCV_DRIVE_DATA, STORAGE_DEVICE_NUMBER, STORAGE_PROPERTY_ID, STORAGE_PROPERTY_QUERY,
    STORAGE_PROTOCOL_DATA_DESCRIPTOR, STORAGE_PROTOCOL_SPECIFIC_DATA, StorageDeviceProperty,
    StorageDeviceProtocolSpecificProperty, StorageDeviceSeekPenaltyProperty,
};
use windows::core::HSTRING;

/// Property 11, kept from the original hand-written code. Note that 11 is
/// `StorageDeviceLBProvisioningProperty`; `StorageDeviceTemperatureProperty` is 52.
const PROP_TEMPERATURE: STORAGE_PROPERTY_ID = STORAGE_PROPERTY_ID(11);
const NVME_LOG_DATA_SIZE: u32 = 512;

/// Owned Win32 handle.
struct Handle(HANDLE);

// SAFETY: device handles may be used from any thread.
unsafe impl Send for Handle {}

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: valid handle owned by us.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

fn open(path: &str, access: u32) -> Option<Handle> {
    // SAFETY: NUL-terminated path; no template.
    unsafe {
        CreateFileW(
            &HSTRING::from(path),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(0),
            None,
        )
    }
    .ok()
    .map(Handle)
}

/// `DeviceIoControl` with `input` sent as-is; returns the number of bytes written to `out`.
fn ioctl<I: ?Sized>(h: &Handle, code: u32, input: &I, out: &mut [u8]) -> Option<usize> {
    let mut got = 0;
    // SAFETY: buffers are valid for the given lengths.
    unsafe {
        DeviceIoControl(
            h.0,
            code,
            Some((input as *const I).cast()),
            size_of_val(input) as u32,
            Some(out.as_mut_ptr().cast()),
            out.len() as u32,
            Some(&mut got),
            None,
        )
    }
    .ok()?;
    Some(got as usize)
}

/// A Win32 struct from the start of an IOCTL output buffer.
fn read_struct<T: Copy>(b: &[u8]) -> Option<T> {
    // SAFETY: length checked; the buffer is fully initialized and the read is unaligned-safe.
    (b.len() >= size_of::<T>()).then(|| unsafe { ptr::read_unaligned(b.as_ptr().cast()) })
}

/// `IOCTL_STORAGE_QUERY_PROPERTY` with a standard query for `property`.
fn query_property(h: &Handle, property: STORAGE_PROPERTY_ID, out_len: usize) -> Option<Vec<u8>> {
    let q = STORAGE_PROPERTY_QUERY {
        PropertyId: property,
        QueryType: PropertyStandardQuery,
        AdditionalParameters: [0],
    };
    let mut out = vec![0u8; out_len];
    let n = ioctl(h, IOCTL_STORAGE_QUERY_PROPERTY, &q, &mut out)?;
    out.truncate(n);
    Some(out)
}

fn temperature(h: &Handle) -> Option<TemperatureData> {
    parse_temperature_descriptor(&query_property(h, PROP_TEMPERATURE, 512)?)
}

fn nvme_health(h: &Handle) -> Option<NvmeHealth> {
    // STORAGE_PROPERTY_QUERY whose AdditionalParameters hold STORAGE_PROTOCOL_SPECIFIC_DATA,
    // followed by room for the log page.
    const SPEC_AT: usize = offset_of!(STORAGE_PROPERTY_QUERY, AdditionalParameters);
    const SPEC_SIZE: usize = size_of::<STORAGE_PROTOCOL_SPECIFIC_DATA>();
    let mut q = vec![0u8; SPEC_AT + SPEC_SIZE + NVME_LOG_DATA_SIZE as usize];
    let query = STORAGE_PROPERTY_QUERY {
        PropertyId: StorageDeviceProtocolSpecificProperty,
        QueryType: PropertyStandardQuery,
        AdditionalParameters: [0],
    };
    let spec = STORAGE_PROTOCOL_SPECIFIC_DATA {
        ProtocolType: ProtocolTypeNvme,
        DataType: NVMeDataTypeLogPage.0 as u32,
        ProtocolDataRequestValue: NVME_LOG_PAGE_HEALTH_INFO.0 as u32,
        ProtocolDataOffset: SPEC_SIZE as u32,
        ProtocolDataLength: NVME_LOG_DATA_SIZE,
        ..Default::default()
    };
    // SAFETY: both writes are in bounds; `spec` overwrites the query's AdditionalParameters.
    unsafe {
        ptr::write_unaligned(q.as_mut_ptr().cast(), query);
        ptr::write_unaligned(q.as_mut_ptr().add(SPEC_AT).cast(), spec);
    }
    let mut out = vec![0u8; q.len()];
    let n = ioctl(h, IOCTL_STORAGE_QUERY_PROPERTY, q.as_slice(), &mut out)?;
    let d: STORAGE_PROTOCOL_DATA_DESCRIPTOR = read_struct(&out)?;
    let data = d.ProtocolSpecificData;
    let start = offset_of!(STORAGE_PROTOCOL_DATA_DESCRIPTOR, ProtocolSpecificData)
        + data.ProtocolDataOffset as usize;
    parse_nvme_health(out.get(start..(start + data.ProtocolDataLength as usize).min(n))?)
}

/// ATA SMART READ DATA through the legacy SMART IOCTL. Needs a read/write handle, which
/// Windows only grants to administrators for physical drives.
fn ata_smart(h: &Handle, drive: u32) -> Option<AtaSmart> {
    let input = SENDCMDINPARAMS {
        cBufferSize: 512,
        irDriveRegs: IDEREGS {
            bFeaturesReg: READ_ATTRIBUTES as u8,
            bSectorCountReg: 1,
            bSectorNumberReg: 1,
            bCylLowReg: SMART_CYL_LOW as u8,
            bCylHighReg: SMART_CYL_HI as u8,
            bDriveHeadReg: 0xA0 | (((drive & 1) as u8) << 4),
            bCommandReg: SMART_CMD as u8,
            bReserved: 0,
        },
        bDriveNumber: drive as u8,
        ..Default::default()
    };
    // SAFETY: SENDCMDINPARAMS is packed(1) with no padding; the trailing 1-byte buffer is
    // not sent.
    let input = unsafe {
        std::slice::from_raw_parts(
            (&input as *const SENDCMDINPARAMS).cast::<u8>(),
            offset_of!(SENDCMDINPARAMS, bBuffer),
        )
    };
    const HEADER: usize = offset_of!(SENDCMDOUTPARAMS, bBuffer);
    let mut out = [0u8; HEADER + 512];
    let n = ioctl(h, SMART_RCV_DRIVE_DATA, input, &mut out)?;
    parse_ata_smart(out.get(HEADER..n)?)
}

fn disk_size(h: &Handle) -> Option<u64> {
    let mut out = [0u8; 256];
    ioctl(h, IOCTL_DISK_GET_DRIVE_GEOMETRY_EX, &[0u8; 0], &mut out)?;
    Some(read_struct::<DISK_GEOMETRY_EX>(&out)?.DiskSize as u64)
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
        let mut num = [0u8; size_of::<STORAGE_DEVICE_NUMBER>()];
        if ioctl(&h, IOCTL_STORAGE_GET_DEVICE_NUMBER, &[0u8; 0], &mut num).is_none() {
            continue;
        }
        let disk = read_struct::<STORAGE_DEVICE_NUMBER>(&num)
            .unwrap()
            .DeviceNumber;
        let root = HSTRING::from(format!(r"{letter}:\"));
        let (mut avail, mut total, mut free) = (0u64, 0u64, 0u64);
        // SAFETY: valid root path and out pointers.
        let _ = unsafe {
            GetDiskFreeSpaceExW(&root, Some(&mut avail), Some(&mut total), Some(&mut free))
        };
        let mut label = [0u16; 128];
        let mut fs = [0u16; 32];
        // SAFETY: buffer lengths passed with the slices.
        let _ = unsafe {
            GetVolumeInformationW(&root, Some(&mut label), None, None, None, Some(&mut fs))
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
        let Some(desc) = query_property(&h, StorageDeviceProperty, 1024)
            .and_then(|b| parse_device_descriptor(&b))
        else {
            continue;
        };
        let ssd = query_property(&h, StorageDeviceSeekPenaltyProperty, 12)
            .and_then(|b| b.get(8).map(|&v| v == 0));
        let nvme_health = if desc.bus == BusType::Nvme {
            // The protocol-specific query generally needs read/write access (administrator).
            open(&path, (GENERIC_READ | GENERIC_WRITE).0)
                .and_then(|rw| nvme_health(&rw))
                .or_else(|| nvme_health(&h))
        } else {
            None
        };
        let ata_smart = if desc.bus.smart_capable() {
            open(&path, (GENERIC_READ | GENERIC_WRITE).0).and_then(|rw| ata_smart(&rw, index))
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
            for (name, v) in read_array(c, PDH_FMT_DOUBLE_NOCAP100, &mut self.buf) {
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
        if query_property(&h, StorageDeviceProperty, 1024)
            .and_then(|b| parse_device_descriptor(&b))
            .is_some_and(|d| d.bus.smart_capable())
        {
            return true;
        }
    }
    false
}
