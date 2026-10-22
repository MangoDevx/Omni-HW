//! Fake machines for tests: complete [`SystemInfo`]s and sensor readings that don't depend on
//! the PC running the tests. Built for this crate's tests and, through the `testkit` feature,
//! for the app's page tests.

use crate::SystemInfo;
use crate::cpuid::{CacheDesc, CacheKind, CpuInfo, Vendor};
use crate::gpu::{Display, GpuAdapter, NvmlDetails};
use crate::sensors::{Reading, SensorTable, Unit};
use crate::smbios::{self, Bios, Board, MemoryDevice, Processor, Smbios};
use crate::storage::{BusType, DiskLayout, Drive, NvmeHealth, PartitionStyle, Volume};
use crate::topology::{CacheInstance, Core, LogicalId, Topology};
use crate::win::{Battery, MemoryStatus, OsInfo};

const GIB: u64 = 1 << 30;

fn caches(l3_mb: u64, cores: u32) -> Vec<CacheDesc> {
    let c = |level, kind, size_bytes, ways, shared_by| CacheDesc {
        level,
        kind,
        size_bytes,
        ways,
        line_size: 64,
        sets: (size_bytes / 64 / u64::from(ways)) as u32,
        shared_by,
    };
    vec![
        c(1, CacheKind::Data, 32 << 10, 8, 2),
        c(1, CacheKind::Instruction, 32 << 10, 8, 2),
        c(2, CacheKind::Unified, 1 << 20, 8, 2),
        c(3, CacheKind::Unified, l3_mb << 20, 16, cores * 2),
    ]
}

/// `cores` cores with two threads each (`efficiency_class` per core from `classes`, else 0).
fn topology(cores: u8, smt: bool, classes: &[u8], l3_mb: u64) -> Topology {
    let per = if smt { 2 } else { 1 };
    let cores: Vec<Core> = (0..cores)
        .map(|i| Core {
            efficiency_class: classes.get(i as usize).copied().unwrap_or(0),
            smt,
            logical: (0..per)
                .map(|t| LogicalId {
                    group: 0,
                    number: i * per + t,
                })
                .collect(),
        })
        .collect();
    let all: Vec<LogicalId> = cores.iter().flat_map(|c| c.logical.clone()).collect();
    Topology {
        packages: 1,
        dies: 1,
        numa_nodes: 1,
        groups: 1,
        caches: vec![CacheInstance {
            level: 3,
            kind: CacheKind::Unified,
            size_bytes: l3_mb << 20,
            associativity: 16,
            line_size: 64,
            logical: all,
        }],
        cores,
    }
}

fn dimm(locator: &str, size_mb: u64, part: &str, speed: u32, kind: &'static str) -> MemoryDevice {
    MemoryDevice {
        locator: locator.into(),
        bank_locator: format!("BANK {}", &locator[locator.len() - 1..]),
        size_mb,
        form_factor: if kind == "LPDDR5" {
            "Row of chips"
        } else {
            "DIMM"
        },
        memory_type: kind,
        speed_mts: speed,
        configured_speed_mts: speed,
        manufacturer: "Kingston".into(),
        part_number: part.into(),
        serial: "1A2B3C4D".into(),
        rank: 2,
        data_width: 64,
        configured_voltage_mv: 1350,
    }
}

/// A desktop: AMD Ryzen 7 7800X3D on an ASUS B650 board, 2 x 16 GB DDR5-6000, an NVIDIA RTX
/// 4070 (with NVML details), a Samsung 990 PRO NVMe and a SATA hard drive, Windows 11.
pub fn desktop() -> SystemInfo {
    let cpu = CpuInfo {
        vendor: Vendor::Amd,
        vendor_string: "AuthenticAMD".into(),
        brand: "AMD Ryzen 7 7800X3D 8-Core Processor".into(),
        family: 0x19,
        model: 0x61,
        stepping: 2,
        base_family: 0xF,
        base_model: 1,
        ext_family: 0xA,
        ext_model: 6,
        max_leaf: 0x10,
        max_ext_leaf: 0x8000_0028,
        features: vec!["SSE4.2", "AVX", "AVX2", "AVX-512F", "AES", "SHA"],
        hybrid: false,
        caches: caches(96, 8),
        phys_addr_bits: 48,
        virt_addr_bits: 48,
    };
    SystemInfo {
        codename: crate::codename::lookup(cpu.vendor, cpu.family, cpu.model),
        cpu,
        topology: Some(topology(8, true, &[], 96)),
        smbios: Some(Smbios {
            version: (3, 6),
            bios: Some(Bios {
                vendor: "American Megatrends Inc.".into(),
                version: "2613".into(),
                release_date: "05/10/2026".into(),
            }),
            system: Some(smbios::SystemInfo {
                manufacturer: "ASUS".into(),
                product: "System Product Name".into(),
                ..Default::default()
            }),
            board: Some(Board {
                manufacturer: "ASUSTeK COMPUTER INC.".into(),
                product: "ROG STRIX B650E-F GAMING WIFI".into(),
                version: "Rev 1.xx".into(),
            }),
            processors: vec![Processor {
                socket: "AM5".into(),
                manufacturer: "Advanced Micro Devices, Inc.".into(),
                version: "AMD Ryzen 7 7800X3D 8-Core Processor".into(),
                external_clock_mhz: 100,
                max_speed_mhz: 5050,
                current_speed_mhz: 4200,
                core_count: 8,
                thread_count: 16,
            }],
            memory_slots: 4,
            memory: vec![
                dimm("DIMM_A1", 0, "", 0, "Unknown"),
                dimm("DIMM_A2", 16384, "KF560C30-16", 6000, "DDR5"),
                dimm("DIMM_B1", 0, "", 0, "Unknown"),
                dimm("DIMM_B2", 16384, "KF560C30-16", 6000, "DDR5"),
            ],
        }),
        base_mhz: Some(4200),
        memory: Some(MemoryStatus {
            load_percent: 38,
            total_bytes: 32 * GIB,
            available_bytes: 20 * GIB,
        }),
        gpus: vec![GpuAdapter {
            name: "NVIDIA GeForce RTX 4070".into(),
            vendor_id: 0x10DE,
            device_id: 0x2786,
            subsys_id: 0x8A8F_1462,
            revision: 0xA1,
            dedicated_vram: 12 * GIB,
            shared_memory: 16 * GIB,
            luid: 0x1234,
            driver_version: "32.0.15.8157".into(),
            nvml: Some(NvmlDetails {
                index: 0,
                name: "NVIDIA GeForce RTX 4070".into(),
                driver_version: "581.57".into(),
                vbios: "95.04.31.00.6B".into(),
                bus_id: "00000000:01:00.0".into(),
                memory_total: 12 * GIB,
                memory_bus_width: Some(192),
                cuda_cores: Some(5888),
                max_core_clock_mhz: Some(3105),
                max_mem_clock_mhz: Some(10501),
                power_limit_w: Some(200.0),
                pcie_current: Some((4, 16)),
                pcie_max: Some((4, 16)),
                bar1_total: Some(16 * GIB),
            }),
            displays: vec![Display {
                name: r"\\.\DISPLAY1".into(),
                width: 2560,
                height: 1440,
            }],
            driver_date: Some((2026, 9, 30)),
        }],
        drives: vec![
            Drive {
                index: 0,
                model: "Samsung SSD 990 PRO 2TB".into(),
                vendor: "Samsung".into(),
                firmware: "4B2QJXD7".into(),
                serial: "S6Z2NJ0W123456".into(),
                bus: BusType::Nvme,
                size_bytes: 2_000_398_934_016,
                ssd: Some(true),
                removable: false,
                volumes: vec![Volume {
                    mount: "C:".into(),
                    label: "Windows".into(),
                    file_system: "NTFS".into(),
                    total_bytes: 1_999_000_000_000,
                    free_bytes: 1_200_000_000_000,
                }],
                nvme_health: Some(NvmeHealth {
                    temperature_c: 41,
                    available_spare_pct: 100,
                    spare_threshold_pct: 10,
                    percentage_used: 2,
                    data_read_bytes: 40_000_000_000_000,
                    data_written_bytes: 25_000_000_000_000,
                    power_cycles: 812,
                    power_on_hours: 4120,
                    unsafe_shutdowns: 31,
                    sensor_temps_c: vec![41, 52],
                    ..Default::default()
                }),
                ata_smart: None,
                layout: Some(DiskLayout {
                    style: PartitionStyle::Gpt,
                    partitions: 4,
                    unallocated_bytes: 0,
                }),
                sata_link: None,
            },
            Drive {
                index: 1,
                model: "ST4000DM004-2U9104".into(),
                vendor: "Seagate".into(),
                firmware: "0001".into(),
                serial: "ZFN1ABCD".into(),
                bus: BusType::Sata,
                size_bytes: 4_000_787_030_016,
                ssd: Some(false),
                removable: false,
                volumes: vec![Volume {
                    mount: "D:".into(),
                    label: "Data".into(),
                    file_system: "NTFS".into(),
                    total_bytes: 4_000_650_883_072,
                    free_bytes: 900_000_000_000,
                }],
                nvme_health: None,
                ata_smart: None,
                layout: Some(DiskLayout {
                    style: PartitionStyle::Gpt,
                    partitions: 2,
                    unallocated_bytes: 0,
                }),
                sata_link: None,
            },
        ],
        os: Some(OsInfo {
            product: "Windows 11 Pro".into(),
            display_version: "25H2".into(),
            build: 26200,
            ubr: 6899,
            computer_name: "DESKTOP-TEST".into(),
        }),
        battery: None,
        battery_health: None,
        microcode: Some(0x0A60_1209),
        ..Default::default()
    }
}

/// A laptop: Intel Core i7-1360P (hybrid), 16 GB LPDDR5, Intel Iris Xe graphics, one NVMe drive
/// and a battery at 72 % on battery power.
pub fn laptop() -> SystemInfo {
    let cpu = CpuInfo {
        vendor: Vendor::Intel,
        vendor_string: "GenuineIntel".into(),
        brand: "13th Gen Intel(R) Core(TM) i7-1360P".into(),
        family: 6,
        model: 0xBA,
        stepping: 2,
        base_family: 6,
        base_model: 0xA,
        ext_family: 0,
        ext_model: 0xB,
        max_leaf: 0x20,
        max_ext_leaf: 0x8000_0008,
        features: vec!["SSE4.2", "AVX", "AVX2", "AES", "SHA"],
        hybrid: true,
        caches: caches(18, 12),
        phys_addr_bits: 39,
        virt_addr_bits: 48,
    };
    // 4 P-cores (class 1, two threads) then 8 E-cores (class 0, one thread).
    let mut topo = topology(4, true, &[1, 1, 1, 1], 18);
    let e = topology(8, false, &[], 18);
    topo.cores
        .extend(e.cores.into_iter().enumerate().map(|(i, mut c)| {
            c.logical = vec![LogicalId {
                group: 0,
                number: 8 + i as u8,
            }];
            c
        }));
    SystemInfo {
        codename: crate::codename::lookup(cpu.vendor, cpu.family, cpu.model),
        cpu,
        topology: Some(topo),
        smbios: Some(Smbios {
            version: (3, 4),
            bios: Some(Bios {
                vendor: "LENOVO".into(),
                version: "N3JET41W (1.21)".into(),
                release_date: "03/02/2025".into(),
            }),
            system: Some(smbios::SystemInfo {
                manufacturer: "LENOVO".into(),
                product: "21HM0012US".into(),
                version: "ThinkPad X1 Carbon Gen 11".into(),
                family: "ThinkPad X1 Carbon Gen 11".into(),
            }),
            board: Some(Board {
                manufacturer: "LENOVO".into(),
                product: "21HM0012US".into(),
                version: "SDK0T76530 WIN".into(),
            }),
            processors: Vec::new(),
            memory_slots: 2,
            memory: vec![
                dimm(
                    "Controller0-ChannelA",
                    8192,
                    "MT62F1G32D2DS-026",
                    6400,
                    "LPDDR5",
                ),
                dimm(
                    "Controller1-ChannelA",
                    8192,
                    "MT62F1G32D2DS-026",
                    6400,
                    "LPDDR5",
                ),
            ],
        }),
        base_mhz: Some(2200),
        memory: Some(MemoryStatus {
            load_percent: 61,
            total_bytes: 16 * GIB,
            available_bytes: 6 * GIB,
        }),
        gpus: vec![GpuAdapter {
            name: "Intel(R) Iris(R) Xe Graphics".into(),
            vendor_id: 0x8086,
            device_id: 0xA7A0,
            dedicated_vram: 128 << 20,
            shared_memory: 8 * GIB,
            luid: 0x5678,
            driver_version: "32.0.101.6913".into(),
            displays: vec![Display {
                name: r"\\.\DISPLAY1".into(),
                width: 2880,
                height: 1800,
            }],
            driver_date: Some((2026, 6, 12)),
            ..Default::default()
        }],
        drives: vec![Drive {
            index: 0,
            model: "SAMSUNG MZVL2512HCJQ-00BL7".into(),
            vendor: "Samsung".into(),
            bus: BusType::Nvme,
            size_bytes: 512_110_190_592,
            ssd: Some(true),
            volumes: vec![Volume {
                mount: "C:".into(),
                label: "Windows".into(),
                file_system: "NTFS".into(),
                total_bytes: 510_000_000_000,
                free_bytes: 40_000_000_000,
            }],
            nvme_health: Some(NvmeHealth {
                temperature_c: 38,
                available_spare_pct: 100,
                spare_threshold_pct: 10,
                percentage_used: 7,
                power_on_hours: 2210,
                power_cycles: 1950,
                ..Default::default()
            }),
            ..Default::default()
        }],
        os: Some(OsInfo {
            product: "Windows 11 Pro".into(),
            display_version: "24H2".into(),
            build: 26100,
            ubr: 4351,
            computer_name: "LAPTOP-TEST".into(),
        }),
        battery: Some(Battery {
            percent: 72,
            on_ac: false,
            charging: false,
            seconds_left: Some(3 * 3600 + 20 * 60),
        }),
        ..Default::default()
    }
}

fn reading(id: &str, label: &str, group: &str, unit: Unit, value: f64) -> Reading {
    Reading {
        id: id.into(),
        label: label.into(),
        group: group.into(),
        unit,
        value,
    }
}

/// One sample of typical sensor readings for `info` (CPU clocks, loads and temperatures,
/// memory, and NVML-style GPU readings for an NVIDIA GPU).
pub fn readings(info: &SystemInfo) -> Vec<Reading> {
    let mut r = vec![
        reading("cpu/clock/avg", "Average Clock", "CPU", Unit::Mhz, 4850.0),
        reading("cpu/load/total", "Total Load", "CPU", Unit::Percent, 23.0),
        reading(
            "cpu/temp/package",
            "Package",
            "CPU Temperatures",
            Unit::Celsius,
            61.0,
        ),
        reading(
            "cpu/power/package",
            "Package Power",
            "CPU Power",
            Unit::Watt,
            48.5,
        ),
        reading("mem/load", "Memory Load", "Memory", Unit::Percent, 38.0),
        reading(
            "mem/used",
            "Memory Used",
            "Memory",
            Unit::Megabytes,
            12_450.0,
        ),
    ];
    let cores = info.topology.as_ref().map_or(0, |t| t.cores.len());
    for i in 0..cores {
        r.push(reading(
            &format!("cpu/clock/core{i}"),
            &format!("Core {i}"),
            "CPU Clocks",
            Unit::Mhz,
            4700.0 + 25.0 * i as f64,
        ));
        r.push(reading(
            &format!("cpu/temp/core{i}"),
            &format!("Core {i}"),
            "CPU Temperatures",
            Unit::Celsius,
            55.0 + i as f64,
        ));
    }
    for l in info
        .topology
        .iter()
        .flat_map(|t| t.cores.iter().flat_map(|c| c.logical.iter()))
    {
        r.push(reading(
            &format!("cpu/load/{},{}", l.group, l.number),
            &format!("CPU {}", l.number),
            "CPU Load",
            Unit::Percent,
            10.0 + f64::from(l.number % 5) * 7.0,
        ));
    }
    if let Some(b) = info.battery {
        r.push(reading(
            "battery/charge",
            "Charge Level",
            "Battery",
            Unit::Percent,
            f64::from(b.percent),
        ));
        r.push(reading(
            "battery/ac",
            "On AC Power",
            "Battery",
            Unit::None,
            if b.on_ac { 1.0 } else { 0.0 },
        ));
    }
    for g in info.gpus.iter().filter(|g| g.nvml.is_some()) {
        let p = g.sensor_prefix();
        let group = format!("GPU: {}", g.name);
        r.extend([
            reading(
                &format!("{p}/temp"),
                "GPU Temperature",
                &group,
                Unit::Celsius,
                47.0,
            ),
            reading(
                &format!("{p}/clock/core"),
                "GPU Core Clock",
                &group,
                Unit::Mhz,
                2610.0,
            ),
            reading(
                &format!("{p}/clock/mem"),
                "GPU Memory Clock",
                &group,
                Unit::Mhz,
                10501.0,
            ),
            reading(
                &format!("{p}/power"),
                "GPU Power",
                &group,
                Unit::Watt,
                142.0,
            ),
            reading(&format!("{p}/fan"), "GPU Fan", &group, Unit::Percent, 41.0),
            reading(
                &format!("{p}/nv/load"),
                "GPU Core Load",
                &group,
                Unit::Percent,
                87.0,
            ),
            reading(
                &format!("{p}/vram/used"),
                "GPU Memory Used",
                &group,
                Unit::Megabytes,
                6120.0,
            ),
        ]);
    }
    r
}

/// A sensor table holding `samples` samples of [`readings`] (enough for min/max and charts).
pub fn sensor_table(info: &SystemInfo, samples: usize) -> SensorTable {
    let mut t = SensorTable::default();
    for _ in 0..samples {
        t.update(readings(info));
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn machines_are_complete() {
        for info in [desktop(), laptop()] {
            assert!(info.codename.is_some(), "{}", info.cpu.brand);
            let smbios = info.smbios.as_ref().unwrap();
            assert!(smbios.populated_memory().count() >= 2);
            assert!(!info.gpus.is_empty() && !info.drives.is_empty());
            let t = sensor_table(&info, 3);
            assert!(t.entries().len() > 10);
        }
        assert_eq!(laptop().topology.unwrap().cores.len(), 12);
    }
}
