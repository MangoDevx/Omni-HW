//! CPUID decoding.
//!
//! Decoding works on any [`CpuidSource`], so tests can feed canned register
//! values. [`NativeCpuid`] executes the real instruction on x86-64.

/// The four registers returned by one CPUID invocation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Regs {
    pub eax: u32,
    pub ebx: u32,
    pub ecx: u32,
    pub edx: u32,
}

/// Anything that can answer CPUID queries.
pub trait CpuidSource {
    fn cpuid(&self, leaf: u32, subleaf: u32) -> Regs;
}

/// Executes the CPUID instruction on the current CPU.
#[derive(Debug, Clone, Copy, Default)]
pub struct NativeCpuid;

impl CpuidSource for NativeCpuid {
    #[cfg(target_arch = "x86_64")]
    fn cpuid(&self, leaf: u32, subleaf: u32) -> Regs {
        // SAFETY: CPUID is available on every x86-64 CPU. (Safe fn on newer toolchains.)
        #[allow(unused_unsafe)]
        let r = unsafe { core::arch::x86_64::__cpuid_count(leaf, subleaf) };
        Regs {
            eax: r.eax,
            ebx: r.ebx,
            ecx: r.ecx,
            edx: r.edx,
        }
    }

    #[cfg(not(target_arch = "x86_64"))]
    fn cpuid(&self, _leaf: u32, _subleaf: u32) -> Regs {
        Regs::default()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Vendor {
    Intel,
    Amd,
    #[default]
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheKind {
    Data,
    Instruction,
    Unified,
}

impl CacheKind {
    pub fn suffix(self) -> &'static str {
        match self {
            CacheKind::Data => "D",
            CacheKind::Instruction => "I",
            CacheKind::Unified => "",
        }
    }
}

/// One cache level as described by CPUID leaf 4 / 0x8000001D.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheDesc {
    pub level: u8,
    pub kind: CacheKind,
    pub size_bytes: u64,
    pub ways: u32,
    pub line_size: u32,
    pub sets: u32,
    /// Maximum number of logical processors sharing this cache.
    pub shared_by: u32,
}

impl CacheDesc {
    pub fn name(&self) -> String {
        format!("L{}{}", self.level, self.kind.suffix())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CpuInfo {
    pub vendor: Vendor,
    pub vendor_string: String,
    pub brand: String,
    /// Displayed (combined) family.
    pub family: u32,
    /// Displayed (combined) model.
    pub model: u32,
    pub stepping: u32,
    pub base_family: u32,
    pub base_model: u32,
    pub ext_family: u32,
    pub ext_model: u32,
    pub max_leaf: u32,
    pub max_ext_leaf: u32,
    /// Instruction set extensions, in display order.
    pub features: Vec<&'static str>,
    /// CPUID leaf 7 EDX bit 15 (Intel hybrid architecture).
    pub hybrid: bool,
    pub caches: Vec<CacheDesc>,
    /// Physical address bits (leaf 0x80000008), 0 if unknown.
    pub phys_addr_bits: u32,
    pub virt_addr_bits: u32,
}

impl CpuInfo {
    pub fn has(&self, feature: &str) -> bool {
        self.features.contains(&feature)
    }

    /// CPU-Z style instruction list, e.g. "MMX, SSE, SSE2, ..., AVX2".
    pub fn instructions_line(&self) -> String {
        self.features.join(", ")
    }
}

fn bit(v: u32, n: u32) -> bool {
    (v >> n) & 1 == 1
}

fn regs_to_ascii(out: &mut Vec<u8>, regs: &[u32]) {
    for r in regs {
        out.extend_from_slice(&r.to_le_bytes());
    }
}

fn clean_string(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    let s = String::from_utf8_lossy(&bytes[..end]);
    // Brand strings are often padded and sometimes contain runs of spaces.
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Decode a leaf-4 style cache descriptor; `None` terminates the list.
fn decode_cache_leaf(r: Regs) -> Option<CacheDesc> {
    let kind = match r.eax & 0x1F {
        1 => CacheKind::Data,
        2 => CacheKind::Instruction,
        3 => CacheKind::Unified,
        _ => return None,
    };
    let level = ((r.eax >> 5) & 0x7) as u8;
    let shared_by = ((r.eax >> 14) & 0xFFF) + 1;
    let line_size = (r.ebx & 0xFFF) + 1;
    let partitions = ((r.ebx >> 12) & 0x3FF) + 1;
    let ways = ((r.ebx >> 22) & 0x3FF) + 1;
    let sets = r.ecx.wrapping_add(1);
    let size_bytes = ways as u64 * partitions as u64 * line_size as u64 * sets as u64;
    Some(CacheDesc {
        level,
        kind,
        size_bytes,
        ways,
        line_size,
        sets,
        shared_by,
    })
}

/// Decode everything we need from CPUID.
pub fn decode(src: &impl CpuidSource) -> CpuInfo {
    let l0 = src.cpuid(0, 0);
    let max_leaf = l0.eax;
    let mut vb = Vec::with_capacity(12);
    regs_to_ascii(&mut vb, &[l0.ebx, l0.edx, l0.ecx]);
    let vendor_string = clean_string(&vb);
    let vendor = match vendor_string.as_str() {
        "GenuineIntel" => Vendor::Intel,
        "AuthenticAMD" | "HygonGenuine" => Vendor::Amd,
        _ => Vendor::Other,
    };

    let l1 = if max_leaf >= 1 {
        src.cpuid(1, 0)
    } else {
        Regs::default()
    };
    let stepping = l1.eax & 0xF;
    let base_model = (l1.eax >> 4) & 0xF;
    let base_family = (l1.eax >> 8) & 0xF;
    let ext_model = (l1.eax >> 16) & 0xF;
    let ext_family = (l1.eax >> 20) & 0xFF;
    let family = if base_family == 0xF {
        base_family + ext_family
    } else {
        base_family
    };
    let model = if base_family == 0x6 || base_family == 0xF {
        (ext_model << 4) | base_model
    } else {
        base_model
    };

    let l7 = if max_leaf >= 7 {
        src.cpuid(7, 0)
    } else {
        Regs::default()
    };
    let l7_1 = if max_leaf >= 7 && l7.eax >= 1 {
        src.cpuid(7, 1)
    } else {
        Regs::default()
    };

    let max_ext_leaf = src.cpuid(0x8000_0000, 0).eax;
    let has_ext = |leaf: u32| max_ext_leaf >= 0x8000_0000 && max_ext_leaf >= leaf;
    let e1 = if has_ext(0x8000_0001) {
        src.cpuid(0x8000_0001, 0)
    } else {
        Regs::default()
    };

    let brand = if has_ext(0x8000_0004) {
        let mut b = Vec::with_capacity(48);
        for leaf in 0x8000_0002..=0x8000_0004 {
            let r = src.cpuid(leaf, 0);
            regs_to_ascii(&mut b, &[r.eax, r.ebx, r.ecx, r.edx]);
        }
        clean_string(&b)
    } else {
        String::new()
    };

    let (phys_addr_bits, virt_addr_bits) = if has_ext(0x8000_0008) {
        let r = src.cpuid(0x8000_0008, 0);
        (r.eax & 0xFF, (r.eax >> 8) & 0xFF)
    } else {
        (0, 0)
    };

    // Feature list, ordered roughly like CPU-Z.
    let checks: &[(bool, &'static str)] = &[
        (bit(l1.edx, 23), "MMX"),
        (bit(l1.edx, 25), "SSE"),
        (bit(l1.edx, 26), "SSE2"),
        (bit(l1.ecx, 0), "SSE3"),
        (bit(l1.ecx, 9), "SSSE3"),
        (bit(l1.ecx, 19), "SSE4.1"),
        (bit(l1.ecx, 20), "SSE4.2"),
        (vendor == Vendor::Amd && bit(e1.ecx, 6), "SSE4A"),
        (bit(e1.edx, 29), "x86-64"),
        (bit(l1.ecx, 5), "VT-x"),
        (vendor == Vendor::Amd && bit(e1.ecx, 2), "AMD-V"),
        (bit(l1.ecx, 25), "AES"),
        (bit(l1.ecx, 1), "PCLMULQDQ"),
        (bit(l1.ecx, 28), "AVX"),
        (bit(l7.ebx, 5), "AVX2"),
        (bit(l7_1.eax, 4), "AVX-VNNI"),
        (bit(l7_1.eax, 23), "AVX-IFMA"),
        (bit(l7.ebx, 16), "AVX512F"),
        (bit(l7.ebx, 17), "AVX512DQ"),
        (bit(l7.ebx, 28), "AVX512CD"),
        (bit(l7.ebx, 30), "AVX512BW"),
        (bit(l7.ebx, 31), "AVX512VL"),
        (bit(l7.ebx, 21), "AVX512_IFMA"),
        (bit(l7.ecx, 1), "AVX512_VBMI"),
        (bit(l7.ecx, 6), "AVX512_VBMI2"),
        (bit(l7.ecx, 11), "AVX512_VNNI"),
        (bit(l7.ecx, 12), "AVX512_BITALG"),
        (bit(l7.ecx, 14), "AVX512_VPOPCNTDQ"),
        (bit(l7_1.eax, 5), "AVX512_BF16"),
        (bit(l7.edx, 23), "AVX512_FP16"),
        (bit(l7_1.edx, 19), "AVX10"),
        (bit(l7.edx, 24), "AMX-TILE"),
        (bit(l7.edx, 22), "AMX-BF16"),
        (bit(l7.edx, 25), "AMX-INT8"),
        (bit(l1.ecx, 12), "FMA3"),
        (vendor == Vendor::Amd && bit(e1.ecx, 16), "FMA4"),
        (bit(l1.ecx, 29), "F16C"),
        (bit(l7.ebx, 29), "SHA"),
        (bit(l7.ecx, 9), "VAES"),
        (bit(l7.ecx, 10), "VPCLMULQDQ"),
        (bit(l7.ecx, 8), "GFNI"),
        (bit(l7.ebx, 3), "BMI1"),
        (bit(l7.ebx, 8), "BMI2"),
        (bit(e1.ecx, 5), "LZCNT"),
        (bit(l1.ecx, 23), "POPCNT"),
        (bit(l7.ebx, 19), "ADX"),
        (bit(l1.ecx, 30), "RDRAND"),
        (bit(l7.ebx, 18), "RDSEED"),
    ];
    let features = checks
        .iter()
        .filter(|(on, _)| *on)
        .map(|(_, n)| *n)
        .collect();

    let hybrid = vendor == Vendor::Intel && bit(l7.edx, 15);

    let mut caches = Vec::new();
    let cache_leaf = match vendor {
        Vendor::Intel if max_leaf >= 4 => Some(4),
        // Topology extensions (0x8000001D) are flagged by 0x80000001 ECX bit 22.
        Vendor::Amd if has_ext(0x8000_001D) && bit(e1.ecx, 22) => Some(0x8000_001D),
        _ => None,
    };
    if let Some(leaf) = cache_leaf {
        for sub in 0..16 {
            match decode_cache_leaf(src.cpuid(leaf, sub)) {
                Some(c) => caches.push(c),
                None => break,
            }
        }
    }

    CpuInfo {
        vendor,
        vendor_string,
        brand,
        family,
        model,
        stepping,
        base_family,
        base_model,
        ext_family,
        ext_model,
        max_leaf,
        max_ext_leaf,
        features,
        hybrid,
        caches,
        phys_addr_bits,
        virt_addr_bits,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Canned CPUID responses keyed by (leaf, subleaf). Missing entries return zero.
    pub struct Canned(pub HashMap<(u32, u32), Regs>);

    impl CpuidSource for Canned {
        fn cpuid(&self, leaf: u32, subleaf: u32) -> Regs {
            self.0.get(&(leaf, subleaf)).copied().unwrap_or_default()
        }
    }

    fn r(eax: u32, ebx: u32, ecx: u32, edx: u32) -> Regs {
        Regs { eax, ebx, ecx, edx }
    }

    fn brand_regs(s: &str) -> [Regs; 3] {
        let mut b = [0u8; 48];
        b[..s.len()].copy_from_slice(s.as_bytes());
        let w = |i: usize| u32::from_le_bytes(b[i * 4..i * 4 + 4].try_into().unwrap());
        [
            r(w(0), w(1), w(2), w(3)),
            r(w(4), w(5), w(6), w(7)),
            r(w(8), w(9), w(10), w(11)),
        ]
    }

    /// Register dump modeled on an Intel Core i9-13900K (Raptor Lake-S, B0).
    pub fn raptor_lake() -> Canned {
        let mut m = HashMap::new();
        m.insert((0, 0), r(0x20, 0x756E_6547, 0x6C65_746E, 0x4965_6E69)); // GenuineIntel
        m.insert(
            (1, 0),
            r(0x000B_0671, 0x0080_0800, 0x7FFA_FBFF, 0xBFEB_FBFF),
        );
        m.insert((4, 0), r(0xFC00_4121, 0x02C0_003F, 0x0000_003F, 0)); // L1D 48K 12-way
        m.insert((4, 1), r(0xFC00_4122, 0x01C0_003F, 0x0000_003F, 0)); // L1I 32K 8-way
        m.insert((4, 2), r(0xFC01_C143, 0x03C0_003F, 0x0000_07FF, 0)); // L2 2M 16-way
        m.insert((4, 3), r(0xFC1F_C163, 0x02C0_003F, 0x0000_BFFF, 0x4)); // L3 36M 12-way
        m.insert((7, 0), r(0x2, 0x239C_A7EB, 0x9840_072C, 0xFC1C_C410));
        m.insert((7, 1), r(0x0040_0810, 0, 0, 0));
        m.insert((0x8000_0000, 0), r(0x8000_0008, 0, 0, 0));
        m.insert((0x8000_0001, 0), r(0, 0, 0x121, 0x2C10_0800));
        let b = brand_regs("13th Gen Intel(R) Core(TM) i9-13900K");
        m.insert((0x8000_0002, 0), b[0]);
        m.insert((0x8000_0003, 0), b[1]);
        m.insert((0x8000_0004, 0), b[2]);
        m.insert((0x8000_0008, 0), r(0x3027, 0, 0, 0));
        Canned(m)
    }

    /// Register dump modeled on an AMD Ryzen 7 7800X3D (Raphael, family 19h model 61h).
    pub fn raphael() -> Canned {
        let mut m = HashMap::new();
        m.insert((0, 0), r(0x10, 0x6874_7541, 0x444D_4163, 0x6974_6E65)); // AuthenticAMD
        m.insert(
            (1, 0),
            r(0x00A6_0F12, 0x0010_0800, 0x7EF8_320B, 0x178B_FBFF),
        );
        m.insert((7, 0), r(0x1, 0xF1BF_97A9, 0x0040_5FCE, 0x1000_0010));
        m.insert((7, 1), r(0x20, 0, 0, 0));
        m.insert(
            (0x8000_0000, 0),
            r(0x8000_0028, 0x6874_7541, 0x444D_4163, 0x6974_6E65),
        );
        m.insert(
            (0x8000_0001, 0),
            r(0x00A6_0F12, 0, 0x75C2_37FF, 0x2FD3_FBFF),
        );
        let b = brand_regs("AMD Ryzen 7 7800X3D 8-Core Processor           ");
        m.insert((0x8000_0002, 0), b[0]);
        m.insert((0x8000_0003, 0), b[1]);
        m.insert((0x8000_0004, 0), b[2]);
        m.insert((0x8000_0008, 0), r(0x3030, 0, 0, 0));
        m.insert(
            (0x8000_001D, 0),
            r(0x0000_4121, 0x01C0_003F, 0x0000_003F, 0),
        ); // L1D 32K 8-way
        m.insert(
            (0x8000_001D, 1),
            r(0x0000_4122, 0x01C0_003F, 0x0000_003F, 0),
        ); // L1I 32K 8-way
        m.insert(
            (0x8000_001D, 2),
            r(0x0000_4143, 0x01C0_003F, 0x0000_07FF, 0x2),
        ); // L2 1M 8-way
        m.insert(
            (0x8000_001D, 3),
            r(0x0003_C163, 0x03C0_003F, 0x0001_7FFF, 0x1),
        ); // L3 96M 16-way
        Canned(m)
    }

    #[test]
    fn intel_identity() {
        let c = decode(&raptor_lake());
        assert_eq!(c.vendor, Vendor::Intel);
        assert_eq!(c.vendor_string, "GenuineIntel");
        assert_eq!(c.brand, "13th Gen Intel(R) Core(TM) i9-13900K");
        assert_eq!((c.family, c.model, c.stepping), (6, 0xB7, 1));
        assert_eq!((c.ext_family, c.ext_model), (0, 0xB));
        assert!(c.hybrid);
        assert_eq!((c.phys_addr_bits, c.virt_addr_bits), (39, 48));
    }

    #[test]
    fn intel_features() {
        let c = decode(&raptor_lake());
        for f in [
            "MMX", "SSE4.2", "x86-64", "VT-x", "AES", "AVX", "AVX2", "AVX-VNNI", "FMA3", "SHA",
            "BMI2",
        ] {
            assert!(c.has(f), "missing {f}");
        }
        assert!(!c.has("AVX512F"));
        assert!(!c.has("SSE4A"));
        assert!(!c.has("AMD-V"));
    }

    #[test]
    fn intel_caches() {
        let c = decode(&raptor_lake());
        let summary: Vec<_> = c
            .caches
            .iter()
            .map(|d| (d.name(), d.size_bytes / 1024, d.ways))
            .collect();
        assert_eq!(
            summary,
            vec![
                ("L1D".to_string(), 48, 12),
                ("L1I".to_string(), 32, 8),
                ("L2".to_string(), 2048, 16),
                ("L3".to_string(), 36 * 1024, 12),
            ]
        );
        assert_eq!(c.caches[2].shared_by, 8);
    }

    #[test]
    fn amd_identity_and_caches() {
        let c = decode(&raphael());
        assert_eq!(c.vendor, Vendor::Amd);
        assert_eq!(c.brand, "AMD Ryzen 7 7800X3D 8-Core Processor");
        assert_eq!((c.family, c.model, c.stepping), (0x19, 0x61, 2));
        assert_eq!((c.base_family, c.ext_family), (0xF, 0xA));
        assert!(!c.hybrid);
        assert!(c.has("AMD-V") && c.has("SSE4A") && c.has("AVX512F") && c.has("AVX512_BF16"));
        assert!(!c.has("VT-x"));
        let l3 = c.caches.iter().find(|d| d.level == 3).unwrap();
        assert_eq!(l3.size_bytes, 96 * 1024 * 1024);
        assert_eq!(l3.ways, 16);
        assert_eq!(l3.shared_by, 16);
    }

    #[test]
    fn empty_source_does_not_panic() {
        let c = decode(&Canned(HashMap::new()));
        assert_eq!(c.vendor, Vendor::Other);
        assert!(c.features.is_empty() && c.caches.is_empty() && c.brand.is_empty());
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn native_runs() {
        let c = decode(&NativeCpuid);
        assert!(c.has("SSE2") && c.has("x86-64"));
    }
}
