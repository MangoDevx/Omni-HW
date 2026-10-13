//! Live CPU sensors from PDH counters, plus memory load.
//!
//! Per-core clock = `% Processor Performance` x nominal MHz. Load is
//! `% Processor Utility` (the counter Task Manager uses).

use crate::topology::LogicalId;

/// Parse a `Processor Information` instance name such as "0,13".
/// Returns `None` for "_Total" and "0,_Total".
pub fn parse_instance(name: &str) -> Option<LogicalId> {
    let (g, n) = name.split_once(',')?;
    Some(LogicalId {
        group: g.trim().parse().ok()?,
        number: n.trim().parse().ok()?,
    })
}

/// Friendly label for an `Energy Meter` instance such as "RAPL_Package0_PKG".
pub fn energy_label(instance: &str) -> String {
    let domain = instance.rsplit('_').next().unwrap_or(instance);
    match domain {
        "PKG" => "CPU Package Power".into(),
        "PP0" => "CPU Cores Power".into(),
        "PP1" => "CPU Graphics Power".into(),
        "DRAM" => "DRAM Power".into(),
        "PSYS" => "Platform Power".into(),
        _ => instance.to_string(),
    }
}

#[cfg(windows)]
pub use imp::{BatteryBackend, CpuPerfBackend, GpuPerfBackend, MemoryBackend};

#[cfg(windows)]
mod imp {
    use super::{energy_label, parse_instance};
    use crate::gpu::{GpuAdapter, aggregate_engine_load, engtype_label, parse_adapter_instance};
    use crate::sensors::{Reading, SensorBackend, Unit};
    use crate::topology::Topology;
    use crate::win::pdh::{Counter, PDH_FMT_DOUBLE, PDH_FMT_DOUBLE_NOCAP100, Query, read_array};
    use std::collections::HashMap;

    /// Per-core clocks, per-thread load, total load and (if exposed) RAPL power.
    pub struct CpuPerfBackend {
        query: Query,
        perf: Option<Counter>,
        utility: Option<Counter>,
        energy: Option<Counter>,
        base_mhz: f64,
        /// Core index for each logical processor, and core labels.
        core_of: HashMap<(u16, u8), usize>,
        core_labels: Vec<String>,
        buf: Vec<u64>,
    }

    impl CpuPerfBackend {
        pub fn new(base_mhz: u32, topology: Option<&Topology>) -> Result<Self, String> {
            let query = Query::open()?;
            let perf = query.add(r"\Processor Information(*)\% Processor Performance");
            let utility = query.add(r"\Processor Information(*)\% Processor Utility");
            let energy = query.add(r"\Energy Meter(*)\Power");
            if perf.is_none() && utility.is_none() {
                return Err("Processor Information counters unavailable".into());
            }
            // Rate counters need a first sample to diff against.
            query.collect()?;
            let mut core_of = HashMap::new();
            let mut core_labels = Vec::new();
            if let Some(t) = topology {
                for (i, c) in t.cores.iter().enumerate() {
                    for l in &c.logical {
                        core_of.insert((l.group, l.number), i);
                    }
                    core_labels.push(t.core_label(i));
                }
            }
            Ok(Self {
                query,
                perf,
                utility,
                energy,
                base_mhz: base_mhz as f64,
                core_of,
                core_labels,
                buf: Vec::new(),
            })
        }
    }

    impl SensorBackend for CpuPerfBackend {
        fn name(&self) -> &str {
            "PDH"
        }

        fn poll(&mut self, out: &mut Vec<Reading>) -> Result<(), String> {
            self.query.collect()?;
            let fmt = PDH_FMT_DOUBLE_NOCAP100;

            if let Some(c) = &self.perf {
                let mut per_core: Vec<f64> = vec![f64::NAN; self.core_labels.len()];
                let mut threads = read_array(c, fmt, &mut self.buf);
                threads.sort_by_key(|(n, _)| parse_instance(n));
                for (name, pct) in threads {
                    let mhz = pct / 100.0 * self.base_mhz;
                    match parse_instance(&name) {
                        Some(id) => match self.core_of.get(&(id.group, id.number)) {
                            // Threads of one core share its clock; keep the highest.
                            Some(&core) => {
                                per_core[core] = if per_core[core].is_nan() {
                                    mhz
                                } else {
                                    per_core[core].max(mhz)
                                }
                            }
                            None => out.push(Reading {
                                id: format!("cpu/clock/{},{}", id.group, id.number),
                                label: format!("CPU {} Clock", id.number),
                                group: "CPU Clocks".into(),
                                unit: Unit::Mhz,
                                value: mhz,
                            }),
                        },
                        None if name == "_Total" => out.push(Reading {
                            id: "cpu/clock/avg".into(),
                            label: "Average Core Clock".into(),
                            group: "CPU Clocks".into(),
                            unit: Unit::Mhz,
                            value: mhz,
                        }),
                        None => {}
                    }
                }
                for (i, mhz) in per_core.into_iter().enumerate() {
                    if mhz.is_finite() {
                        out.push(Reading {
                            id: format!("cpu/clock/core{i}"),
                            label: format!("{} Clock", self.core_labels[i]),
                            group: "CPU Clocks".into(),
                            unit: Unit::Mhz,
                            value: mhz,
                        });
                    }
                }
            }

            if let Some(c) = &self.utility {
                let mut threads = read_array(c, fmt, &mut self.buf);
                threads.sort_by_key(|(n, _)| parse_instance(n));
                for (name, pct) in threads {
                    let value = pct.clamp(0.0, 100.0);
                    match parse_instance(&name) {
                        Some(id) => {
                            let label = match self.core_of.get(&(id.group, id.number)) {
                                Some(&core) => {
                                    format!("{} / T{}", self.core_labels[core], id.number)
                                }
                                None => format!("CPU {}", id.number),
                            };
                            out.push(Reading {
                                id: format!("cpu/load/{},{}", id.group, id.number),
                                label: format!("{label} Usage"),
                                group: "CPU Usage".into(),
                                unit: Unit::Percent,
                                value,
                            });
                        }
                        None if name == "_Total" => out.push(Reading {
                            id: "cpu/load/total".into(),
                            label: "Total CPU Usage".into(),
                            group: "CPU Usage".into(),
                            unit: Unit::Percent,
                            value,
                        }),
                        None => {}
                    }
                }
            }

            if let Some(c) = &self.energy {
                for (name, mw) in read_array(c, PDH_FMT_DOUBLE, &mut self.buf) {
                    if name == "_Total" {
                        continue;
                    }
                    // Energy Meter "Power" is reported in milliwatts.
                    out.push(Reading {
                        id: format!("power/{name}"),
                        label: energy_label(&name),
                        group: "Power".into(),
                        unit: Unit::Watt,
                        value: mw / 1000.0,
                    });
                }
            }
            Ok(())
        }
    }

    /// Vendor-neutral GPU load (per engine type) and VRAM usage, as shown by Task Manager.
    pub struct GpuPerfBackend {
        query: Query,
        engines: Option<Counter>,
        dedicated: Option<Counter>,
        shared: Option<Counter>,
        /// LUID -> (sensor id prefix, group name)
        adapters: HashMap<u64, (String, String)>,
        buf: Vec<u64>,
    }

    impl GpuPerfBackend {
        pub fn new(adapters: &[GpuAdapter]) -> Result<Self, String> {
            if adapters.is_empty() {
                return Err("no GPU adapters".into());
            }
            let query = Query::open()?;
            let engines = query.add(r"\GPU Engine(*)\Utilization Percentage");
            let dedicated = query.add(r"\GPU Adapter Memory(*)\Dedicated Usage");
            let shared = query.add(r"\GPU Adapter Memory(*)\Shared Usage");
            if engines.is_none() && dedicated.is_none() {
                return Err("GPU counters unavailable".into());
            }
            query.collect()?;
            let adapters = adapters
                .iter()
                .map(|a| (a.luid, (a.sensor_prefix(), format!("GPU: {}", a.name))))
                .collect();
            Ok(Self {
                query,
                engines,
                dedicated,
                shared,
                adapters,
                buf: Vec::new(),
            })
        }
    }

    impl SensorBackend for GpuPerfBackend {
        fn name(&self) -> &str {
            "PDH GPU"
        }

        fn poll(&mut self, out: &mut Vec<Reading>) -> Result<(), String> {
            self.query.collect()?;
            if let Some(c) = &self.engines {
                let samples = read_array(c, PDH_FMT_DOUBLE_NOCAP100, &mut self.buf);
                let loads = aggregate_engine_load(samples.iter().map(|(n, v)| (n.as_str(), *v)));
                let mut luids: Vec<_> = loads.keys().copied().collect();
                luids.sort();
                for luid in luids {
                    let Some((prefix, group)) = self.adapters.get(&luid) else {
                        continue;
                    };
                    let load = &loads[&luid];
                    out.push(Reading {
                        id: format!("{prefix}/load"),
                        label: "GPU Load (busiest engine)".into(),
                        group: group.clone(),
                        unit: Unit::Percent,
                        value: load.overall(),
                    });
                    for (t, v) in &load.by_type {
                        out.push(Reading {
                            id: format!("{prefix}/engine/{t}"),
                            label: format!("GPU {} Load", engtype_label(t)),
                            group: group.clone(),
                            unit: Unit::Percent,
                            value: *v,
                        });
                    }
                }
            }
            for (counter, id, label) in [
                (&self.dedicated, "vram/dedicated", "Dedicated Memory Usage"),
                (&self.shared, "vram/shared", "Shared Memory Usage"),
            ] {
                let Some(c) = counter else { continue };
                for (name, bytes) in read_array(c, PDH_FMT_DOUBLE, &mut self.buf) {
                    let Some((prefix, group)) =
                        parse_adapter_instance(&name).and_then(|l| self.adapters.get(&l))
                    else {
                        continue;
                    };
                    out.push(Reading {
                        id: format!("{prefix}/{id}"),
                        label: label.into(),
                        group: group.clone(),
                        unit: Unit::Megabytes,
                        value: bytes / 1048576.0,
                    });
                }
            }
            Ok(())
        }
    }

    /// Battery charge from `GetSystemPowerStatus` (only useful on systems with a battery).
    #[derive(Default)]
    pub struct BatteryBackend;

    impl SensorBackend for BatteryBackend {
        fn name(&self) -> &str {
            "Battery"
        }

        fn poll(&mut self, out: &mut Vec<Reading>) -> Result<(), String> {
            let Some(b) = crate::win::battery() else {
                return Ok(());
            };
            let mut push = |id: &str, label: &str, unit, value| {
                out.push(Reading {
                    id: id.into(),
                    label: label.into(),
                    group: "Battery".into(),
                    unit,
                    value,
                })
            };
            push(
                "battery/charge",
                "Charge Level",
                Unit::Percent,
                b.percent as f64,
            );
            push(
                "battery/ac",
                "On AC Power",
                Unit::None,
                if b.on_ac { 1.0 } else { 0.0 },
            );
            if let Some(s) = b.seconds_left {
                push(
                    "battery/minutes_left",
                    "Estimated Minutes Left",
                    Unit::None,
                    s as f64 / 60.0,
                );
            }
            Ok(())
        }
    }

    /// Physical memory load from `GlobalMemoryStatusEx`.
    #[derive(Default)]
    pub struct MemoryBackend;

    impl SensorBackend for MemoryBackend {
        fn name(&self) -> &str {
            "Memory"
        }

        fn poll(&mut self, out: &mut Vec<Reading>) -> Result<(), String> {
            let m = crate::win::memory_status().ok_or("GlobalMemoryStatusEx failed")?;
            let mb = |b: u64| b as f64 / (1024.0 * 1024.0);
            let mut push = |id: &str, label: &str, unit, value| {
                out.push(Reading {
                    id: id.into(),
                    label: label.into(),
                    group: "Memory".into(),
                    unit,
                    value,
                })
            };
            push(
                "mem/load",
                "Physical Memory Load",
                Unit::Percent,
                m.load_percent as f64,
            );
            push(
                "mem/used",
                "Physical Memory Used",
                Unit::Megabytes,
                mb(m.total_bytes - m.available_bytes),
            );
            push(
                "mem/avail",
                "Physical Memory Available",
                Unit::Megabytes,
                mb(m.available_bytes),
            );
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instances() {
        assert_eq!(
            parse_instance("0,13"),
            Some(LogicalId {
                group: 0,
                number: 13
            })
        );
        assert_eq!(
            parse_instance("1,0"),
            Some(LogicalId {
                group: 1,
                number: 0
            })
        );
        assert_eq!(parse_instance("0,_Total"), None);
        assert_eq!(parse_instance("_Total"), None);
    }

    #[test]
    fn energy_labels() {
        assert_eq!(energy_label("RAPL_Package0_PKG"), "CPU Package Power");
        assert_eq!(energy_label("RAPL_Package0_DRAM"), "DRAM Power");
        assert_eq!(energy_label("Foo"), "Foo");
    }
}
