//! CPU topology and caches from `/sys/devices/system/cpu`.

use super::{Root, parse_cpu_list};
use crate::cpuid::CacheKind;
use crate::topology::{CacheInstance, Core, LogicalId, Topology};
use std::collections::{BTreeMap, BTreeSet};

const CPU: &str = "/sys/devices/system/cpu";

/// Linux CPU number as the Windows-style (group, number) pair the rest of the app uses:
/// 64 logical processors per group.
pub fn logical_id(cpu: u32) -> LogicalId {
    LogicalId {
        group: (cpu / 64) as u16,
        number: (cpu % 64) as u8,
    }
}

/// Inverse of [`logical_id`].
pub fn cpu_number(id: LogicalId) -> u32 {
    id.group as u32 * 64 + id.number as u32
}

/// Online CPUs: the `online` list, else every `cpuN` directory.
pub fn online_cpus(root: &Root) -> Vec<u32> {
    if let Some(s) = root.read_nonempty(&format!("{CPU}/online")) {
        return parse_cpu_list(&s);
    }
    root.list(CPU)
        .iter()
        .filter_map(|n| n.strip_prefix("cpu")?.parse().ok())
        .collect()
}

/// "48K", "1024K", "32M" (sysfs cache sizes).
pub fn parse_cache_size(s: &str) -> Option<u64> {
    let s = s.trim();
    let (num, mult) = match s.chars().last()? {
        'K' | 'k' => (&s[..s.len() - 1], 1024),
        'M' | 'm' => (&s[..s.len() - 1], 1024 * 1024),
        'G' | 'g' => (&s[..s.len() - 1], 1024 * 1024 * 1024),
        _ => (s, 1),
    };
    Some(num.trim().parse::<u64>().ok()? * mult)
}

/// Efficiency class per CPU on hybrids (higher = faster): Intel's `cpu_core` / `cpu_atom` PMU
/// lists, else distinct `cpu_capacity` values ranked. Empty when every core is the same.
fn efficiency_classes(root: &Root, cpus: &[u32]) -> BTreeMap<u32, u8> {
    let mut out = BTreeMap::new();
    if let (Some(p), Some(e)) = (
        root.read_nonempty("/sys/devices/cpu_core/cpus"),
        root.read_nonempty("/sys/devices/cpu_atom/cpus"),
    ) {
        out.extend(parse_cpu_list(&p).into_iter().map(|c| (c, 1)));
        out.extend(parse_cpu_list(&e).into_iter().map(|c| (c, 0)));
        return out;
    }
    let caps: BTreeMap<u32, u64> = cpus
        .iter()
        .filter_map(|&c| Some((c, root.read_u64(&format!("{CPU}/cpu{c}/cpu_capacity"))?)))
        .collect();
    let distinct: BTreeSet<u64> = caps.values().copied().collect();
    if distinct.len() > 1 {
        for (c, cap) in caps {
            let rank = distinct.iter().position(|&d| d == cap).unwrap_or(0);
            out.insert(c, rank.min(u8::MAX as usize) as u8);
        }
    }
    out
}

fn cache_kind(s: &str) -> Option<CacheKind> {
    Some(match s {
        "Data" => CacheKind::Data,
        "Instruction" => CacheKind::Instruction,
        "Unified" => CacheKind::Unified,
        _ => return None,
    })
}

pub fn topology(root: &Root) -> Option<Topology> {
    let cpus = online_cpus(root);
    if cpus.is_empty() {
        return None;
    }
    let classes = efficiency_classes(root, &cpus);
    let mut packages = BTreeSet::new();
    let mut dies = BTreeSet::new();
    // Threads grouped by core: (package, die, core id) -> CPUs.
    let mut cores: BTreeMap<(i64, i64, i64), Vec<u32>> = BTreeMap::new();
    let mut caches: BTreeMap<(u8, u8, Vec<u32>), CacheInstance> = BTreeMap::new();
    for &c in &cpus {
        let base = format!("{CPU}/cpu{c}");
        let topo = |f: &str| root.read_i64(&format!("{base}/topology/{f}"));
        let package = topo("physical_package_id").unwrap_or(0);
        let die = topo("die_id").unwrap_or(0);
        // Without a core id, treat each CPU as its own core.
        let core = topo("core_id").unwrap_or(-1 - c as i64);
        packages.insert(package);
        dies.insert((package, die));
        cores.entry((package, die, core)).or_default().push(c);

        for idx in root.list(&format!("{base}/cache")) {
            if !idx.starts_with("index") {
                continue;
            }
            let p = format!("{base}/cache/{idx}");
            let (Some(level), Some(kind)) = (
                root.read_u64(&format!("{p}/level")),
                root.read(&format!("{p}/type"))
                    .as_deref()
                    .and_then(cache_kind),
            ) else {
                continue;
            };
            let Some(size_bytes) = root
                .read(&format!("{p}/size"))
                .and_then(|s| parse_cache_size(&s))
            else {
                continue;
            };
            let mut shared = root
                .read(&format!("{p}/shared_cpu_list"))
                .map(|s| parse_cpu_list(&s))
                .unwrap_or_else(|| vec![c]);
            shared.retain(|x| cpus.contains(x));
            let kind_key = match kind {
                CacheKind::Data => 0,
                CacheKind::Instruction => 1,
                CacheKind::Unified => 2,
            };
            caches
                .entry((level as u8, kind_key, shared.clone()))
                .or_insert_with(|| CacheInstance {
                    level: level as u8,
                    kind,
                    size_bytes,
                    associativity: root
                        .read_u64(&format!("{p}/ways_of_associativity"))
                        .map(|w| w.min(0xFF) as u8)
                        .unwrap_or(0),
                    line_size: root
                        .read_u64(&format!("{p}/coherency_line_size"))
                        .map(|l| l.min(u16::MAX as u64) as u16)
                        .unwrap_or(0),
                    logical: shared.iter().map(|&x| logical_id(x)).collect(),
                });
        }
    }

    let mut cores: Vec<Vec<u32>> = cores.into_values().collect();
    cores.sort_by_key(|t| t.iter().min().copied());
    let cores = cores
        .into_iter()
        .map(|threads| Core {
            efficiency_class: classes.get(&threads[0]).copied().unwrap_or(0),
            smt: threads.len() > 1,
            logical: threads.iter().map(|&x| logical_id(x)).collect(),
        })
        .collect();
    // Cache instances in CPU order, like Windows lists them.
    let mut caches: Vec<CacheInstance> = caches.into_values().collect();
    caches.sort_by_key(|c| (c.level, c.logical.first().copied()));

    let numa_nodes = root
        .list("/sys/devices/system/node")
        .iter()
        .filter(|n| {
            n.strip_prefix("node")
                .is_some_and(|d| d.parse::<u32>().is_ok())
        })
        .count()
        .max(1);
    Some(Topology {
        packages: packages.len(),
        dies: dies.len(),
        numa_nodes,
        groups: (cpus.iter().max().copied().unwrap_or(0) / 64 + 1) as usize,
        cores,
        caches,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::linux::Fixture;

    /// A 2P (SMT) + 2E hybrid: CPUs 0-1 = P0, 2-3 = P1, 4 = E0, 5 = E1.
    fn hybrid() -> Fixture {
        let f = Fixture::new();
        f.file("sys/devices/system/cpu/online", "0-5\n")
            .file("sys/devices/cpu_core/cpus", "0-3\n")
            .file("sys/devices/cpu_atom/cpus", "4-5\n")
            .dir("sys/devices/system/node/node0");
        let core_ids = [0, 0, 4, 4, 8, 9];
        for (cpu, core) in core_ids.iter().enumerate() {
            let b = format!("sys/devices/system/cpu/cpu{cpu}");
            f.file(&format!("{b}/topology/physical_package_id"), "0")
                .file(&format!("{b}/topology/die_id"), "0")
                .file(&format!("{b}/topology/core_id"), core.to_string());
            let l1 = if cpu < 4 {
                format!("{}-{}", cpu / 2 * 2, cpu / 2 * 2 + 1)
            } else {
                cpu.to_string()
            };
            let caches = [
                ("index0", "1", "Data", "48K", "12", l1.as_str()),
                ("index1", "1", "Instruction", "32K", "8", l1.as_str()),
                ("index2", "3", "Unified", "30M", "12", "0-5"),
            ];
            for (idx, level, kind, size, ways, shared) in caches {
                let p = format!("{b}/cache/{idx}");
                f.file(&format!("{p}/level"), level)
                    .file(&format!("{p}/type"), kind)
                    .file(&format!("{p}/size"), size)
                    .file(&format!("{p}/ways_of_associativity"), ways)
                    .file(&format!("{p}/coherency_line_size"), "64")
                    .file(&format!("{p}/shared_cpu_list"), shared);
            }
        }
        f
    }

    #[test]
    fn hybrid_topology() {
        let f = hybrid();
        let t = topology(&f.root()).unwrap();
        assert_eq!((t.packages, t.dies, t.numa_nodes, t.groups), (1, 1, 1, 1));
        assert_eq!(t.cores.len(), 4);
        assert_eq!(t.logical_count(), 6);
        assert!(t.is_hybrid());
        assert_eq!(t.p_e_counts(), (2, 2));
        assert!(t.cores[0].smt && !t.cores[3].smt);
        assert_eq!(t.core_label(2), "E-core 0");
        let s = t.cache_summary();
        assert_eq!(s.len(), 3);
        assert_eq!((s[0].level, s[0].kind, s[0].count), (1, CacheKind::Data, 4));
        assert_eq!(s[2].size_bytes, 30 * 1024 * 1024);
        assert_eq!(s[2].count, 1);
    }

    #[test]
    fn ids_round_trip() {
        assert_eq!(
            logical_id(70),
            LogicalId {
                group: 1,
                number: 6
            }
        );
        assert_eq!(cpu_number(logical_id(70)), 70);
    }

    #[test]
    fn sizes() {
        assert_eq!(parse_cache_size("48K"), Some(48 * 1024));
        assert_eq!(parse_cache_size("32M"), Some(32 << 20));
        assert_eq!(parse_cache_size("512"), Some(512));
    }
}
