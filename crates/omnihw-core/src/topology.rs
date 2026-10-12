//! Parser for the buffer returned by `GetLogicalProcessorInformationEx(RelationAll)`.
//!
//! The buffer is a sequence of variable-length
//! `SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX` records. Offsets below are for
//! 64-bit Windows (`KAFFINITY` is 8 bytes, `GROUP_AFFINITY` is 16 bytes).

use crate::cpuid::CacheKind;

const REL_PROCESSOR_CORE: u32 = 0;
const REL_NUMA_NODE: u32 = 1;
const REL_CACHE: u32 = 2;
const REL_PROCESSOR_PACKAGE: u32 = 3;
const REL_GROUP: u32 = 4;
const REL_PROCESSOR_DIE: u32 = 5;
const REL_NUMA_NODE_EX: u32 = 6;

const LTP_PC_SMT: u8 = 0x1;
const GROUP_AFFINITY_SIZE: usize = 16;

/// A logical processor identified by its processor group and index within the group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LogicalId {
    pub group: u16,
    pub number: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Core {
    /// Higher = more performant. All zero on non-hybrid CPUs.
    pub efficiency_class: u8,
    pub smt: bool,
    pub logical: Vec<LogicalId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheInstance {
    pub level: u8,
    pub kind: CacheKind,
    pub size_bytes: u64,
    /// 0xFF means fully associative.
    pub associativity: u8,
    pub line_size: u16,
    pub logical: Vec<LogicalId>,
}

/// Caches of the same level/kind/size, aggregated CPU-Z style ("8 x 48 KB").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheSummary {
    pub level: u8,
    pub kind: CacheKind,
    pub size_bytes: u64,
    pub associativity: u8,
    pub line_size: u16,
    pub count: usize,
}

impl CacheSummary {
    pub fn name(&self) -> String {
        format!("L{}{}", self.level, self.kind.suffix())
    }

    pub fn ways_text(&self) -> String {
        match self.associativity {
            0xFF => "fully assoc.".into(),
            0 => "-".into(),
            n => format!("{n}-way"),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Topology {
    pub packages: usize,
    pub dies: usize,
    pub numa_nodes: usize,
    pub groups: usize,
    pub cores: Vec<Core>,
    pub caches: Vec<CacheInstance>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    Truncated { offset: usize },
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::Truncated { offset } => write!(f, "record truncated at offset {offset}"),
        }
    }
}

impl std::error::Error for ParseError {}

fn u8_at(b: &[u8], o: usize) -> Option<u8> {
    b.get(o).copied()
}
fn u16_at(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?))
}
fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
}
fn u64_at(b: &[u8], o: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(o..o + 8)?.try_into().ok()?))
}

/// Expand `count` GROUP_AFFINITY structs starting at `off` into logical ids.
fn group_masks(rec: &[u8], off: usize, count: usize) -> Option<Vec<LogicalId>> {
    let mut out = Vec::new();
    for i in 0..count {
        let base = off + i * GROUP_AFFINITY_SIZE;
        let mask = u64_at(rec, base)?;
        let group = u16_at(rec, base + 8)?;
        for n in 0..64u8 {
            if mask >> n & 1 == 1 {
                out.push(LogicalId { group, number: n });
            }
        }
    }
    Some(out)
}

impl Topology {
    pub fn parse(buf: &[u8]) -> Result<Topology, ParseError> {
        let mut t = Topology::default();
        let mut off = 0;
        while off < buf.len() {
            let truncated = ParseError::Truncated { offset: off };
            let rel = u32_at(buf, off).ok_or(truncated.clone())?;
            let size = u32_at(buf, off + 4).ok_or(truncated.clone())? as usize;
            if size < 8 || off + size > buf.len() {
                return Err(truncated);
            }
            let rec = &buf[off..off + size];
            match rel {
                REL_PROCESSOR_CORE => {
                    let flags = u8_at(rec, 8).ok_or(truncated.clone())?;
                    let efficiency_class = u8_at(rec, 9).ok_or(truncated.clone())?;
                    let gc = u16_at(rec, 30).ok_or(truncated.clone())? as usize;
                    let logical = group_masks(rec, 32, gc).ok_or(truncated.clone())?;
                    t.cores.push(Core {
                        efficiency_class,
                        smt: flags & LTP_PC_SMT != 0,
                        logical,
                    });
                }
                REL_PROCESSOR_PACKAGE => t.packages += 1,
                REL_PROCESSOR_DIE => t.dies += 1,
                REL_NUMA_NODE | REL_NUMA_NODE_EX => t.numa_nodes += 1,
                REL_GROUP => t.groups = u16_at(rec, 10).ok_or(truncated.clone())? as usize,
                REL_CACHE => {
                    let level = u8_at(rec, 8).ok_or(truncated.clone())?;
                    let associativity = u8_at(rec, 9).ok_or(truncated.clone())?;
                    let line_size = u16_at(rec, 10).ok_or(truncated.clone())?;
                    let size_bytes = u32_at(rec, 12).ok_or(truncated.clone())? as u64;
                    let kind = match u32_at(rec, 16).ok_or(truncated.clone())? {
                        1 => CacheKind::Instruction,
                        2 => CacheKind::Data,
                        3 => {
                            // Trace caches are obsolete; skip them.
                            off += size;
                            continue;
                        }
                        _ => CacheKind::Unified,
                    };
                    // GroupCount was a reserved field (0) before Windows 11; treat as 1.
                    let gc = (u16_at(rec, 38).ok_or(truncated.clone())? as usize).max(1);
                    let logical = group_masks(rec, 40, gc).ok_or(truncated.clone())?;
                    t.caches.push(CacheInstance {
                        level,
                        kind,
                        size_bytes,
                        associativity,
                        line_size,
                        logical,
                    });
                }
                _ => {}
            }
            off += size;
        }
        Ok(t)
    }

    pub fn logical_count(&self) -> usize {
        self.cores.iter().map(|c| c.logical.len()).sum()
    }

    /// True if cores report more than one efficiency class (Intel hybrid / AMD Zen 5c mixes).
    pub fn is_hybrid(&self) -> bool {
        let mut it = self.cores.iter().map(|c| c.efficiency_class);
        match it.next() {
            Some(first) => it.any(|c| c != first),
            None => false,
        }
    }

    fn max_class(&self) -> u8 {
        self.cores
            .iter()
            .map(|c| c.efficiency_class)
            .max()
            .unwrap_or(0)
    }

    /// Whether `core` is a performance core. On non-hybrid CPUs every core is.
    pub fn is_p_core(&self, core: &Core) -> bool {
        !self.is_hybrid() || core.efficiency_class == self.max_class()
    }

    /// (P-cores, E-cores). On non-hybrid CPUs E-cores is 0.
    pub fn p_e_counts(&self) -> (usize, usize) {
        let p = self.cores.iter().filter(|c| self.is_p_core(c)).count();
        (p, self.cores.len() - p)
    }

    /// Index of the core owning `id`, if any.
    pub fn core_of(&self, id: LogicalId) -> Option<usize> {
        self.cores.iter().position(|c| c.logical.contains(&id))
    }

    /// Display label for a core, e.g. "P-core 3" / "E-core 0" / "Core 5".
    pub fn core_label(&self, index: usize) -> String {
        let Some(core) = self.cores.get(index) else {
            return format!("Core {index}");
        };
        if !self.is_hybrid() {
            return format!("Core {index}");
        }
        let p = self.is_p_core(core);
        let n = self.cores[..index]
            .iter()
            .filter(|c| self.is_p_core(c) == p)
            .count();
        format!("{}-core {n}", if p { "P" } else { "E" })
    }

    /// Aggregate cache instances by (level, kind, size, associativity), sorted by level then kind.
    /// Within a level and kind, Windows' enumeration order is kept (P-core caches first on hybrids).
    pub fn cache_summary(&self) -> Vec<CacheSummary> {
        let mut out: Vec<CacheSummary> = Vec::new();
        for c in &self.caches {
            match out.iter_mut().find(|s| {
                s.level == c.level
                    && s.kind == c.kind
                    && s.size_bytes == c.size_bytes
                    && s.associativity == c.associativity
            }) {
                Some(s) => s.count += 1,
                None => out.push(CacheSummary {
                    level: c.level,
                    kind: c.kind,
                    size_bytes: c.size_bytes,
                    associativity: c.associativity,
                    line_size: c.line_size,
                    count: 1,
                }),
            }
        }
        let kind_order = |k: CacheKind| match k {
            CacheKind::Data => 0,
            CacheKind::Instruction => 1,
            CacheKind::Unified => 2,
        };
        out.sort_by_key(|s| (s.level, kind_order(s.kind)));
        out
    }
}

/// Builders for synthetic buffers, used by tests here and in other modules.
#[cfg(test)]
pub(crate) mod build {
    const PROCESSOR_GROUP_INFO_SIZE: usize = 48;

    fn header(rel: u32, size: usize) -> Vec<u8> {
        let mut v = vec![0u8; size];
        v[0..4].copy_from_slice(&rel.to_le_bytes());
        v[4..8].copy_from_slice(&(size as u32).to_le_bytes());
        v
    }

    fn put_mask(v: &mut [u8], off: usize, mask: u64, group: u16) {
        v[off..off + 8].copy_from_slice(&mask.to_le_bytes());
        v[off + 8..off + 10].copy_from_slice(&group.to_le_bytes());
    }

    pub fn core(smt: bool, class: u8, mask: u64) -> Vec<u8> {
        let mut v = header(0, 48);
        v[8] = smt as u8;
        v[9] = class;
        v[30..32].copy_from_slice(&1u16.to_le_bytes());
        put_mask(&mut v, 32, mask, 0);
        v
    }

    pub fn package(mask: u64) -> Vec<u8> {
        let mut v = header(3, 48);
        v[30..32].copy_from_slice(&1u16.to_le_bytes());
        put_mask(&mut v, 32, mask, 0);
        v
    }

    /// `kind`: 0 unified, 1 instruction, 2 data.
    pub fn cache(level: u8, ways: u8, kind: u32, size: u32, mask: u64) -> Vec<u8> {
        let mut v = header(2, 56);
        v[8] = level;
        v[9] = ways;
        v[10..12].copy_from_slice(&64u16.to_le_bytes());
        v[12..16].copy_from_slice(&size.to_le_bytes());
        v[16..20].copy_from_slice(&kind.to_le_bytes());
        v[38..40].copy_from_slice(&1u16.to_le_bytes());
        put_mask(&mut v, 40, mask, 0);
        v
    }

    pub fn group(active_procs: u8) -> Vec<u8> {
        let mut v = header(4, 32 + PROCESSOR_GROUP_INFO_SIZE);
        v[8..10].copy_from_slice(&1u16.to_le_bytes());
        v[10..12].copy_from_slice(&1u16.to_le_bytes());
        v[32] = active_procs;
        v[33] = active_procs;
        v
    }

    pub fn numa() -> Vec<u8> {
        header(1, 48)
    }

    /// A small hybrid part: 2 P-cores with SMT + 4 E-cores, one package.
    pub fn hybrid_2p4e() -> Vec<u8> {
        let mut b = Vec::new();
        b.extend(core(true, 1, 0b0000_0011));
        b.extend(cache(1, 12, 2, 48 * 1024, 0b0000_0011));
        b.extend(cache(1, 8, 1, 32 * 1024, 0b0000_0011));
        b.extend(cache(2, 10, 0, 1280 * 1024, 0b0000_0011));
        b.extend(core(true, 1, 0b0000_1100));
        b.extend(cache(1, 12, 2, 48 * 1024, 0b0000_1100));
        b.extend(cache(1, 8, 1, 32 * 1024, 0b0000_1100));
        b.extend(cache(2, 10, 0, 1280 * 1024, 0b0000_1100));
        for i in 0..4 {
            let m = 1u64 << (4 + i);
            b.extend(core(false, 0, m));
            b.extend(cache(1, 8, 2, 32 * 1024, m));
            b.extend(cache(1, 8, 1, 64 * 1024, m));
        }
        b.extend(cache(2, 16, 0, 2048 * 1024, 0b1111_0000));
        b.extend(cache(3, 12, 0, 12 * 1024 * 1024, 0xFF));
        b.extend(package(0xFF));
        b.extend(numa());
        b.extend(group(8));
        b
    }
}

#[cfg(test)]
mod tests {
    use super::build::*;
    use super::*;

    #[test]
    fn hybrid_counts() {
        let t = Topology::parse(&hybrid_2p4e()).unwrap();
        assert_eq!(t.packages, 1);
        assert_eq!(t.numa_nodes, 1);
        assert_eq!(t.groups, 1);
        assert_eq!(t.cores.len(), 6);
        assert_eq!(t.logical_count(), 8);
        assert!(t.is_hybrid());
        assert_eq!(t.p_e_counts(), (2, 4));
        assert_eq!(t.core_label(1), "P-core 1");
        assert_eq!(t.core_label(2), "E-core 0");
        assert_eq!(
            t.core_of(LogicalId {
                group: 0,
                number: 3
            }),
            Some(1)
        );
        assert_eq!(
            t.core_of(LogicalId {
                group: 0,
                number: 7
            }),
            Some(5)
        );
        assert!(t.cores[0].smt && !t.cores[5].smt);
    }

    #[test]
    fn hybrid_cache_summary() {
        let t = Topology::parse(&hybrid_2p4e()).unwrap();
        let s: Vec<_> = t
            .cache_summary()
            .iter()
            .map(|c| {
                format!(
                    "{} {}x{}K {}",
                    c.name(),
                    c.count,
                    c.size_bytes / 1024,
                    c.ways_text()
                )
            })
            .collect();
        assert_eq!(
            s,
            vec![
                "L1D 2x48K 12-way",
                "L1D 4x32K 8-way",
                "L1I 2x32K 8-way",
                "L1I 4x64K 8-way",
                "L2 2x1280K 10-way",
                "L2 1x2048K 16-way",
                "L3 1x12288K 12-way",
            ]
        );
    }

    #[test]
    fn non_hybrid_labels() {
        let mut b = Vec::new();
        b.extend(core(true, 0, 0b11));
        b.extend(core(true, 0, 0b1100));
        let t = Topology::parse(&b).unwrap();
        assert!(!t.is_hybrid());
        assert_eq!(t.p_e_counts(), (2, 0));
        assert_eq!(t.core_label(1), "Core 1");
    }

    #[test]
    fn truncated_is_error() {
        let b = hybrid_2p4e();
        assert!(Topology::parse(&b[..b.len() - 3]).is_err());
        assert!(Topology::parse(&[1, 0, 0, 0, 4, 0, 0, 0]).is_err());
    }

    #[test]
    fn empty_buffer() {
        assert_eq!(Topology::parse(&[]).unwrap(), Topology::default());
    }
}
