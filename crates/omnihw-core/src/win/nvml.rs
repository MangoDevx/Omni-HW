//! NVIDIA NVML, loaded at runtime from `nvml.dll` (ships with the driver).
//! Absent library or missing entry points simply disable the related data.

use crate::gpu::{GpuAdapter, NvmlDetails};
use crate::sensors::{Reading, SensorBackend, Unit};
use std::ffi::c_void;
use std::ptr;
use windows::Win32::Foundation::{FreeLibrary, HMODULE};
use windows::Win32::System::LibraryLoader::{
    GetProcAddress, LOAD_LIBRARY_FLAGS, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW,
};
use windows::core::{s, w};

// NVML is a third-party driver DLL with no `windows` crate bindings, so its handle type,
// structs and entry-point signatures below stay hand-written (from nvml.h).
type Device = *mut c_void;
type Ret = i32;
const SUCCESS: Ret = 0;

const TEMPERATURE_GPU: u32 = 0;
const CLOCK_GRAPHICS: u32 = 0;
const CLOCK_MEM: u32 = 2;
const CLOCK_VIDEO: u32 = 3;

#[repr(C)]
#[derive(Default)]
struct Utilization {
    gpu: u32,
    memory: u32,
}

#[repr(C)]
#[derive(Default)]
struct Memory {
    total: u64,
    free: u64,
    used: u64,
}

#[repr(C)]
struct PciInfo {
    bus_id_legacy: [u8; 16],
    domain: u32,
    bus: u32,
    device: u32,
    pci_device_id: u32,
    pci_sub_system_id: u32,
    bus_id: [u8; 32],
}

type FnVoid = unsafe extern "C" fn() -> Ret;
type FnU32Out = unsafe extern "C" fn(*mut u32) -> Ret;
type FnHandle = unsafe extern "C" fn(u32, *mut Device) -> Ret;
type FnStr = unsafe extern "C" fn(Device, *mut u8, u32) -> Ret;
type FnSysStr = unsafe extern "C" fn(*mut u8, u32) -> Ret;
type FnDevU32 = unsafe extern "C" fn(Device, *mut u32) -> Ret;
type FnDevArgU32 = unsafe extern "C" fn(Device, u32, *mut u32) -> Ret;
type FnUtil = unsafe extern "C" fn(Device, *mut Utilization) -> Ret;
type FnMem = unsafe extern "C" fn(Device, *mut Memory) -> Ret;
type FnPci = unsafe extern "C" fn(Device, *mut PciInfo) -> Ret;

struct Fns {
    shutdown: FnVoid,
    name: Option<FnStr>,
    driver: Option<FnSysStr>,
    vbios: Option<FnStr>,
    temperature: Option<FnDevArgU32>,
    clock: Option<FnDevArgU32>,
    max_clock: Option<FnDevArgU32>,
    power: Option<FnDevU32>,
    power_limit: Option<FnDevU32>,
    fan: Option<FnDevU32>,
    utilization: Option<FnUtil>,
    memory: Option<FnMem>,
    pci: Option<FnPci>,
    pcie_gen: Option<FnDevU32>,
    pcie_width: Option<FnDevU32>,
    pcie_max_gen: Option<FnDevU32>,
    pcie_max_width: Option<FnDevU32>,
    cores: Option<FnDevU32>,
    bus_width: Option<FnDevU32>,
    pstate: Option<FnDevU32>,
    /// `nvmlDeviceGetBAR1MemoryInfo`; `nvmlBAR1Memory_t` has the same layout as `Memory`.
    bar1: Option<FnMem>,
}

pub struct Nvml {
    lib: HMODULE,
    f: Fns,
    devices: Vec<Device>,
}

// SAFETY: NVML is thread-safe; handles are plain identifiers.
unsafe impl Send for Nvml {}

fn c_str(buf: &[u8]) -> String {
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).trim().to_string()
}

impl Nvml {
    pub fn load() -> Option<Nvml> {
        // SAFETY: literal names; System32-only search avoids DLL planting, and the
        // legacy fallback is an absolute path.
        let lib = unsafe {
            LoadLibraryExW(w!("nvml.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32).or_else(|_| {
                LoadLibraryExW(
                    w!(r"C:\Program Files\NVIDIA Corporation\NVSMI\nvml.dll"),
                    None,
                    LOAD_LIBRARY_FLAGS(0),
                )
            })
        }
        .ok()?;
        macro_rules! opt {
            ($name:literal, $t:ty) => {
                // SAFETY: valid module and NUL-terminated name; NVML exports these
                // symbols with the declared C signatures.
                unsafe { GetProcAddress(lib, s!($name)) }.map(|p| unsafe {
                    std::mem::transmute::<unsafe extern "system" fn() -> isize, $t>(p)
                })
            };
        }
        let (Some(init), Some(shutdown), Some(count), Some(handle)) = (
            opt!("nvmlInit_v2", FnVoid),
            opt!("nvmlShutdown", FnVoid),
            opt!("nvmlDeviceGetCount_v2", FnU32Out),
            opt!("nvmlDeviceGetHandleByIndex_v2", FnHandle),
        ) else {
            // SAFETY: module loaded above.
            let _ = unsafe { FreeLibrary(lib) };
            return None;
        };
        // SAFETY: entry points resolved above.
        if unsafe { init() } != SUCCESS {
            let _ = unsafe { FreeLibrary(lib) };
            return None;
        }
        let f = Fns {
            shutdown,
            name: opt!("nvmlDeviceGetName", FnStr),
            driver: opt!("nvmlSystemGetDriverVersion", FnSysStr),
            vbios: opt!("nvmlDeviceGetVbiosVersion", FnStr),
            temperature: opt!("nvmlDeviceGetTemperature", FnDevArgU32),
            clock: opt!("nvmlDeviceGetClockInfo", FnDevArgU32),
            max_clock: opt!("nvmlDeviceGetMaxClockInfo", FnDevArgU32),
            power: opt!("nvmlDeviceGetPowerUsage", FnDevU32),
            power_limit: opt!("nvmlDeviceGetEnforcedPowerLimit", FnDevU32),
            fan: opt!("nvmlDeviceGetFanSpeed", FnDevU32),
            utilization: opt!("nvmlDeviceGetUtilizationRates", FnUtil),
            memory: opt!("nvmlDeviceGetMemoryInfo", FnMem),
            pci: opt!("nvmlDeviceGetPciInfo_v3", FnPci),
            pcie_gen: opt!("nvmlDeviceGetCurrPcieLinkGeneration", FnDevU32),
            pcie_width: opt!("nvmlDeviceGetCurrPcieLinkWidth", FnDevU32),
            pcie_max_gen: opt!("nvmlDeviceGetMaxPcieLinkGeneration", FnDevU32),
            pcie_max_width: opt!("nvmlDeviceGetMaxPcieLinkWidth", FnDevU32),
            cores: opt!("nvmlDeviceGetNumGpuCores", FnDevU32),
            bus_width: opt!("nvmlDeviceGetMemoryBusWidth", FnDevU32),
            pstate: opt!("nvmlDeviceGetPerformanceState", FnDevU32),
            bar1: opt!("nvmlDeviceGetBAR1MemoryInfo", FnMem),
        };
        let mut n = 0;
        // SAFETY: valid out pointer.
        let mut devices = Vec::new();
        if unsafe { count(&mut n) } == SUCCESS {
            for i in 0..n {
                let mut d = ptr::null_mut();
                // SAFETY: index < count.
                if unsafe { handle(i, &mut d) } == SUCCESS {
                    devices.push(d);
                }
            }
        }
        Some(Nvml { lib, f, devices })
    }

    pub fn device_count(&self) -> usize {
        self.devices.len()
    }

    fn u32_of(&self, f: Option<FnDevU32>, i: usize) -> Option<u32> {
        let mut v = 0;
        // SAFETY: function resolved from NVML; device handle valid while initialized.
        (unsafe { f?(self.devices[i], &mut v) } == SUCCESS).then_some(v)
    }

    fn arg_u32(&self, f: Option<FnDevArgU32>, i: usize, arg: u32) -> Option<u32> {
        let mut v = 0;
        // SAFETY: as above.
        (unsafe { f?(self.devices[i], arg, &mut v) } == SUCCESS).then_some(v)
    }

    fn string(&self, f: Option<FnStr>, i: usize) -> String {
        let mut buf = [0u8; 96];
        match f {
            // SAFETY: buffer length passed explicitly.
            Some(f)
                if unsafe { f(self.devices[i], buf.as_mut_ptr(), buf.len() as u32) } == SUCCESS =>
            {
                c_str(&buf)
            }
            _ => String::new(),
        }
    }

    fn pci_info(&self, i: usize) -> Option<PciInfo> {
        // SAFETY: zeroed POD out-struct.
        let mut p: PciInfo = unsafe { std::mem::zeroed() };
        (unsafe { self.f.pci?(self.devices[i], &mut p) } == SUCCESS).then_some(p)
    }

    /// (pci device id = device<<16|vendor, subsystem id) per device, for matching to DXGI.
    pub fn pci_ids(&self) -> Vec<(u32, u32)> {
        (0..self.devices.len())
            .map(|i| {
                self.pci_info(i)
                    .map(|p| (p.pci_device_id, p.pci_sub_system_id))
                    .unwrap_or_default()
            })
            .collect()
    }

    pub fn details(&self, i: usize) -> NvmlDetails {
        let mut driver = [0u8; 96];
        let driver_version = match self.f.driver {
            // SAFETY: buffer length passed explicitly.
            Some(f) if unsafe { f(driver.as_mut_ptr(), driver.len() as u32) } == SUCCESS => {
                c_str(&driver)
            }
            _ => String::new(),
        };
        let memory_total = self.memory(i).map(|m| m.total).unwrap_or(0);
        let pair = |a, b| Some((self.u32_of(a, i)?, self.u32_of(b, i)?));
        NvmlDetails {
            index: i as u32,
            name: self.string(self.f.name, i),
            driver_version,
            vbios: self.string(self.f.vbios, i),
            bus_id: self
                .pci_info(i)
                .map(|p| c_str(&p.bus_id))
                .unwrap_or_default(),
            memory_total,
            memory_bus_width: self.u32_of(self.f.bus_width, i),
            cuda_cores: self.u32_of(self.f.cores, i),
            max_core_clock_mhz: self.arg_u32(self.f.max_clock, i, CLOCK_GRAPHICS),
            max_mem_clock_mhz: self.arg_u32(self.f.max_clock, i, CLOCK_MEM),
            power_limit_w: self
                .u32_of(self.f.power_limit, i)
                .map(|mw| mw as f64 / 1000.0),
            pcie_current: pair(self.f.pcie_gen, self.f.pcie_width),
            pcie_max: pair(self.f.pcie_max_gen, self.f.pcie_max_width),
            bar1_total: self.bar1_total(i),
        }
    }

    fn memory(&self, i: usize) -> Option<Memory> {
        let mut m = Memory::default();
        // SAFETY: valid out pointer.
        (unsafe { self.f.memory?(self.devices[i], &mut m) } == SUCCESS).then_some(m)
    }

    fn bar1_total(&self, i: usize) -> Option<u64> {
        let mut m = Memory::default();
        // SAFETY: valid out pointer; same layout as nvmlBAR1Memory_t.
        (unsafe { self.f.bar1?(self.devices[i], &mut m) } == SUCCESS).then_some(m.total)
    }

    fn utilization(&self, i: usize) -> Option<Utilization> {
        let mut u = Utilization::default();
        // SAFETY: valid out pointer.
        (unsafe { self.f.utilization?(self.devices[i], &mut u) } == SUCCESS).then_some(u)
    }
}

impl Drop for Nvml {
    fn drop(&mut self) {
        // SAFETY: initialized in load(); library freed once.
        unsafe {
            (self.f.shutdown)();
            let _ = FreeLibrary(self.lib);
        }
    }
}

/// Fill `nvml` details on matching DXGI adapters.
pub fn attach_details(adapters: &mut [GpuAdapter]) {
    let Some(nvml) = Nvml::load() else { return };
    let matches = crate::gpu::match_nvml(adapters, &nvml.pci_ids());
    for (a, m) in adapters.iter_mut().zip(matches) {
        if let Some(i) = m {
            a.nvml = Some(nvml.details(i));
        }
    }
}

/// Live NVIDIA sensors: temperature, clocks, power, fan, load, VRAM.
pub struct NvmlBackend {
    nvml: Nvml,
    /// (NVML index, sensor id prefix, group name)
    targets: Vec<(usize, String, String)>,
}

impl NvmlBackend {
    pub fn new(adapters: &[GpuAdapter]) -> Option<Self> {
        let targets: Vec<_> = adapters
            .iter()
            .filter_map(|a| {
                Some((
                    a.nvml.as_ref()?.index as usize,
                    a.sensor_prefix(),
                    format!("GPU: {}", a.name),
                ))
            })
            .collect();
        if targets.is_empty() {
            return None;
        }
        let nvml = Nvml::load()?;
        let targets = targets
            .into_iter()
            .filter(|(i, _, _)| *i < nvml.device_count())
            .collect();
        Some(NvmlBackend { nvml, targets })
    }
}

impl SensorBackend for NvmlBackend {
    fn name(&self) -> &str {
        "NVML"
    }

    fn poll(&mut self, out: &mut Vec<Reading>) -> Result<(), String> {
        let n = &self.nvml;
        for (i, prefix, group) in &self.targets {
            let i = *i;
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
            push(
                "temp",
                "GPU Temperature",
                Unit::Celsius,
                n.arg_u32(n.f.temperature, i, TEMPERATURE_GPU)
                    .map(f64::from),
            );
            push(
                "clock/core",
                "GPU Core Clock",
                Unit::Mhz,
                n.arg_u32(n.f.clock, i, CLOCK_GRAPHICS).map(f64::from),
            );
            push(
                "clock/mem",
                "GPU Memory Clock",
                Unit::Mhz,
                n.arg_u32(n.f.clock, i, CLOCK_MEM).map(f64::from),
            );
            push(
                "clock/video",
                "GPU Video Clock",
                Unit::Mhz,
                n.arg_u32(n.f.clock, i, CLOCK_VIDEO).map(f64::from),
            );
            let power = n.u32_of(n.f.power, i).map(|mw| mw as f64 / 1000.0);
            push("power", "GPU Power", Unit::Watt, power);
            if let (Some(p), Some(limit)) = (power, n.u32_of(n.f.power_limit, i).filter(|&l| l > 0))
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
                n.u32_of(n.f.fan, i).map(f64::from),
            );
            if let Some(u) = n.utilization(i) {
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
            if let Some(m) = n.memory(i) {
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
                n.u32_of(n.f.pstate, i).map(f64::from),
            );
        }
        Ok(())
    }
}
