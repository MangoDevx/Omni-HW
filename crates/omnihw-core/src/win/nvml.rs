//! NVIDIA NVML through the `nvml-wrapper` crate, loaded at runtime from `nvml.dll` (ships
//! with the driver). An absent library or unsupported queries simply disable the related data.

use crate::gpu::{GpuAdapter, NvmlDetails};
use crate::sensors::{Reading, SensorBackend, Unit};
use nvml_wrapper::Nvml;
use nvml_wrapper::bitmasks::device::ThrottleReasons;
use nvml_wrapper::enum_wrappers::device::{Clock, PerformanceState, TemperatureSensor};
use std::path::PathBuf;
use windows::Win32::System::SystemInformation::GetSystemDirectoryW;

/// Where older drivers installed NVML.
const LEGACY_PATH: &str = r"C:\Program Files\NVIDIA Corporation\NVSMI\nvml.dll";

/// `C:\Windows\System32`.
fn system_dir() -> Option<PathBuf> {
    let mut buf = [0u16; 260];
    // SAFETY: valid buffer; the call writes at most its length.
    let n = unsafe { GetSystemDirectoryW(Some(&mut buf)) } as usize;
    (n > 0 && n < buf.len()).then(|| PathBuf::from(String::from_utf16_lossy(&buf[..n])))
}

/// NVML from System32, else its legacy folder. Always by absolute path, so a planted
/// `nvml.dll` elsewhere on the search path is never loaded.
fn load() -> Option<Nvml> {
    let candidates = system_dir()
        .map(|d| d.join("nvml.dll"))
        .into_iter()
        .chain([PathBuf::from(LEGACY_PATH)]);
    for path in candidates {
        if let Ok(nvml) = Nvml::builder().lib_path(path.as_os_str()).init() {
            return Some(nvml);
        }
    }
    None
}

/// (pci device id = device<<16|vendor, subsystem id) per NVML device, for matching to the
/// the DXGI adapters.
fn pci_ids(nvml: &Nvml) -> Vec<(u32, u32)> {
    let count = nvml.device_count().unwrap_or(0);
    (0..count)
        .map(|i| {
            nvml.device_by_index(i)
                .and_then(|d| d.pci_info())
                .map(|p| (p.pci_device_id, p.pci_sub_system_id.unwrap_or(0)))
                .unwrap_or_default()
        })
        .collect()
}

fn details(nvml: &Nvml, i: u32) -> Option<NvmlDetails> {
    let d = nvml.device_by_index(i).ok()?;
    let pair = |a: Result<u32, _>, b: Result<u32, _>| Some((a.ok()?, b.ok()?));
    Some(NvmlDetails {
        index: i,
        name: d.name().unwrap_or_default(),
        driver_version: nvml.sys_driver_version().unwrap_or_default(),
        vbios: d.vbios_version().unwrap_or_default(),
        bus_id: d.pci_info().map(|p| p.bus_id).unwrap_or_default(),
        memory_total: d.memory_info().map_or(0, |m| m.total),
        memory_bus_width: d.memory_bus_width().ok(),
        cuda_cores: d.num_cores().ok(),
        max_core_clock_mhz: d.max_clock_info(Clock::Graphics).ok(),
        max_mem_clock_mhz: d.max_clock_info(Clock::Memory).ok(),
        power_limit_w: d.enforced_power_limit().ok().map(|mw| mw as f64 / 1000.0),
        pcie_current: pair(d.current_pcie_link_gen(), d.current_pcie_link_width()),
        pcie_max: pair(d.max_pcie_link_gen(), d.max_pcie_link_width()),
        bar1_total: d.bar1_memory_info().ok().map(|m| m.total),
    })
}

/// Fill `nvml` details on matching DXGI adapters.
pub fn attach_details(adapters: &mut [GpuAdapter]) {
    let Some(nvml) = load() else { return };
    let matches = crate::gpu::match_nvml(adapters, &pci_ids(&nvml));
    for (a, m) in adapters.iter_mut().zip(matches) {
        if let Some(i) = m {
            a.nvml = details(&nvml, i as u32);
        }
    }
}

/// The P-state as its number (P0 = 0 ... P15 = 15).
fn pstate_number(p: PerformanceState) -> Option<f64> {
    use PerformanceState::*;
    let n = match p {
        Zero => 0,
        One => 1,
        Two => 2,
        Three => 3,
        Four => 4,
        Five => 5,
        Six => 6,
        Seven => 7,
        Eight => 8,
        Nine => 9,
        Ten => 10,
        Eleven => 11,
        Twelve => 12,
        Thirteen => 13,
        Fourteen => 14,
        Fifteen => 15,
        Unknown => return None,
    };
    Some(f64::from(n))
}

/// Live NVIDIA sensors: temperature, clocks, power, fan, load, VRAM.
pub struct NvmlBackend {
    nvml: Nvml,
    /// (NVML index, sensor id prefix, group name)
    targets: Vec<(u32, String, String)>,
}

impl NvmlBackend {
    pub fn new(adapters: &[GpuAdapter]) -> Option<Self> {
        let targets: Vec<_> = adapters
            .iter()
            .filter_map(|a| {
                Some((
                    a.nvml.as_ref()?.index,
                    a.sensor_prefix(),
                    format!("GPU: {}", a.name),
                ))
            })
            .collect();
        if targets.is_empty() {
            return None;
        }
        let nvml = load()?;
        let count = nvml.device_count().ok()?;
        let targets = targets.into_iter().filter(|(i, _, _)| *i < count).collect();
        Some(NvmlBackend { nvml, targets })
    }
}

impl SensorBackend for NvmlBackend {
    fn name(&self) -> &str {
        "NVML"
    }

    fn poll(&mut self, out: &mut Vec<Reading>) -> Result<(), String> {
        for (i, prefix, group) in &self.targets {
            let Ok(d) = self.nvml.device_by_index(*i) else {
                continue;
            };
            let mut push = |id: &str, label: &str, unit: Unit, value: Option<f64>| {
                if let Some(value) = value {
                    out.push(Reading {
                        id: format!("{prefix}/{id}"),
                        label: label.into(),
                        group: group.clone(),
                        unit,
                        value,
                    });
                }
            };
            let clock = |c| d.clock_info(c).ok().map(f64::from);
            push(
                "temp",
                "GPU Temperature",
                Unit::Celsius,
                d.temperature(TemperatureSensor::Gpu).ok().map(f64::from),
            );
            push(
                "clock/core",
                "GPU Core Clock",
                Unit::Mhz,
                clock(Clock::Graphics),
            );
            push(
                "clock/mem",
                "GPU Memory Clock",
                Unit::Mhz,
                clock(Clock::Memory),
            );
            push(
                "clock/video",
                "GPU Video Clock",
                Unit::Mhz,
                clock(Clock::Video),
            );
            let power = d.power_usage().ok().map(|mw| mw as f64 / 1000.0);
            push("power", "GPU Power", Unit::Watt, power);
            if let (Some(p), Some(limit)) =
                (power, d.enforced_power_limit().ok().filter(|&l| l > 0))
            {
                push(
                    "power_pct",
                    "GPU Power (% of limit)",
                    Unit::Percent,
                    Some(p * 100_000.0 / limit as f64),
                );
            }
            push(
                "fan",
                "GPU Fan",
                Unit::Percent,
                d.fan_speed(0).ok().map(f64::from),
            );
            if let Ok(u) = d.utilization_rates() {
                push(
                    "nv/load",
                    "GPU Core Load",
                    Unit::Percent,
                    Some(u.gpu as f64),
                );
                push(
                    "nv/memload",
                    "GPU Memory Controller Load",
                    Unit::Percent,
                    Some(u.memory as f64),
                );
            }
            if let Ok(m) = d.memory_info() {
                push(
                    "vram/used",
                    "GPU Memory Used",
                    Unit::Megabytes,
                    Some(m.used as f64 / 1048576.0),
                );
            }
            push(
                "pstate",
                "Performance State (P)",
                Unit::None,
                d.performance_state().ok().and_then(pstate_number),
            );
            // Hardware clock limiters as 0/1, read by the Health Check's throttling rule.
            if let Ok(r) = d.current_throttle_reasons() {
                let flag = |f: ThrottleReasons| Some(if r.intersects(f) { 1.0 } else { 0.0 });
                push(
                    "nv/throttle/hw_slowdown",
                    "HW Slowdown (1 = active)",
                    Unit::None,
                    flag(ThrottleReasons::HW_SLOWDOWN),
                );
                push(
                    "nv/throttle/hw_thermal",
                    "HW Thermal Slowdown (1 = active)",
                    Unit::None,
                    flag(ThrottleReasons::HW_THERMAL_SLOWDOWN),
                );
                push(
                    "nv/throttle/power_brake",
                    "HW Power Brake (1 = active)",
                    Unit::None,
                    flag(ThrottleReasons::HW_POWER_BRAKE_SLOWDOWN),
                );
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pstates() {
        assert_eq!(pstate_number(PerformanceState::Zero), Some(0.0));
        assert_eq!(pstate_number(PerformanceState::Eight), Some(8.0));
        assert_eq!(pstate_number(PerformanceState::Unknown), None);
    }
}
