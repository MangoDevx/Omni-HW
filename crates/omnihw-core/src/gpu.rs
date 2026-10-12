//! GPU data model: adapters (DXGI), optional vendor details (NVML), a small
//! chip database, and parsing of the `GPU Engine` / `GPU Adapter Memory` PDH
//! instance names used for vendor-neutral live load and VRAM usage.

use std::collections::HashMap;

pub const VENDOR_NVIDIA: u32 = 0x10DE;
pub const VENDOR_AMD: u32 = 0x1002;
pub const VENDOR_INTEL: u32 = 0x8086;

/// One display adapter as reported by DXGI.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GpuAdapter {
    pub name: String,
    pub vendor_id: u32,
    pub device_id: u32,
    /// PCI subsystem id: low 16 bits = board vendor, high 16 bits = board id.
    pub subsys_id: u32,
    pub revision: u32,
    pub dedicated_vram: u64,
    pub shared_memory: u64,
    /// Adapter LUID, used to match PDH counter instances.
    pub luid: u64,
    /// User-mode driver version, e.g. "32.0.16.1742".
    pub driver_version: String,
    /// Vendor library details, if available.
    pub nvml: Option<NvmlDetails>,
    /// Displays connected to this adapter's own outputs.
    pub displays: Vec<Display>,
    /// Driver date from the display adapter's registry key, as (year, month, day).
    pub driver_date: Option<(i64, u32, u32)>,
}

/// Resizable BAR state: whether the CPU can address all of VRAM at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReBar {
    Enabled {
        bar_bytes: u64,
    },
    Disabled {
        bar_bytes: u64,
    },
    /// No source for this GPU (only NVIDIA via NVML is supported so far).
    Unknown,
}

impl ReBar {
    /// Without ReBAR the aperture is the legacy 256 MiB window; with it, it spans (nearly) all VRAM.
    /// Anything above 1 GiB means the BAR was resized.
    pub fn from_bar_size(bar_bytes: u64) -> ReBar {
        if bar_bytes > 1 << 30 {
            ReBar::Enabled { bar_bytes }
        } else {
            ReBar::Disabled { bar_bytes }
        }
    }

    /// "On", "Off", "Unknown".
    pub fn label(self) -> &'static str {
        match self {
            ReBar::Enabled { .. } => "On",
            ReBar::Disabled { .. } => "Off",
            ReBar::Unknown => "Unknown",
        }
    }
}

/// A desktop display driven by an adapter output.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Display {
    /// e.g. `\\.\DISPLAY1`
    pub name: String,
    pub width: u32,
    pub height: u32,
}

impl Display {
    /// "DISPLAY1 (2560x1440)"
    pub fn summary(&self) -> String {
        let short = self.name.rsplit(['\\', '.']).next().unwrap_or(&self.name);
        format!("{short} ({}x{})", self.width, self.height)
    }
}

impl GpuAdapter {
    pub fn vendor_name(&self) -> &'static str {
        pci_vendor_name(self.vendor_id).unwrap_or("Unknown")
    }

    /// Board partner (from the subsystem vendor), e.g. "ZOTAC".
    pub fn board_vendor(&self) -> Option<&'static str> {
        pci_vendor_name(self.subsys_id & 0xFFFF)
    }

    pub fn chip(&self) -> Option<GpuChip> {
        lookup_chip(self.vendor_id, self.device_id)
    }

    /// Integrated graphics (part of the CPU) rather than a separate card: Intel or AMD graphics
    /// that aren't a known discrete chip and report little dedicated memory.
    pub fn is_integrated(&self) -> bool {
        matches!(self.vendor_id, VENDOR_INTEL | VENDOR_AMD)
            && self.chip().is_none()
            && self.dedicated_vram <= 2 << 30
    }

    /// Prefix for this adapter's sensor ids: "gpu/<luid hex>".
    pub fn sensor_prefix(&self) -> String {
        sensor_prefix(self.luid)
    }

    /// Resizable BAR state, from NVML's BAR1 size on NVIDIA GPUs.
    pub fn rebar(&self) -> ReBar {
        match self.nvml.as_ref().and_then(|n| n.bar1_total) {
            Some(b) if b > 0 => ReBar::from_bar_size(b),
            _ => ReBar::Unknown,
        }
    }

    /// Whether the GPU generation supports Resizable BAR (RTX 30 and newer, RX 6000 and newer,
    /// Intel Arc). `None` when the chip isn't in the database.
    pub fn supports_rebar(&self) -> Option<bool> {
        let chip = self.chip()?;
        Some(matches!(
            chip.architecture,
            "Ampere"
                | "Ada Lovelace"
                | "Blackwell"
                | "RDNA 2"
                | "RDNA 3"
                | "RDNA 4"
                | "Xe-HPG"
                | "Xe2-HPG"
        ))
    }

    /// "10DE 2204 - 19DA 1613" style PCI id string.
    pub fn pci_ids(&self) -> String {
        format!(
            "{:04X} {:04X} - {:04X} {:04X}",
            self.vendor_id,
            self.device_id,
            self.subsys_id & 0xFFFF,
            self.subsys_id >> 16
        )
    }
}

pub fn sensor_prefix(luid: u64) -> String {
    format!("gpu/{luid:x}")
}

/// Static details from NVIDIA's NVML.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NvmlDetails {
    pub index: u32,
    pub name: String,
    pub driver_version: String,
    pub vbios: String,
    pub bus_id: String,
    pub memory_total: u64,
    pub memory_bus_width: Option<u32>,
    pub cuda_cores: Option<u32>,
    pub max_core_clock_mhz: Option<u32>,
    pub max_mem_clock_mhz: Option<u32>,
    pub power_limit_w: Option<f64>,
    pub pcie_current: Option<(u32, u32)>,
    pub pcie_max: Option<(u32, u32)>,
    /// Size of the CPU-visible BAR1 aperture (`nvmlDeviceGetBAR1MemoryInfo`).
    pub bar1_total: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GpuChip {
    pub chip: &'static str,
    pub architecture: &'static str,
    pub process: &'static str,
}

pub fn pci_vendor_name(id: u32) -> Option<&'static str> {
    Some(match id {
        0x10DE => "NVIDIA",
        0x1002 | 0x1022 => "AMD",
        0x8086 => "Intel",
        0x1414 => "Microsoft",
        0x5143 => "Qualcomm",
        0x1043 => "ASUS",
        0x1462 => "MSI",
        0x1458 => "Gigabyte",
        0x19DA => "ZOTAC",
        0x3842 => "EVGA",
        0x1569 => "Palit",
        0x1B4C => "Galax / KFA2",
        0x174B | 0x1DA2 => "Sapphire",
        0x148C => "PowerColor",
        0x1682 => "XFX",
        0x7377 => "Colorful",
        0x196E => "PNY",
        0x10B0 => "Gainward",
        0x1849 => "ASRock",
        0x1028 => "Dell",
        0x103C => "HP",
        0x17AA => "Lenovo",
        0x1025 => "Acer",
        0x144D => "Samsung",
        0x1D05 => "Tongfang",
        0x1558 => "Clevo",
        0x1D72 => "Xiaomi",
        _ => return None,
    })
}

struct ChipEntry {
    vendor: u32,
    lo: u32,
    hi: u32,
    chip: GpuChip,
}

const fn c(
    vendor: u32,
    lo: u32,
    hi: u32,
    chip: &'static str,
    architecture: &'static str,
    process: &'static str,
) -> ChipEntry {
    ChipEntry {
        vendor,
        lo,
        hi,
        chip: GpuChip {
            chip,
            architecture,
            process,
        },
    }
}

const NV: u32 = VENDOR_NVIDIA;
const AMD: u32 = VENDOR_AMD;
const INTEL: u32 = VENDOR_INTEL;

/// Device-id ranges per chip. NVIDIA and AMD allocate ids in blocks per die,
/// so ranges cover desktop, mobile and workstation SKUs of the same chip.
#[rustfmt::skip]
static CHIPS: &[ChipEntry] = &[
    c(NV, 0x1B00, 0x1B3F, "GP102", "Pascal", "TSMC 16 nm"),
    c(NV, 0x1B80, 0x1BBF, "GP104", "Pascal", "TSMC 16 nm"),
    c(NV, 0x1C00, 0x1C7F, "GP106", "Pascal", "TSMC 16 nm"),
    c(NV, 0x1C80, 0x1CFF, "GP107", "Pascal", "Samsung 14 nm"),
    c(NV, 0x1E00, 0x1E3F, "TU102", "Turing", "TSMC 12 nm"),
    c(NV, 0x1E80, 0x1EFF, "TU104", "Turing", "TSMC 12 nm"),
    c(NV, 0x1F00, 0x1F7F, "TU106", "Turing", "TSMC 12 nm"),
    c(NV, 0x1F80, 0x1FFF, "TU117", "Turing", "TSMC 12 nm"),
    c(NV, 0x2180, 0x21FF, "TU116", "Turing", "TSMC 12 nm"),
    c(NV, 0x2200, 0x223F, "GA102", "Ampere", "Samsung 8 nm"),
    c(NV, 0x2400, 0x24FF, "GA104", "Ampere", "Samsung 8 nm"),
    c(NV, 0x2500, 0x259F, "GA106", "Ampere", "Samsung 8 nm"),
    c(NV, 0x25A0, 0x25FF, "GA107", "Ampere", "Samsung 8 nm"),
    c(NV, 0x2600, 0x26FF, "AD102", "Ada Lovelace", "TSMC 4N"),
    c(NV, 0x2700, 0x277F, "AD103", "Ada Lovelace", "TSMC 4N"),
    c(NV, 0x2780, 0x27FF, "AD104", "Ada Lovelace", "TSMC 4N"),
    c(NV, 0x2800, 0x281F, "AD106", "Ada Lovelace", "TSMC 4N"),
    c(NV, 0x2820, 0x28FF, "AD107", "Ada Lovelace", "TSMC 4N"),
    c(NV, 0x2B00, 0x2BFF, "GB202", "Blackwell", "TSMC 4N"),
    c(NV, 0x2C00, 0x2CFF, "GB203", "Blackwell", "TSMC 4N"),
    c(NV, 0x2D00, 0x2D7F, "GB206", "Blackwell", "TSMC 4N"),
    c(NV, 0x2D80, 0x2DFF, "GB207", "Blackwell", "TSMC 4N"),
    c(NV, 0x2F00, 0x2FFF, "GB205", "Blackwell", "TSMC 4N"),
    c(AMD, 0x7310, 0x731F, "Navi 10", "RDNA", "TSMC 7 nm"),
    c(AMD, 0x7340, 0x734F, "Navi 14", "RDNA", "TSMC 7 nm"),
    c(AMD, 0x73A0, 0x73BF, "Navi 21", "RDNA 2", "TSMC 7 nm"),
    c(AMD, 0x73C0, 0x73DF, "Navi 22", "RDNA 2", "TSMC 7 nm"),
    c(AMD, 0x73E0, 0x73FF, "Navi 23", "RDNA 2", "TSMC 7 nm"),
    c(AMD, 0x7420, 0x743F, "Navi 24", "RDNA 2", "TSMC 6 nm"),
    c(AMD, 0x7440, 0x745F, "Navi 31", "RDNA 3", "TSMC 5 nm + 6 nm"),
    c(AMD, 0x7460, 0x747F, "Navi 32", "RDNA 3", "TSMC 5 nm + 6 nm"),
    c(AMD, 0x7480, 0x749F, "Navi 33", "RDNA 3", "TSMC 6 nm"),
    c(AMD, 0x7550, 0x755F, "Navi 48", "RDNA 4", "TSMC N4P"),
    c(AMD, 0x7590, 0x759F, "Navi 44", "RDNA 4", "TSMC N4P"),
    c(INTEL, 0x56A0, 0x56BF, "DG2 (Alchemist)", "Xe-HPG", "TSMC N6"),
    c(INTEL, 0xE200, 0xE21F, "BMG-G21 (Battlemage)", "Xe2-HPG", "TSMC N5"),
];

pub fn lookup_chip(vendor: u32, device: u32) -> Option<GpuChip> {
    CHIPS
        .iter()
        .find(|e| e.vendor == vendor && (e.lo..=e.hi).contains(&device))
        .map(|e| e.chip)
}

/// Parsed `GPU Engine` instance, e.g.
/// `pid_11228_luid_0x00000000_0x0000EF68_phys_0_eng_3_engtype_3D`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineInstance {
    pub pid: u32,
    pub luid: u64,
    pub phys: u32,
    pub engine: u32,
    /// Lower-cased engine type, e.g. "3d", "copy", "videodecode".
    pub engtype: String,
}

fn hex(s: &str) -> Option<u32> {
    let s = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X"))?;
    u32::from_str_radix(s, 16).ok()
}

/// Parse the `luid_0xHHHHHHHH_0xLLLLLLLL_phys_N` part shared by both counter sets.
fn parse_luid_phys<'a>(parts: &mut impl Iterator<Item = &'a str>) -> Option<(u64, u32)> {
    let high = hex(parts.next()?)?;
    let low = hex(parts.next()?)?;
    if !parts.next()?.eq_ignore_ascii_case("phys") {
        return None;
    }
    let phys = parts.next()?.parse().ok()?;
    Some((((high as u64) << 32) | low as u64, phys))
}

pub fn parse_engine_instance(name: &str) -> Option<EngineInstance> {
    let (head, engtype) = match name.split_once("_engtype_") {
        Some((h, t)) => (h, t),
        None => {
            let lower = name.to_ascii_lowercase();
            let i = lower.find("_engtype_")?;
            (&name[..i], &name[i + 9..])
        }
    };
    let mut parts = head.split('_');
    if !parts.next()?.eq_ignore_ascii_case("pid") {
        return None;
    }
    let pid = parts.next()?.parse().ok()?;
    if !parts.next()?.eq_ignore_ascii_case("luid") {
        return None;
    }
    let (luid, phys) = parse_luid_phys(&mut parts)?;
    if !parts.next()?.eq_ignore_ascii_case("eng") {
        return None;
    }
    let engine = parts.next()?.parse().ok()?;
    Some(EngineInstance {
        pid,
        luid,
        phys,
        engine,
        engtype: engtype.to_ascii_lowercase(),
    })
}

/// Parse a `GPU Adapter Memory` instance (`luid_0x..._0x..._phys_0`) into the LUID.
pub fn parse_adapter_instance(name: &str) -> Option<u64> {
    let mut parts = name.split('_');
    if !parts.next()?.eq_ignore_ascii_case("luid") {
        return None;
    }
    parse_luid_phys(&mut parts).map(|(luid, _)| luid)
}

/// Per-adapter utilization by engine type, computed like Task Manager:
/// sum each engine over all processes, then take the busiest engine per type.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AdapterLoad {
    /// (engine type, percent), sorted by engine type.
    pub by_type: Vec<(String, f64)>,
}

impl AdapterLoad {
    /// Overall GPU load: the busiest engine type.
    pub fn overall(&self) -> f64 {
        self.by_type.iter().map(|(_, v)| *v).fold(0.0, f64::max)
    }

    pub fn get(&self, engtype: &str) -> Option<f64> {
        self.by_type
            .iter()
            .find(|(t, _)| t == engtype)
            .map(|(_, v)| *v)
    }
}

pub fn aggregate_engine_load<'a>(
    samples: impl IntoIterator<Item = (&'a str, f64)>,
) -> HashMap<u64, AdapterLoad> {
    // (luid, phys, engine, engtype) -> summed percent
    let mut engines: HashMap<(u64, u32, u32, String), f64> = HashMap::new();
    for (name, v) in samples {
        if let Some(i) = parse_engine_instance(name) {
            *engines
                .entry((i.luid, i.phys, i.engine, i.engtype))
                .or_default() += v;
        }
    }
    let mut out: HashMap<u64, AdapterLoad> = HashMap::new();
    for ((luid, _, _, engtype), v) in engines {
        let load = out.entry(luid).or_default();
        let v = v.clamp(0.0, 100.0);
        match load.by_type.iter_mut().find(|(t, _)| *t == engtype) {
            Some((_, cur)) => *cur = cur.max(v),
            None => load.by_type.push((engtype, v)),
        }
    }
    for l in out.values_mut() {
        l.by_type.sort_by(|a, b| a.0.cmp(&b.0));
    }
    out
}

/// Display label for an engine type.
pub fn engtype_label(t: &str) -> String {
    match t {
        "3d" => "3D".into(),
        "copy" => "Copy".into(),
        "videodecode" => "Video Decode".into(),
        "videoencode" => "Video Encode".into(),
        "videoprocessing" => "Video Processing".into(),
        "compute" => "Compute".into(),
        "legacyoverlay" => "Legacy Overlay".into(),
        "vr" => "VR".into(),
        _ if t.starts_with("ofa") => "Optical Flow".into(),
        _ if t.starts_with("compute_") => format!("Compute {}", &t[8..]),
        _ => {
            let mut s = t.to_string();
            if let Some(f) = s.get_mut(..1) {
                f.make_ascii_uppercase();
            }
            s
        }
    }
}

/// Match NVML devices to DXGI adapters by PCI device + subsystem id, in order.
/// `nvml` items are (pci vendor|device<<16, subsystem id). Returns, per adapter, the NVML index.
pub fn match_nvml(adapters: &[GpuAdapter], nvml: &[(u32, u32)]) -> Vec<Option<usize>> {
    let mut used = vec![false; nvml.len()];
    adapters
        .iter()
        .map(|a| {
            if a.vendor_id != VENDOR_NVIDIA {
                return None;
            }
            let want = (a.device_id << 16) | a.vendor_id;
            let i = nvml
                .iter()
                .enumerate()
                .position(|(i, &(dev, sub))| !used[i] && dev == want && sub == a.subsys_id)?;
            used[i] = true;
            Some(i)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chips() {
        assert_eq!(lookup_chip(NV, 0x2204).unwrap().chip, "GA102");
        assert_eq!(
            lookup_chip(NV, 0x2684).unwrap().architecture,
            "Ada Lovelace"
        );
        assert_eq!(lookup_chip(NV, 0x2B85).unwrap().chip, "GB202");
        assert_eq!(lookup_chip(AMD, 0x744C).unwrap().chip, "Navi 31");
        assert_eq!(lookup_chip(INTEL, 0x56A0).unwrap().architecture, "Xe-HPG");
        assert!(lookup_chip(NV, 0x0001).is_none());
    }

    #[test]
    fn chip_ranges_do_not_overlap() {
        for (i, a) in CHIPS.iter().enumerate() {
            for b in &CHIPS[i + 1..] {
                assert!(
                    !(a.vendor == b.vendor && a.lo <= b.hi && b.lo <= a.hi),
                    "{} vs {}",
                    a.chip.chip,
                    b.chip.chip
                );
            }
        }
    }

    #[test]
    fn adapter_names() {
        let a = GpuAdapter {
            vendor_id: NV,
            device_id: 0x2204,
            subsys_id: 0x1613_19DA,
            ..Default::default()
        };
        assert_eq!(a.vendor_name(), "NVIDIA");
        assert_eq!(a.board_vendor(), Some("ZOTAC"));
        assert_eq!(a.pci_ids(), "10DE 2204 - 19DA 1613");
    }

    #[test]
    fn rebar_classification() {
        assert_eq!(
            ReBar::from_bar_size(256 << 20),
            ReBar::Disabled {
                bar_bytes: 256 << 20
            }
        );
        assert_eq!(
            ReBar::from_bar_size(32 << 30),
            ReBar::Enabled {
                bar_bytes: 32 << 30
            }
        );
        let mut g = GpuAdapter {
            vendor_id: NV,
            device_id: 0x2204,
            ..Default::default()
        };
        assert_eq!(g.rebar(), ReBar::Unknown);
        assert_eq!(g.supports_rebar(), Some(true));
        g.nvml = Some(NvmlDetails {
            bar1_total: Some(256 << 20),
            ..Default::default()
        });
        assert_eq!(g.rebar().label(), "Off");
        let turing = GpuAdapter {
            vendor_id: NV,
            device_id: 0x1E04,
            ..Default::default()
        };
        assert_eq!(turing.supports_rebar(), Some(false));
        assert_eq!(GpuAdapter::default().supports_rebar(), None);
    }

    #[test]
    fn integrated_detection() {
        let uhd = GpuAdapter {
            vendor_id: INTEL,
            device_id: 0xA780,
            dedicated_vram: 128 << 20,
            ..Default::default()
        };
        let arc = GpuAdapter {
            vendor_id: INTEL,
            device_id: 0x56A0,
            dedicated_vram: 16 << 30,
            ..Default::default()
        };
        let apu = GpuAdapter {
            vendor_id: AMD,
            device_id: 0x164E,
            dedicated_vram: 512 << 20,
            ..Default::default()
        };
        let rx = GpuAdapter {
            vendor_id: AMD,
            device_id: 0x744C,
            dedicated_vram: 24 << 30,
            ..Default::default()
        };
        let rtx = GpuAdapter {
            vendor_id: NV,
            device_id: 0x2204,
            dedicated_vram: 24 << 30,
            ..Default::default()
        };
        assert!(uhd.is_integrated() && apu.is_integrated());
        assert!(!arc.is_integrated() && !rx.is_integrated() && !rtx.is_integrated());
    }

    #[test]
    fn engine_instances() {
        let i = parse_engine_instance(
            "pid_11228_luid_0x00000000_0x0000EF68_phys_0_eng_10_engtype_Copy",
        )
        .unwrap();
        assert_eq!(
            i,
            EngineInstance {
                pid: 11228,
                luid: 0xEF68,
                phys: 0,
                engine: 10,
                engtype: "copy".into()
            }
        );
        let i = parse_engine_instance("pid_4_luid_0x00000001_0x0000ef68_phys_1_eng_0_engtype_")
            .unwrap();
        assert_eq!((i.luid, i.phys, i.engtype.as_str()), (0x1_0000_EF68, 1, ""));
        assert!(parse_engine_instance("_Total").is_none());
        assert_eq!(
            parse_adapter_instance("luid_0x00000000_0x000108A9_phys_0"),
            Some(0x108A9)
        );
        assert_eq!(parse_adapter_instance("garbage"), None);
    }

    #[test]
    fn aggregation() {
        let a = "pid_1_luid_0x00000000_0x0000EF68_phys_0_eng_0_engtype_3D";
        let b = "pid_2_luid_0x00000000_0x0000EF68_phys_0_eng_0_engtype_3D";
        let c = "pid_2_luid_0x00000000_0x0000EF68_phys_0_eng_5_engtype_VideoDecode";
        let d = "pid_3_luid_0x00000000_0x0000EF68_phys_0_eng_6_engtype_VideoDecode";
        let other = "pid_3_luid_0x00000000_0x000108A9_phys_0_eng_0_engtype_3D";
        let m = aggregate_engine_load([(a, 30.0), (b, 25.0), (c, 10.0), (d, 40.0), (other, 80.0)]);
        let l = &m[&0xEF68];
        assert_eq!(l.get("3d"), Some(55.0));
        assert_eq!(l.get("videodecode"), Some(40.0));
        assert_eq!(l.overall(), 55.0);
        assert_eq!(m[&0x108A9].overall(), 80.0);
    }

    #[test]
    fn engine_labels() {
        assert_eq!(engtype_label("3d"), "3D");
        assert_eq!(engtype_label("videodecode"), "Video Decode");
        assert_eq!(engtype_label("compute_1"), "Compute 1");
        assert_eq!(engtype_label("security"), "Security");
        assert_eq!(engtype_label("ofa_0"), "Optical Flow");
    }

    #[test]
    fn nvml_matching() {
        let nv = |dev, sub| GpuAdapter {
            vendor_id: NV,
            device_id: dev,
            subsys_id: sub,
            ..Default::default()
        };
        let amd = GpuAdapter {
            vendor_id: AMD,
            ..Default::default()
        };
        let adapters = [amd, nv(0x2204, 0x161319DA), nv(0x2204, 0x161319DA)];
        let devs = [(0x2204_10DE, 0x161319DA), (0x2204_10DE, 0x161319DA)];
        assert_eq!(match_nvml(&adapters, &devs), vec![None, Some(0), Some(1)]);
        assert_eq!(match_nvml(&adapters[..2], &[]), vec![None, None]);
    }
}
