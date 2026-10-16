//! Fixtures shared by the check tests.

use super::{Context, Level, Outcome};
use crate::SystemInfo;
use crate::sensors::{Reading, SensorTable, Unit};
use crate::smbios::{Bios, MemoryDevice, Smbios};
use crate::win::MemoryStatus;

pub(super) fn dimm(
    locator: &str,
    size_mb: u64,
    part: &str,
    speed: u32,
    configured: u32,
) -> MemoryDevice {
    MemoryDevice {
        locator: locator.into(),
        size_mb,
        part_number: part.into(),
        speed_mts: speed,
        configured_speed_mts: configured,
        memory_type: "DDR4",
        form_factor: "DIMM",
        ..Default::default()
    }
}

/// A desktop with these modules, a 2024 BIOS and 32 GB of RAM.
pub(super) fn system(dimms: Vec<MemoryDevice>) -> SystemInfo {
    SystemInfo {
        smbios: Some(Smbios {
            memory_slots: 4,
            memory: dimms,
            bios: Some(Bios {
                vendor: "AMI".into(),
                version: "1.0".into(),
                release_date: "10/21/2024".into(),
            }),
            ..Default::default()
        }),
        memory: Some(MemoryStatus {
            load_percent: 30,
            total_bytes: 32 << 30,
            available_bytes: 20 << 30,
        }),
        ..Default::default()
    }
}

pub(super) fn ctx<'a>(info: &'a SystemInfo, sensors: &'a SensorTable) -> Context<'a> {
    Context {
        info,
        sensors,
        today: (2026, 10, 12),
        uptime_secs: Some(3600),
        power: None,
    }
}

pub(super) fn feed(t: &mut SensorTable, id: &str, unit: Unit, values: &[f64]) {
    for &v in values {
        t.update([Reading {
            id: id.into(),
            label: id.into(),
            group: "g".into(),
            unit,
            value: v,
        }]);
    }
}

pub(super) fn levels(o: &Outcome) -> Vec<Level> {
    match o {
        Outcome::Findings(f) => f.iter().map(|f| f.level).collect(),
        _ => Vec::new(),
    }
}
