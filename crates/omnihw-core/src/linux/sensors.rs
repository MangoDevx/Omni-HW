//! Live sensors on Linux: `/proc/stat` and cpufreq (CPU), RAPL through powercap, hwmon
//! (temperatures, fans, voltages), `/proc/diskstats` and DRM sysfs (amdgpu, i915).
//!
//! Sensor ids follow the scheme of the original Windows backends (`cpu/load/total`,
//! `power/RAPL_Package0_PKG`, ...), so saved alert rules and recordings stay compatible.

use super::topology::{cpu_number, logical_id, online_cpus};
use super::{Root, key_value, parse_int};
use crate::gpu::GpuAdapter;
use crate::sensors::{CPU_TEMP, Reading, SensorBackend, Unit};
use crate::storage::Drive;
use crate::topology::Topology;
use std::collections::{HashMap, HashSet};
use std::time::Instant;

/// Friendly label for a RAPL zone id such as "RAPL_Package0_PKG".
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

fn reading(
    id: impl Into<String>,
    label: impl Into<String>,
    group: &str,
    unit: Unit,
    value: f64,
) -> Reading {
    Reading {
        id: id.into(),
        label: label.into(),
        group: group.into(),
        unit,
        value,
    }
}

/// Busy and total jiffies per `/proc/stat` CPU line ("cpu" = all, "cpu3" = CPU 3).
pub fn parse_proc_stat(text: &str) -> Vec<(String, u64, u64)> {
    text.lines()
        .filter(|l| l.starts_with("cpu"))
        .filter_map(|l| {
            let mut f = l.split_whitespace();
            let name = f.next()?.to_string();
            // user nice system idle iowait irq softirq steal (guest time is inside user).
            let v: Vec<u64> = f.take(8).filter_map(|x| x.parse().ok()).collect();
            if v.len() < 4 {
                return None;
            }
            let total: u64 = v.iter().sum();
            let idle = v[3] + v.get(4).copied().unwrap_or(0);
            Some((name, total - idle, total))
        })
        .collect()
}

/// Percent busy between two (busy, total) samples.
pub fn busy_pct(prev: (u64, u64), cur: (u64, u64)) -> Option<f64> {
    let dt = cur.1.checked_sub(prev.1)?;
    if dt == 0 {
        return None;
    }
    let db = cur.0.saturating_sub(prev.0);
    Some((db as f64 / dt as f64 * 100.0).clamp(0.0, 100.0))
}

/// Energy counter delta, allowing for one wrap at `max`.
pub fn energy_delta(prev: u64, cur: u64, max: u64) -> u64 {
    if cur >= prev {
        cur - prev
    } else {
        max.saturating_sub(prev).saturating_add(cur)
    }
}

struct RaplZone {
    path: String,
    id: String,
    label: String,
    max: u64,
    last: Option<(u64, Instant)>,
}

/// RAPL zones: "intel-rapl:0" (package-0) and its sub-zones ("core", "uncore", "dram"), plus
/// "psys". The counters are root-only on current kernels; unreadable zones are left out.
fn rapl_zones(root: &Root) -> Vec<RaplZone> {
    const PC: &str = "/sys/class/powercap";
    let mut out = Vec::new();
    for z in root.list(PC) {
        let Some(rest) = z.strip_prefix("intel-rapl:") else {
            continue;
        };
        let path = format!("{PC}/{z}");
        if root.read_u64(&format!("{path}/energy_uj")).is_none() {
            continue;
        }
        let name = root.read(&format!("{path}/name")).unwrap_or_default();
        let package = rest.split(':').next().unwrap_or("0");
        let domain = match name.as_str() {
            n if n.starts_with("package") => "PKG",
            "core" => "PP0",
            "uncore" => "PP1",
            "dram" => "DRAM",
            "psys" => "PSYS",
            _ => continue,
        };
        let instance = format!("RAPL_Package{package}_{domain}");
        out.push(RaplZone {
            label: energy_label(&instance),
            id: format!("power/{instance}"),
            max: root
                .read_u64(&format!("{path}/max_energy_range_uj"))
                .unwrap_or(u64::MAX),
            path,
            last: None,
        });
    }
    out
}

/// Per-core clocks (cpufreq), per-thread and total load (`/proc/stat`) and RAPL power.
pub struct CpuBackend {
    root: Root,
    cpus: Vec<u32>,
    /// Core index per CPU number, and core labels.
    core_of: HashMap<u32, usize>,
    core_labels: Vec<String>,
    prev: HashMap<String, (u64, u64)>,
    rapl: Vec<RaplZone>,
}

impl CpuBackend {
    pub fn new(root: Root, topology: Option<&Topology>) -> CpuBackend {
        let mut core_of = HashMap::new();
        let mut core_labels = Vec::new();
        if let Some(t) = topology {
            for (i, c) in t.cores.iter().enumerate() {
                for &l in &c.logical {
                    core_of.insert(cpu_number(l), i);
                }
                core_labels.push(t.core_label(i));
            }
        }
        let mut b = CpuBackend {
            cpus: online_cpus(&root),
            rapl: rapl_zones(&root),
            root,
            core_of,
            core_labels,
            prev: HashMap::new(),
        };
        // Load needs a first sample to diff against.
        b.sample_load(&mut Vec::new());
        b
    }

    fn sample_load(&mut self, out: &mut Vec<Reading>) {
        let Some(text) = self.root.read("/proc/stat") else {
            return;
        };
        for (name, busy, total) in parse_proc_stat(&text) {
            let prev = self.prev.insert(name.clone(), (busy, total));
            let Some(pct) = prev.and_then(|p| busy_pct(p, (busy, total))) else {
                continue;
            };
            if name == "cpu" {
                out.push(reading(
                    "cpu/load/total",
                    "Total CPU Usage",
                    "CPU Usage",
                    Unit::Percent,
                    pct,
                ));
                continue;
            }
            let Some(cpu) = name.strip_prefix("cpu").and_then(|n| n.parse::<u32>().ok()) else {
                continue;
            };
            let id = logical_id(cpu);
            let label = match self.core_of.get(&cpu) {
                Some(&core) => format!("{} / T{}", self.core_labels[core], id.number),
                None => format!("CPU {}", id.number),
            };
            out.push(reading(
                format!("cpu/load/{},{}", id.group, id.number),
                format!("{label} Usage"),
                "CPU Usage",
                Unit::Percent,
                pct,
            ));
        }
    }

    fn sample_clocks(&self, out: &mut Vec<Reading>) {
        let mut per_core = vec![f64::NAN; self.core_labels.len()];
        let mut all = Vec::new();
        for &cpu in &self.cpus {
            let Some(khz) = self.root.read_u64(&format!(
                "/sys/devices/system/cpu/cpu{cpu}/cpufreq/scaling_cur_freq"
            )) else {
                continue;
            };
            let mhz = khz as f64 / 1000.0;
            all.push(mhz);
            match self.core_of.get(&cpu) {
                // Threads of one core share its clock; keep the highest (NaN.max(x) is x).
                Some(&core) => per_core[core] = per_core[core].max(mhz),
                None => {
                    let id = logical_id(cpu);
                    out.push(reading(
                        format!("cpu/clock/{},{}", id.group, id.number),
                        format!("CPU {} Clock", id.number),
                        "CPU Clocks",
                        Unit::Mhz,
                        mhz,
                    ));
                }
            }
        }
        if !all.is_empty() {
            let avg = all.iter().sum::<f64>() / all.len() as f64;
            out.push(reading(
                "cpu/clock/avg",
                "Average Core Clock",
                "CPU Clocks",
                Unit::Mhz,
                avg,
            ));
        }
        for (i, mhz) in per_core.into_iter().enumerate() {
            if mhz.is_finite() {
                out.push(reading(
                    format!("cpu/clock/core{i}"),
                    format!("{} Clock", self.core_labels[i]),
                    "CPU Clocks",
                    Unit::Mhz,
                    mhz,
                ));
            }
        }
    }

    fn sample_rapl(&mut self, out: &mut Vec<Reading>) {
        let now = Instant::now();
        for z in &mut self.rapl {
            let Some(uj) = self.root.read_u64(&format!("{}/energy_uj", z.path)) else {
                continue;
            };
            if let Some((prev, at)) = z.last {
                let dt = now.duration_since(at).as_secs_f64();
                if dt > 0.0 {
                    let joules = energy_delta(prev, uj, z.max) as f64 / 1e6;
                    out.push(reading(
                        z.id.clone(),
                        z.label.clone(),
                        "Power",
                        Unit::Watt,
                        joules / dt,
                    ));
                }
            }
            z.last = Some((uj, now));
        }
    }
}

impl SensorBackend for CpuBackend {
    fn name(&self) -> &str {
        "CPU"
    }

    fn poll(&mut self, out: &mut Vec<Reading>) -> Result<(), String> {
        self.sample_clocks(out);
        self.sample_load(out);
        self.sample_rapl(out);
        Ok(())
    }
}

/// Physical memory load from `/proc/meminfo`.
pub struct MemoryBackend {
    pub root: Root,
}

impl SensorBackend for MemoryBackend {
    fn name(&self) -> &str {
        "Memory"
    }

    fn poll(&mut self, out: &mut Vec<Reading>) -> Result<(), String> {
        let m = super::os::memory_status(&self.root).ok_or("/proc/meminfo unreadable")?;
        let mb = |b: u64| b as f64 / (1024.0 * 1024.0);
        out.push(reading(
            "mem/load",
            "Physical Memory Load",
            "Memory",
            Unit::Percent,
            m.load_percent as f64,
        ));
        out.push(reading(
            "mem/used",
            "Physical Memory Used",
            "Memory",
            Unit::Megabytes,
            mb(m.total_bytes.saturating_sub(m.available_bytes)),
        ));
        out.push(reading(
            "mem/avail",
            "Physical Memory Available",
            "Memory",
            Unit::Megabytes,
            mb(m.available_bytes),
        ));
        Ok(())
    }
}

/// Battery charge from `/sys/class/power_supply`.
pub struct BatteryBackend {
    pub root: Root,
}

impl SensorBackend for BatteryBackend {
    fn name(&self) -> &str {
        "Battery"
    }

    fn poll(&mut self, out: &mut Vec<Reading>) -> Result<(), String> {
        let Some(b) = super::power_supply::battery(&self.root) else {
            return Ok(());
        };
        out.push(reading(
            "battery/charge",
            "Charge Level",
            "Battery",
            Unit::Percent,
            b.percent as f64,
        ));
        out.push(reading(
            "battery/ac",
            "On AC Power",
            "Battery",
            Unit::None,
            if b.on_ac { 1.0 } else { 0.0 },
        ));
        if let Some(s) = b.seconds_left {
            out.push(reading(
                "battery/minutes_left",
                "Estimated Minutes Left",
                "Battery",
                Unit::None,
                s as f64 / 60.0,
            ));
        }
        Ok(())
    }
}

/// hwmon chips read by other backends (drives, GPUs, batteries) or not useful.
fn hwmon_skipped(chip: &str) -> bool {
    matches!(
        chip,
        "nvme" | "drivetemp" | "amdgpu" | "radeon" | "nouveau" | "i915" | "xe" | "nvidia"
    ) || chip.starts_with("BAT")
        || chip.starts_with("ADP")
        || chip == "AC"
}

/// Group for a chip's readings of `kind` ("Temperatures", "Fans", "Voltages", "Power").
fn hwmon_group(chip: &str, kind: &str) -> String {
    match chip {
        "coretemp" | "k10temp" | "zenpower" => format!("CPU {kind}"),
        "spd5118" | "jc42" | "ee1004" => format!("Memory {kind}"),
        c if c.starts_with("iwlwifi") || c.starts_with("mt7") || c.starts_with("ath") => {
            format!("Network {kind}")
        }
        _ => format!("Motherboard {kind}"),
    }
}

/// One hwmon input: (file stem like "temp1", unit, raw value scale to the unit).
fn hwmon_inputs(root: &Root, dir: &str) -> Vec<(String, Unit, f64)> {
    let mut out: Vec<(String, Unit, f64)> = Vec::new();
    for f in root.list(dir) {
        let (stem, unit, scale) = if let Some(s) = f.strip_suffix("_input") {
            match s.trim_end_matches(|c: char| c.is_ascii_digit()) {
                "temp" => (s, Unit::Celsius, 1000.0),
                "fan" => (s, Unit::Rpm, 1.0),
                "in" => (s, Unit::Volt, 1000.0),
                "power" => (s, Unit::Watt, 1e6),
                _ => continue,
            }
        } else if let Some(s) = f.strip_suffix("_average") {
            match s.trim_end_matches(|c: char| c.is_ascii_digit()) {
                "power" => (s, Unit::Watt, 1e6),
                _ => continue,
            }
        } else {
            continue;
        };
        // A power sensor can expose both; keep one.
        if out.iter().any(|o| o.0 == stem) {
            continue;
        }
        out.push((stem.to_string(), unit, scale));
    }
    out
}

fn plausible(unit: Unit, v: f64) -> bool {
    match unit {
        Unit::Celsius => (-40.0..=150.0).contains(&v),
        Unit::Rpm => (0.0..30_000.0).contains(&v),
        Unit::Volt => (0.0..30.0).contains(&v),
        Unit::Watt => (0.0..5000.0).contains(&v),
        _ => true,
    }
}

fn kind_name(unit: Unit) -> &'static str {
    match unit {
        Unit::Celsius => "Temperatures",
        Unit::Rpm => "Fans",
        Unit::Volt => "Voltages",
        _ => "Power",
    }
}

/// Read one hwmon value, preferring `<stem>_input` then `<stem>_average`.
fn hwmon_value(root: &Root, dir: &str, stem: &str, scale: f64) -> Option<f64> {
    let raw = root
        .read_i64(&format!("{dir}/{stem}_input"))
        .or_else(|| root.read_i64(&format!("{dir}/{stem}_average")))?;
    Some(raw as f64 / scale)
}

struct Chip {
    dir: String,
    name: String,
    /// Id prefix: "hwmon/<name>" plus an index when several chips share a name.
    prefix: String,
    /// Package number for coretemp ("coretemp.1" -> 1).
    package: u32,
    inputs: Vec<(String, Unit, f64)>,
}

/// Motherboard, CPU and other hwmon sensors (`/sys/class/hwmon`).
pub struct HwmonBackend {
    root: Root,
    chips: Vec<Chip>,
    /// (package, coretemp core id) -> topology core index, for "Core N" labels.
    core_index: HashMap<(u32, i64), usize>,
    core_labels: Vec<String>,
    /// Fans that have reported a speed; headers stuck at 0 stay hidden.
    fans_seen: HashSet<String>,
}

impl HwmonBackend {
    pub fn new(root: Root, topology: Option<&Topology>) -> HwmonBackend {
        const HW: &str = "/sys/class/hwmon";
        let mut chips: Vec<Chip> = Vec::new();
        let mut seen: HashMap<String, usize> = HashMap::new();
        for h in root.list(HW) {
            let dir = format!("{HW}/{h}");
            let Some(name) = root.read_nonempty(&format!("{dir}/name")) else {
                continue;
            };
            if hwmon_skipped(&name) {
                continue;
            }
            let n = seen.entry(name.clone()).or_default();
            let prefix = if *n == 0 {
                format!("hwmon/{name}")
            } else {
                format!("hwmon/{name}{n}")
            };
            *n += 1;
            // coretemp's device is the platform device "coretemp.N", N = package.
            let package = root
                .link_name(&format!("{dir}/device"))
                .and_then(|d| d.strip_prefix("coretemp.")?.parse().ok())
                .unwrap_or(0);
            chips.push(Chip {
                inputs: hwmon_inputs(&root, &dir),
                dir,
                name,
                prefix,
                package,
            });
        }
        let mut core_index = HashMap::new();
        let mut core_labels = Vec::new();
        if let Some(t) = topology {
            for (i, c) in t.cores.iter().enumerate() {
                core_labels.push(t.core_label(i));
                let Some(&first) = c.logical.first() else {
                    continue;
                };
                let base = format!("/sys/devices/system/cpu/cpu{}/topology", cpu_number(first));
                if let (Some(pkg), Some(core)) = (
                    root.read_u64(&format!("{base}/physical_package_id")),
                    root.read_i64(&format!("{base}/core_id")),
                ) {
                    core_index.insert((pkg as u32, core), i);
                }
            }
        }
        HwmonBackend {
            root,
            chips,
            core_index,
            core_labels,
            fans_seen: HashSet::new(),
        }
    }

    /// Whether any chip has inputs (if not, the backend isn't worth polling).
    pub fn is_empty(&self) -> bool {
        self.chips.iter().all(|c| c.inputs.is_empty())
    }

    fn cpu_temps(&self, chip: &Chip, out: &mut Vec<Reading>) {
        const TEMPS: &str = "CPU Temperatures";
        let mut cores = Vec::new();
        let mut tctl = None;
        let mut tdie = None;
        for (stem, unit, scale) in &chip.inputs {
            if *unit != Unit::Celsius {
                continue;
            }
            let Some(v) =
                hwmon_value(&self.root, &chip.dir, stem, *scale).filter(|&v| plausible(*unit, v))
            else {
                continue;
            };
            let label = self
                .root
                .read(&format!("{}/{stem}_label", chip.dir))
                .unwrap_or_default();
            if label.starts_with("Package id") {
                if chip.package == 0 {
                    out.push(reading(CPU_TEMP, "CPU Package", TEMPS, Unit::Celsius, v));
                } else {
                    out.push(reading(
                        format!("cpu/temp/package{}", chip.package),
                        format!("CPU {} Package", chip.package),
                        TEMPS,
                        Unit::Celsius,
                        v,
                    ));
                }
            } else if let Some(n) = label
                .strip_prefix("Core ")
                .and_then(|n| n.trim().parse::<i64>().ok())
            {
                match self.core_index.get(&(chip.package, n)) {
                    Some(&i) => cores.push((i, self.core_labels[i].clone(), v)),
                    None => out.push(reading(
                        format!("{}/{stem}", chip.prefix),
                        label.clone(),
                        TEMPS,
                        Unit::Celsius,
                        v,
                    )),
                }
            } else if label == "Tctl" {
                tctl = Some(v);
            } else if label == "Tdie" {
                tdie = Some(v);
            } else if let Some(n) = label.strip_prefix("Tccd") {
                out.push(reading(
                    format!("cpu/temp/ccd{n}"),
                    format!("CCD{n}"),
                    TEMPS,
                    Unit::Celsius,
                    v,
                ));
            } else {
                let label = if label.is_empty() {
                    format!("{} {stem}", chip.name)
                } else {
                    label
                };
                out.push(reading(
                    format!("{}/{stem}", chip.prefix),
                    label,
                    TEMPS,
                    Unit::Celsius,
                    v,
                ));
            }
        }
        match (tctl, tdie) {
            (Some(c), Some(d)) => {
                out.push(reading(
                    "cpu/temp/tctl",
                    "CPU (Tctl)",
                    TEMPS,
                    Unit::Celsius,
                    c,
                ));
                out.push(reading(CPU_TEMP, "CPU (Tdie)", TEMPS, Unit::Celsius, d));
            }
            (Some(t), None) | (None, Some(t)) => out.push(reading(
                CPU_TEMP,
                "CPU (Tctl/Tdie)",
                TEMPS,
                Unit::Celsius,
                t,
            )),
            (None, None) => {}
        }
        cores.sort_by_key(|c| c.0);
        for (i, label, t) in &cores {
            out.push(reading(
                format!("cpu/temp/core{i}"),
                label.clone(),
                TEMPS,
                Unit::Celsius,
                *t,
            ));
        }
        if let Some(max) = cores.iter().map(|c| c.2).reduce(f64::max) {
            out.push(reading(
                "cpu/temp/max",
                "Core Max",
                TEMPS,
                Unit::Celsius,
                max,
            ));
        }
    }
}

impl SensorBackend for HwmonBackend {
    fn name(&self) -> &str {
        "hwmon"
    }

    fn poll(&mut self, out: &mut Vec<Reading>) -> Result<(), String> {
        for chip in &self.chips {
            if matches!(chip.name.as_str(), "coretemp" | "k10temp" | "zenpower") {
                self.cpu_temps(chip, out);
                continue;
            }
            for (stem, unit, scale) in &chip.inputs {
                let Some(v) = hwmon_value(&self.root, &chip.dir, stem, *scale)
                    .filter(|&v| plausible(*unit, v))
                else {
                    continue;
                };
                let id = format!("{}/{stem}", chip.prefix);
                if *unit == Unit::Rpm {
                    if v > 0.0 {
                        self.fans_seen.insert(id.clone());
                    } else if !self.fans_seen.contains(&id) {
                        continue;
                    }
                }
                let label = self
                    .root
                    .read_nonempty(&format!("{}/{stem}_label", chip.dir))
                    .unwrap_or_else(|| format!("{} {stem}", chip.name));
                out.push(reading(
                    id,
                    label,
                    &hwmon_group(&chip.name, kind_name(*unit)),
                    *unit,
                    v,
                ));
            }
        }
        Ok(())
    }
}

/// (sectors read, sectors written, ms doing I/O) per device from `/proc/diskstats`.
pub fn parse_diskstats(text: &str) -> HashMap<String, (u64, u64, u64)> {
    text.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            let n = |i: usize| f.get(i)?.parse::<u64>().ok();
            Some((f.get(2)?.to_string(), (n(5)?, n(9)?, n(12)?)))
        })
        .collect()
}

struct DriveSensor {
    /// sysfs name, e.g. "nvme0n1".
    name: String,
    prefix: String,
    group: String,
    /// hwmon directory with the drive's temperature (nvme or drivetemp).
    hwmon: Option<String>,
}

/// Per block device: (sectors read, sectors written, ms spent doing I/O) from `stat`.
type IoCounters = HashMap<String, (u64, u64, u64)>;

/// Drive temperatures (nvme / drivetemp hwmon) and read/write rates and activity.
pub struct StorageBackend {
    root: Root,
    drives: Vec<DriveSensor>,
    last: Option<(IoCounters, Instant)>,
}

impl StorageBackend {
    /// `drives` must be [`super::block::drives`]' output (same order as the sysfs names).
    pub fn new(root: Root, drives: &[Drive]) -> StorageBackend {
        let names = super::block::drive_names(&root);
        let drives = drives
            .iter()
            .filter_map(|d| {
                let name = names.get(d.index as usize)?.clone();
                let dev = format!("/sys/block/{name}/device");
                // NVMe: the controller has hwmonN directly; drivetemp: device/hwmon/hwmonN.
                let hwmon = root
                    .list(&dev)
                    .into_iter()
                    .find(|h| h.starts_with("hwmon"))
                    .map(|h| format!("{dev}/{h}"))
                    .or_else(|| {
                        root.list(&format!("{dev}/hwmon"))
                            .into_iter()
                            .next()
                            .map(|h| format!("{dev}/hwmon/{h}"))
                    });
                Some(DriveSensor {
                    name,
                    prefix: d.sensor_prefix(),
                    group: format!("Drive: {}", d.name()),
                    hwmon,
                })
            })
            .collect();
        let mut b = StorageBackend {
            root,
            drives,
            last: None,
        };
        b.last = b.stats().map(|s| (s, Instant::now()));
        b
    }

    fn stats(&self) -> Option<HashMap<String, (u64, u64, u64)>> {
        Some(parse_diskstats(&self.root.read("/proc/diskstats")?))
    }
}

impl SensorBackend for StorageBackend {
    fn name(&self) -> &str {
        "Storage"
    }

    fn poll(&mut self, out: &mut Vec<Reading>) -> Result<(), String> {
        for d in &self.drives {
            let t = d
                .hwmon
                .as_ref()
                .and_then(|h| self.root.read_i64(&format!("{h}/temp1_input")))
                .map(|m| m as f64 / 1000.0)
                .filter(|&t| t > 0.0 && t < 150.0);
            if let Some(t) = t {
                out.push(reading(
                    format!("{}/temp", d.prefix),
                    "Drive Temperature",
                    &d.group,
                    Unit::Celsius,
                    t,
                ));
            }
        }
        let now = Instant::now();
        let Some(cur) = self.stats() else {
            return Ok(());
        };
        if let Some((prev, at)) = &self.last {
            let dt = now.duration_since(*at).as_secs_f64();
            if dt > 0.0 {
                for d in &self.drives {
                    let (Some(c), Some(p)) = (cur.get(&d.name), prev.get(&d.name)) else {
                        continue;
                    };
                    let rate =
                        |a: u64, b: u64| a.saturating_sub(b) as f64 * 512.0 / dt / 1_048_576.0;
                    let busy =
                        (c.2.saturating_sub(p.2) as f64 / (dt * 1000.0) * 100.0).clamp(0.0, 100.0);
                    out.push(reading(
                        format!("{}/activity", d.prefix),
                        "Drive Activity",
                        &d.group,
                        Unit::Percent,
                        busy,
                    ));
                    out.push(reading(
                        format!("{}/read", d.prefix),
                        "Read Rate",
                        &d.group,
                        Unit::MegabytesPerSec,
                        rate(c.0, p.0),
                    ));
                    out.push(reading(
                        format!("{}/write", d.prefix),
                        "Write Rate",
                        &d.group,
                        Unit::MegabytesPerSec,
                        rate(c.1, p.1),
                    ));
                }
            }
        }
        self.last = Some((cur, now));
        Ok(())
    }
}

struct GpuSensor {
    /// `/sys/class/drm/cardN/device`
    dev: String,
    hwmon: Option<String>,
    prefix: String,
    group: String,
    /// i915's actual-frequency file, if present.
    i915_freq: Option<String>,
}

/// Load, VRAM, temperatures, power, fan and clocks from DRM sysfs (amdgpu fully; i915, xe and
/// nouveau partly). NVIDIA's proprietary driver exposes none of this; NVML covers it.
pub struct DrmGpuBackend {
    root: Root,
    gpus: Vec<GpuSensor>,
}

impl DrmGpuBackend {
    pub fn new(root: Root, adapters: &[GpuAdapter]) -> DrmGpuBackend {
        let mut gpus = Vec::new();
        for card in super::drm::cards(&root) {
            let dev = format!("/sys/class/drm/{card}/device");
            let Some(slot) = root
                .read(&format!("{dev}/uevent"))
                .and_then(|u| key_value(&u, "PCI_SLOT_NAME"))
            else {
                continue;
            };
            let Some(luid) = super::drm::slot_id(&slot) else {
                continue;
            };
            let Some(a) = adapters.iter().find(|a| a.luid == luid) else {
                continue;
            };
            if gpus
                .iter()
                .any(|g: &GpuSensor| g.prefix == a.sensor_prefix())
            {
                continue;
            }
            let hwmon = root
                .list(&format!("{dev}/hwmon"))
                .into_iter()
                .next()
                .map(|h| format!("{dev}/hwmon/{h}"));
            let i915 = format!("/sys/class/drm/{card}/gt_act_freq_mhz");
            gpus.push(GpuSensor {
                i915_freq: root.exists(&i915).then_some(i915),
                dev,
                hwmon,
                prefix: a.sensor_prefix(),
                group: format!("GPU: {}", a.name),
            });
        }
        DrmGpuBackend { root, gpus }
    }

    pub fn is_empty(&self) -> bool {
        self.gpus.is_empty()
    }
}

impl SensorBackend for DrmGpuBackend {
    fn name(&self) -> &str {
        "DRM"
    }

    fn poll(&mut self, out: &mut Vec<Reading>) -> Result<(), String> {
        let r = &self.root;
        for g in &self.gpus {
            let mut push = |id: &str, label: &str, unit: Unit, v: Option<f64>| {
                if let Some(v) = v {
                    out.push(reading(
                        format!("{}/{id}", g.prefix),
                        label,
                        &g.group,
                        unit,
                        v,
                    ));
                }
            };
            let dev = |f: &str| r.read_u64(&format!("{}/{f}", g.dev));
            push(
                "load",
                "GPU Load",
                Unit::Percent,
                dev("gpu_busy_percent").map(|v| v as f64),
            );
            push(
                "vram/used",
                "GPU Memory Used",
                Unit::Megabytes,
                dev("mem_info_vram_used").map(|b| b as f64 / 1_048_576.0),
            );
            if let Some(h) = &g.hwmon {
                let hw =
                    |f: &str, scale: f64| r.read_i64(&format!("{h}/{f}")).map(|v| v as f64 / scale);
                let label = |f: &str| r.read(&format!("{h}/{f}")).unwrap_or_default();
                // amdgpu: temp1 = edge, temp2 = junction (hotspot), temp3 = memory.
                for (n, id, default) in [
                    (1, "temp", "GPU Temperature"),
                    (2, "temp/hotspot", "GPU Hot Spot"),
                    (3, "temp/mem", "GPU Memory Temperature"),
                ] {
                    let name = match label(&format!("temp{n}_label")).as_str() {
                        "junction" => "GPU Hot Spot",
                        "mem" => "GPU Memory Temperature",
                        _ => default,
                    };
                    push(
                        id,
                        name,
                        Unit::Celsius,
                        hw(&format!("temp{n}_input"), 1000.0)
                            .filter(|t| plausible(Unit::Celsius, *t)),
                    );
                }
                push(
                    "power",
                    "GPU Power",
                    Unit::Watt,
                    hw("power1_average", 1e6).or_else(|| hw("power1_input", 1e6)),
                );
                push("fan_rpm", "GPU Fan", Unit::Rpm, hw("fan1_input", 1.0));
                // amdgpu: freq1 = shader clock, freq2 = memory clock, in Hz.
                push(
                    "clock/core",
                    "GPU Core Clock",
                    Unit::Mhz,
                    hw("freq1_input", 1e6),
                );
                push(
                    "clock/mem",
                    "GPU Memory Clock",
                    Unit::Mhz,
                    hw("freq2_input", 1e6),
                );
            }
            if let Some(f) = &g.i915_freq {
                push(
                    "clock/core",
                    "GPU Core Clock",
                    Unit::Mhz,
                    r.read(f).and_then(|s| parse_int(&s)).map(|v| v as f64),
                );
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::linux::Fixture;

    #[test]
    fn energy_labels() {
        assert_eq!(energy_label("RAPL_Package0_PKG"), "CPU Package Power");
        assert_eq!(energy_label("RAPL_Package0_DRAM"), "DRAM Power");
        assert_eq!(energy_label("Foo"), "Foo");
    }

    #[test]
    fn proc_stat() {
        let a = parse_proc_stat(
            "cpu  100 0 100 800 0 0 0 0 0 0\ncpu0 50 0 50 400 0 0 0 0 0 0\nintr 1 2\n",
        );
        assert_eq!(
            a,
            vec![
                ("cpu".to_string(), 200, 1000),
                ("cpu0".to_string(), 100, 500)
            ]
        );
        assert_eq!(busy_pct((200, 1000), (300, 1200)), Some(50.0));
        assert_eq!(busy_pct((200, 1000), (200, 1000)), None);
    }

    #[test]
    fn energy_wrap() {
        assert_eq!(energy_delta(10, 30, 100), 20);
        assert_eq!(energy_delta(90, 5, 100), 15);
    }

    #[test]
    fn diskstats() {
        let s = parse_diskstats(" 259       0 nvme0n1 1000 0 2048 10 500 0 4096 20 0 300 30\n");
        assert_eq!(s["nvme0n1"], (2048, 4096, 300));
    }

    #[test]
    fn hwmon_chips() {
        let f = Fixture::new();
        f.file("sys/class/hwmon/hwmon0/name", "k10temp")
            .file("sys/class/hwmon/hwmon0/temp1_input", "65250")
            .file("sys/class/hwmon/hwmon0/temp1_label", "Tctl")
            .file("sys/class/hwmon/hwmon0/temp3_input", "60000")
            .file("sys/class/hwmon/hwmon0/temp3_label", "Tccd1")
            .file("sys/class/hwmon/hwmon1/name", "nct6799")
            .file("sys/class/hwmon/hwmon1/fan1_input", "1200")
            .file("sys/class/hwmon/hwmon1/fan2_input", "0")
            .file("sys/class/hwmon/hwmon1/in0_input", "1032")
            .file("sys/class/hwmon/hwmon1/in0_label", "Vcore")
            .file("sys/class/hwmon/hwmon2/name", "nvme")
            .file("sys/class/hwmon/hwmon2/temp1_input", "40000");
        let mut b = HwmonBackend::new(f.root(), None);
        let mut out = Vec::new();
        b.poll(&mut out).unwrap();
        let get = |id: &str| out.iter().find(|r| r.id == id).map(|r| r.value);
        assert_eq!(get(CPU_TEMP), Some(65.25));
        assert_eq!(get("cpu/temp/ccd1"), Some(60.0));
        assert_eq!(get("hwmon/nct6799/fan1"), Some(1200.0));
        assert_eq!(get("hwmon/nct6799/fan2"), None);
        assert_eq!(get("hwmon/nct6799/in0"), Some(1.032));
        assert!(!out.iter().any(|r| r.id.contains("nvme")));
        let vcore = out.iter().find(|r| r.id == "hwmon/nct6799/in0").unwrap();
        assert_eq!(
            (vcore.label.as_str(), vcore.group.as_str()),
            ("Vcore", "Motherboard Voltages")
        );
    }

    #[test]
    fn coretemp_cores() {
        let f = Fixture::new();
        f.file("sys/devices/system/cpu/online", "0-1")
            .file(
                "sys/devices/system/cpu/cpu0/topology/physical_package_id",
                "0",
            )
            .file("sys/devices/system/cpu/cpu0/topology/core_id", "0")
            .file(
                "sys/devices/system/cpu/cpu1/topology/physical_package_id",
                "0",
            )
            .file("sys/devices/system/cpu/cpu1/topology/core_id", "4")
            .file("sys/class/hwmon/hwmon3/name", "coretemp")
            .file("sys/class/hwmon/hwmon3/temp1_input", "55000")
            .file("sys/class/hwmon/hwmon3/temp1_label", "Package id 0")
            .file("sys/class/hwmon/hwmon3/temp2_input", "50000")
            .file("sys/class/hwmon/hwmon3/temp2_label", "Core 0")
            .file("sys/class/hwmon/hwmon3/temp3_input", "53000")
            .file("sys/class/hwmon/hwmon3/temp3_label", "Core 4");
        let t = crate::linux::topology::topology(&f.root()).unwrap();
        let mut b = HwmonBackend::new(f.root(), Some(&t));
        let mut out = Vec::new();
        b.poll(&mut out).unwrap();
        let get = |id: &str| out.iter().find(|r| r.id == id).map(|r| r.value);
        assert_eq!(get(CPU_TEMP), Some(55.0));
        assert_eq!(get("cpu/temp/core0"), Some(50.0));
        assert_eq!(get("cpu/temp/core1"), Some(53.0));
        assert_eq!(get("cpu/temp/max"), Some(53.0));
    }

    #[test]
    fn cpu_clocks() {
        let f = Fixture::new();
        f.file("sys/devices/system/cpu/online", "0-1")
            .file("sys/devices/system/cpu/cpu0/topology/core_id", "0")
            .file("sys/devices/system/cpu/cpu1/topology/core_id", "0")
            .file(
                "sys/devices/system/cpu/cpu0/cpufreq/scaling_cur_freq",
                "4000000",
            )
            .file(
                "sys/devices/system/cpu/cpu1/cpufreq/scaling_cur_freq",
                "4200000",
            )
            .file("proc/stat", "cpu  1 0 1 8 0 0 0 0\ncpu0 1 0 1 8 0 0 0 0\n");
        let t = crate::linux::topology::topology(&f.root()).unwrap();
        let mut b = CpuBackend::new(f.root(), Some(&t));
        f.file(
            "proc/stat",
            "cpu  6 0 1 13 0 0 0 0\ncpu0 6 0 1 13 0 0 0 0\n",
        );
        let mut out = Vec::new();
        b.poll(&mut out).unwrap();
        let get = |id: &str| out.iter().find(|r| r.id == id).map(|r| r.value);
        assert_eq!(get("cpu/clock/core0"), Some(4200.0));
        assert_eq!(get("cpu/clock/avg"), Some(4100.0));
        assert_eq!(get("cpu/load/total"), Some(50.0));
        assert_eq!(get("cpu/load/0,0"), Some(50.0));
    }

    #[test]
    fn amdgpu_sensors() {
        let f = Fixture::new();
        let dev = "sys/class/drm/card0/device";
        f.file(
            &format!("{dev}/uevent"),
            "DRIVER=amdgpu\nPCI_ID=1002:73BF\nPCI_SLOT_NAME=0000:03:00.0\n",
        )
        .file(&format!("{dev}/gpu_busy_percent"), "37")
        .file(&format!("{dev}/mem_info_vram_used"), "1073741824")
        .file(&format!("{dev}/hwmon/hwmon5/temp1_input"), "52000")
        .file(&format!("{dev}/hwmon/hwmon5/temp1_label"), "edge")
        .file(&format!("{dev}/hwmon/hwmon5/temp2_input"), "61000")
        .file(&format!("{dev}/hwmon/hwmon5/temp2_label"), "junction")
        .file(&format!("{dev}/hwmon/hwmon5/power1_average"), "120000000")
        .file(&format!("{dev}/hwmon/hwmon5/freq1_input"), "2100000000");
        let gpus = crate::linux::drm::gpu_adapters(&f.root());
        let mut b = DrmGpuBackend::new(f.root(), &gpus);
        let mut out = Vec::new();
        b.poll(&mut out).unwrap();
        let p = gpus[0].sensor_prefix();
        let get = |id: &str| {
            out.iter()
                .find(|r| r.id == format!("{p}/{id}"))
                .map(|r| r.value)
        };
        assert_eq!(get("load"), Some(37.0));
        assert_eq!(get("vram/used"), Some(1024.0));
        assert_eq!(get("temp"), Some(52.0));
        assert_eq!(get("temp/hotspot"), Some(61.0));
        assert_eq!(get("temp/mem"), None);
        assert_eq!(get("power"), Some(120.0));
        assert_eq!(get("clock/core"), Some(2100.0));
    }
}
