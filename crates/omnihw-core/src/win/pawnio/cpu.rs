//! Per-core CPU temperatures, core voltage and (AMD) per-core power from MSRs and SMN registers,
//! through the IntelMSR / AMDFamily17 PawnIO modules.
//!
//! Read-only: only the modules' `ioctl_read_msr` and `ioctl_read_smn` functions are called.

use super::{AMD_FAMILY_17, GlobalMutex, INTEL_MSR, Module, PCI_MUTEX, locked};
use crate::cpuid::{CpuInfo, Vendor};
use crate::lowlevel::*;
use crate::sensors::{Reading, SensorBackend, Unit};
use crate::topology::{LogicalId, Topology};
use std::time::Instant;
use windows::Win32::System::SystemInformation::GROUP_AFFINITY;
use windows::Win32::System::Threading::{GetCurrentThread, SetThreadGroupAffinity};

const TEMPS: &str = "CPU Temperatures";
const VOLTS: &str = "CPU Voltages";

struct CoreSlot {
    /// Index into `Topology::cores`, used in sensor ids.
    index: usize,
    /// First logical processor of the core; MSRs are read while pinned to it.
    cpu: LogicalId,
    label: String,
    /// Intel: this core's TjMax.
    tjmax: f64,
    /// AMD: last core energy counter reading.
    energy: Option<(u64, Instant)>,
}

enum Vendorish {
    Intel,
    Amd {
        family: u32,
        model: u32,
        tctl_offset: f64,
        joules_per_tick: f64,
        /// (CCD index, SMN address) of CCDs that report a temperature.
        ccds: Vec<(u32, u32)>,
        package_energy: Option<(u64, Instant)>,
        pci: Option<GlobalMutex>,
    },
}

/// CPU sensors that need ring-0 register access.
pub struct CpuMsrBackend {
    module: Module,
    vendor: Vendorish,
    cores: Vec<CoreSlot>,
}

/// Pin the calling thread to `cpu`, returning the previous affinity.
fn pin(cpu: LogicalId) -> Option<GROUP_AFFINITY> {
    let affinity = GROUP_AFFINITY {
        Mask: 1usize << cpu.number,
        Group: cpu.group,
        Reserved: [0; 3],
    };
    let mut previous = GROUP_AFFINITY::default();
    // SAFETY: GetCurrentThread returns a pseudo-handle for the calling thread; both structs
    // outlive the call. Windows moves the thread onto the processor before returning.
    unsafe { SetThreadGroupAffinity(GetCurrentThread(), &affinity, Some(&mut previous)) }
        .as_bool()
        .then_some(previous)
}

fn restore(previous: GROUP_AFFINITY) {
    // SAFETY: as in `pin`; `previous` is an affinity this thread had before.
    let _ = unsafe { SetThreadGroupAffinity(GetCurrentThread(), &previous, None) };
}

/// Visits each core while pinned to it, then puts the thread's affinity back.
fn on_each_core(cores: &mut [CoreSlot], mut f: impl FnMut(&mut CoreSlot)) {
    let mut original = None;
    for c in cores {
        let Some(prev) = pin(c.cpu) else { continue };
        original.get_or_insert(prev);
        f(c);
    }
    if let Some(prev) = original {
        restore(prev);
    }
}

impl CpuMsrBackend {
    pub fn new(cpu: &CpuInfo, topology: Option<&Topology>) -> Result<Self, String> {
        let topology = topology.ok_or("CPU topology unavailable")?;
        let mut cores: Vec<CoreSlot> = topology
            .cores
            .iter()
            .enumerate()
            .filter_map(|(i, c)| {
                Some(CoreSlot {
                    index: i,
                    cpu: *c.logical.first()?,
                    label: topology.core_label(i),
                    tjmax: 100.0,
                    energy: None,
                })
            })
            .collect();
        match cpu.vendor {
            Vendor::Intel => {
                let module = Module::load("IntelMSR", INTEL_MSR)?;
                on_each_core(&mut cores, |c| {
                    if let Some(t) = module
                        .call1("ioctl_read_msr", MSR_IA32_TEMPERATURE_TARGET.into())
                        .ok()
                        .and_then(intel_tjmax)
                    {
                        c.tjmax = t;
                    }
                });
                Ok(Self {
                    module,
                    vendor: Vendorish::Intel,
                    cores,
                })
            }
            Vendor::Amd if (0x17..=0x1A).contains(&cpu.family) => {
                let module = Module::load("AMDFamily17", AMD_FAMILY_17)?;
                let pci = GlobalMutex::open(PCI_MUTEX);
                let ccds = locked(pci.as_ref(), || {
                    (0..MAX_CCDS)
                        .filter_map(|i| {
                            let addr = amd_ccd_temp_addr(cpu.family, cpu.model, i)?;
                            let raw = module.call1("ioctl_read_smn", addr.into()).ok()?;
                            amd_ccd_temp(raw as u32).map(|_| (i, addr))
                        })
                        .collect()
                })
                .unwrap_or_default();
                let joules_per_tick = module
                    .call1("ioctl_read_msr", MSR_AMD_PWR_UNIT.into())
                    .map(amd_energy_unit)
                    .unwrap_or(0.0);
                Ok(Self {
                    module,
                    vendor: Vendorish::Amd {
                        family: cpu.family,
                        model: cpu.model,
                        tctl_offset: amd_tctl_offset(&cpu.brand),
                        joules_per_tick,
                        ccds,
                        package_energy: None,
                        pci,
                    },
                    cores,
                })
            }
            _ => Err("no PawnIO CPU module for this processor".into()),
        }
    }
}

fn reading(id: String, label: String, group: &str, unit: Unit, value: f64) -> Reading {
    Reading {
        id,
        label,
        group: group.into(),
        unit,
        value,
    }
}

/// Watts from two readings of an energy counter; updates `prev`.
fn power(prev: &mut Option<(u64, Instant)>, ticks: u64, joules_per_tick: f64) -> Option<f64> {
    let now = Instant::now();
    let last = prev.replace((ticks, now));
    let (t0, at) = last?;
    let secs = now.duration_since(at).as_secs_f64();
    (secs > 0.05).then(|| energy_delta(t0, ticks) as f64 * joules_per_tick / secs)
}

impl SensorBackend for CpuMsrBackend {
    fn name(&self) -> &str {
        "PawnIO CPU"
    }

    fn poll(&mut self, out: &mut Vec<Reading>) -> Result<(), String> {
        let m = &self.module;
        let msr = |reg: u32| m.call1("ioctl_read_msr", reg.into()).ok();
        match &mut self.vendor {
            Vendorish::Intel => {
                let mut temps = Vec::new();
                let mut vid: Option<f64> = None;
                let mut package = None;
                on_each_core(&mut self.cores, |c| {
                    if let Some(t) =
                        msr(MSR_IA32_THERM_STATUS).and_then(|s| intel_core_temp(s, c.tjmax))
                    {
                        temps.push((c.index, c.label.clone(), t));
                    }
                    if let Some(v) = msr(MSR_IA32_PERF_STATUS).and_then(intel_vid) {
                        vid = Some(vid.map_or(v, |m| m.max(v)));
                    }
                    if package.is_none() {
                        package = msr(MSR_IA32_PACKAGE_THERM_STATUS)
                            .map(|s| intel_package_temp(s, c.tjmax));
                    }
                });
                if let Some(t) = package {
                    out.push(reading(
                        "cpu/temp/package".into(),
                        "CPU Package".into(),
                        TEMPS,
                        Unit::Celsius,
                        t,
                    ));
                }
                push_core_temps(out, &temps);
                if let Some(v) = vid {
                    out.push(reading(
                        "cpu/volt/vid".into(),
                        "Core VID".into(),
                        VOLTS,
                        Unit::Volt,
                        v,
                    ));
                }
            }
            Vendorish::Amd {
                family,
                model,
                tctl_offset,
                joules_per_tick,
                ccds,
                package_energy,
                pci,
            } => {
                let smn = locked(pci.as_ref(), || {
                    let tctl = m.call1("ioctl_read_smn", SMN_THM_TCON_CUR_TMP.into()).ok();
                    let ccd: Vec<_> = ccds
                        .iter()
                        .filter_map(|&(i, addr)| {
                            let raw = m.call1("ioctl_read_smn", addr.into()).ok()?;
                            Some((i, amd_ccd_temp(raw as u32)?))
                        })
                        .collect();
                    (tctl, ccd)
                });
                if let Some((tctl, ccd)) = smn {
                    if let Some(raw) = tctl {
                        let tctl = amd_tctl(raw as u32);
                        if *tctl_offset > 0.0 {
                            out.push(reading(
                                "cpu/temp/tctl".into(),
                                "CPU (Tctl)".into(),
                                TEMPS,
                                Unit::Celsius,
                                tctl,
                            ));
                            out.push(reading(
                                "cpu/temp/package".into(),
                                "CPU (Tdie)".into(),
                                TEMPS,
                                Unit::Celsius,
                                tctl - *tctl_offset,
                            ));
                        } else {
                            out.push(reading(
                                "cpu/temp/package".into(),
                                "CPU (Tctl/Tdie)".into(),
                                TEMPS,
                                Unit::Celsius,
                                tctl,
                            ));
                        }
                    }
                    for (i, t) in ccd {
                        out.push(reading(
                            format!("cpu/temp/ccd{i}"),
                            format!("CCD{}", i + 1),
                            TEMPS,
                            Unit::Celsius,
                            t,
                        ));
                    }
                }

                let jpt = *joules_per_tick;
                if jpt > 0.0
                    && let Some(w) =
                        msr(MSR_AMD_PKG_ENERGY_STAT).and_then(|t| power(package_energy, t, jpt))
                {
                    out.push(reading(
                        "cpu/power/package".into(),
                        "CPU Package Power".into(),
                        "Power",
                        Unit::Watt,
                        w,
                    ));
                }
                let svi2 = amd_uses_svi2(*family, *model);
                let mut vid: Option<f64> = None;
                let mut core_power = Vec::new();
                on_each_core(&mut self.cores, |c| {
                    if jpt > 0.0
                        && let Some(w) =
                            msr(MSR_AMD_CORE_ENERGY_STAT).and_then(|t| power(&mut c.energy, t, jpt))
                    {
                        core_power.push((c.index, c.label.clone(), w));
                    }
                    if svi2
                        && let Some(v) = msr(MSR_AMD_HARDWARE_PSTATE_STATUS).and_then(amd_svi2_vid)
                    {
                        vid = Some(vid.map_or(v, |m| m.max(v)));
                    }
                });
                for (i, label, w) in core_power {
                    out.push(reading(
                        format!("cpu/power/core{i}"),
                        format!("{label} Power"),
                        "CPU Core Power",
                        Unit::Watt,
                        w,
                    ));
                }
                if let Some(v) = vid {
                    out.push(reading(
                        "cpu/volt/vid".into(),
                        "Core VID".into(),
                        VOLTS,
                        Unit::Volt,
                        v,
                    ));
                }
            }
        }
        Ok(())
    }
}

/// Per-core temperatures plus the hottest core.
fn push_core_temps(out: &mut Vec<Reading>, temps: &[(usize, String, f64)]) {
    for (i, label, t) in temps {
        out.push(reading(
            format!("cpu/temp/core{i}"),
            label.clone(),
            TEMPS,
            Unit::Celsius,
            *t,
        ));
    }
    if let Some(max) = temps.iter().map(|(_, _, t)| *t).reduce(f64::max) {
        out.push(reading(
            "cpu/temp/max".into(),
            "Core Max".into(),
            TEMPS,
            Unit::Celsius,
            max,
        ));
    }
}
