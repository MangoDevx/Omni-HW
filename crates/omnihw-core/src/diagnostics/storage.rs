//! Storage checks: free space, SMART health, temperature, partitioning and link speeds.

use super::helpers::{board_query, distro, finding, guided, plural, summarize};
use super::{Category, Context, Level, Outcome, RECENT, Step};
use crate::format_bytes;
use crate::storage::{BusType, PartitionStyle, sata_gen_speed};

pub(super) fn drive_free_space(ctx: &Context) -> Outcome {
    let mut out = Vec::new();
    let mut n = 0;
    for v in ctx.info.drives.iter().flat_map(|d| &d.volumes) {
        if v.total_bytes == 0 {
            continue;
        }
        n += 1;
        let free = v.free_bytes as f64 / v.total_bytes as f64;
        let level = match free {
            f if f < 0.05 => Level::Critical,
            f if f < 0.10 => Level::Warning,
            _ => continue,
        };
        let system = v.mount == "/";
        out.push(guided(
            level,
            Category::Storage,
            format!("{} is almost full", v.mount),
            format!(
                "{} free of {} ({:.0} %).{}",
                format_bytes(v.free_bytes),
                format_bytes(v.total_bytes),
                free * 100.0,
                if system {
                    " A full root file system can break updates and even logging in."
                } else {
                    ""
                }
            ),
            vec![
                Step::new(
                    "See what uses the space with Disk Usage Analyzer (baobab), or \"du -xh --max-depth=1 <folder> | sort -h\".",
                ),
                Step::new(format!(
                    "Clear the package cache ({}) and remove old snaps, flatpaks and kernels you no longer use.",
                    distro(ctx).clean_cache()
                )),
                Step::new("Uninstall unused games/apps or move them to another drive."),
                Step::new("SSDs also slow down when nearly full; keep 10-20 % free."),
            ],
        ));
    }
    if n == 0 {
        return Outcome::Skipped("No volumes found".into());
    }
    if out.is_empty() {
        return Outcome::Pass(format!("All {n} volumes have at least 10 % free"));
    }
    Outcome::Findings(out)
}

pub(super) fn drive_health(ctx: &Context) -> Outcome {
    let mut out = Vec::new();
    let mut seen = Vec::new();
    for d in &ctx.info.drives {
        let Some(h) = &d.nvme_health else { continue };
        let name = d.name();
        seen.push(format!("{name}: {} %", h.health_pct()));
        let warnings = h.warnings();
        if !warnings.is_empty() || h.available_spare_pct < h.spare_threshold_pct {
            out.push(finding(
                Level::Critical,
                Category::Storage,
                format!("{name} reports a critical health warning"),
                format!(
                    "The drive's SMART log flags: {}.",
                    if warnings.is_empty() {
                        "spare capacity below threshold".to_string()
                    } else {
                        warnings.join(", ")
                    }
                ),
                &[
                    "Back up important data from this drive now.",
                    "Plan to replace the drive.",
                ],
            ));
            continue;
        }
        let level = match h.health_pct() {
            p if p < 50 => Some(Level::Critical),
            p if p < 80 => Some(Level::Warning),
            _ => None,
        };
        if let Some(level) = level {
            out.push(finding(
                level,
                Category::Storage,
                format!("{name} is {} % worn", h.percentage_used),
                format!(
                    "The drive estimates {} % of its rated write endurance is used ({:.1} TB written).",
                    h.percentage_used,
                    h.data_written_bytes as f64 / 1e12
                ),
                &["Keep backups current.", "Plan a replacement as health approaches 10-20 %."],
            ));
        }
        if h.media_errors > 0 {
            out.push(finding(
                Level::Warning,
                Category::Storage,
                format!("{name} has recorded media errors"),
                format!(
                    "{} unrecovered data integrity errors are logged.",
                    h.media_errors
                ),
                &[
                    "Back up important data and watch whether the count grows.",
                    "Check the manufacturer's tool for firmware updates.",
                ],
            ));
        }
    }
    let has_nvme = ctx.info.drives.iter().any(|d| d.bus == BusType::Nvme);
    summarize(
        out,
        &seen,
        if has_nvme {
            "The NVMe health log needs root (Read as root on the Storage page)"
        } else {
            "No NVMe drives (SATA drives are covered by the SATA SMART check)"
        },
    )
}

/// SATA drives that can carry ATA SMART (not NVMe, USB, virtual or card readers).
fn sata_like(bus: BusType) -> bool {
    !matches!(
        bus,
        BusType::Nvme
            | BusType::Usb
            | BusType::Virtual
            | BusType::Sd
            | BusType::Mmc
            | BusType::Spaces
    )
}

pub(super) fn sata_smart(ctx: &Context) -> Outcome {
    let mut out = Vec::new();
    let mut seen = Vec::new();
    for d in &ctx.info.drives {
        let Some(s) = &d.ata_smart else { continue };
        let name = d.name();
        let mut summary = vec![];
        if let Some(life) = s.life_remaining_pct() {
            summary.push(format!("{life} % life left"));
        }
        if let Some(h) = s.power_on_hours() {
            summary.push(format!("{h} h"));
        }
        seen.push(if summary.is_empty() {
            format!("{name}: OK")
        } else {
            format!("{name}: {}", summary.join(", "))
        });

        let realloc = s.reallocated_sectors().unwrap_or(0);
        let pending = s.pending_sectors().unwrap_or(0);
        let uncorrectable = s.uncorrectable().unwrap_or(0);
        if realloc + pending + uncorrectable > 0 {
            let level = if pending > 10 || uncorrectable > 10 {
                Level::Critical
            } else {
                Level::Warning
            };
            out.push(finding(
                level,
                Category::Storage,
                format!("{name} has bad or unstable sectors"),
                format!(
                    "SMART reports {realloc} reallocated, {pending} pending and {uncorrectable} uncorrectable sectors. \
                     These counts normally stay at 0; rising values mean the drive's media is failing."
                ),
                &[
                    "Back up important data from this drive now.",
                    "Check again in a few days; if any count grows, replace the drive.",
                    "Run the drive maker's diagnostic tool for an extended self-test.",
                ],
            ));
        }
        if let Some(life) = s.life_remaining_pct() {
            let level = match life {
                l if l < 10 => Some(Level::Critical),
                l if l < 20 => Some(Level::Warning),
                _ => None,
            };
            if let Some(level) = level {
                out.push(finding(
                    level,
                    Category::Storage,
                    format!("{name} is nearly worn out ({life} % life left)"),
                    format!("The SSD estimates {life} % of its rated write endurance remains."),
                    &["Keep backups current.", "Plan to replace the drive soon."],
                ));
            }
        }
        if let Some(crc) = s.crc_errors().filter(|&c| c > 0) {
            out.push(finding(
                Level::Info,
                Category::Storage,
                format!("{name} has recorded cable (CRC) errors"),
                format!(
                    "{crc} interface CRC errors are logged. These come from the SATA cable or port, not the drive itself; \
                     the count never resets, so only an increase means an ongoing problem."
                ),
                &[
                    "Reseat or replace the SATA data cable, or move it to another SATA port.",
                    "Watch whether the count keeps growing.",
                ],
            ));
        }
    }
    let has_sata = ctx.info.drives.iter().any(|d| sata_like(d.bus));
    summarize(
        out,
        &seen,
        if has_sata {
            "SATA SMART data needs root (Read as root on the Storage page)"
        } else {
            "No SATA drives"
        },
    )
}

pub(super) fn drive_temperature(ctx: &Context) -> Outcome {
    let mut out = Vec::new();
    let mut seen = Vec::new();
    for d in &ctx.info.drives {
        let Some(t) = ctx.recent_avg(&format!("{}/temp", d.sensor_prefix()), RECENT) else {
            continue;
        };
        let name = d.name();
        seen.push(format!("{name}: {t:.0} °C"));
        let level = match t {
            t if t >= 80.0 => Level::Critical,
            t if t >= 70.0 => Level::Warning,
            _ => continue,
        };
        out.push(finding(
            level,
            Category::Storage,
            format!("{name} is running hot"),
            format!("Drive temperature has averaged {t:.0} °C. NVMe drives typically throttle around 70-80 °C."),
            &[
                "Use the motherboard's M.2 heatsink (remove its protective film) or add one.",
                "Make sure the drive isn't directly under a hot GPU without airflow.",
            ],
        ));
    }
    summarize(out, &seen, "No drive temperature sensor")
}

pub(super) fn drive_setup(ctx: &Context) -> Outcome {
    let drives: Vec<_> = ctx
        .info
        .drives
        .iter()
        .filter(|d| d.size_bytes > 0 && d.layout.is_some())
        .collect();
    if drives.is_empty() {
        return Outcome::Skipped("Partition information not available".into());
    }
    let mut out = Vec::new();
    for d in &drives {
        let Some(l) = d.layout else { continue };
        let name = d.name();
        if l.style == PartitionStyle::Raw {
            out.push(guided(
                Level::Warning,
                Category::Storage,
                format!("{name} isn't set up yet"),
                format!(
                    "This {} drive has no partition table or file system yet, so nothing can be stored on it. This is normal for a brand-new drive.",
                    format_bytes(d.size_bytes)
                ),
                vec![
                    Step::new(format!(
                        "Open Disks (gnome-disks) or GParted, select {name} and create a GPT partition table. Only do this for a new or empty drive."
                    )),
                    Step::new(
                        "Create a partition on it, formatted as ext4 (or exFAT/NTFS to share it with Windows), and mount it.",
                    ),
                ],
            ));
            continue;
        }
        let threshold = (d.size_bytes / 10).max(10 << 30);
        if l.unallocated_bytes >= threshold {
            let unused_drive = d.volumes.is_empty() && l.partitions <= 1;
            out.push(guided(
                if unused_drive { Level::Warning } else { Level::Info },
                Category::Storage,
                format!(
                    "{name} has {} of unallocated space",
                    format_bytes(l.unallocated_bytes)
                ),
                format!(
                    "{} of this {} drive isn't part of any volume, so it can't be used.",
                    format_bytes(l.unallocated_bytes),
                    format_bytes(d.size_bytes)
                ),
                vec![Step::new(
                    "Open Disks (gnome-disks) or GParted, select the free space and create a partition, or grow the neighbouring one into it.",
                )],
            ));
        }
    }
    if out.is_empty() {
        return Outcome::Pass(format!(
            "{} set up",
            plural(drives.len(), "drive is", "drives are")
        ));
    }
    Outcome::Findings(out)
}

pub(super) fn storage_link(ctx: &Context) -> Outcome {
    let manual_query = |what: &str| {
        board_query(
            ctx,
            &format!("manual {what}"),
            &format!("motherboard manual {what}"),
        )
    };
    let mut out = Vec::new();
    let mut seen = Vec::new();
    for l in &ctx.info.checkup.nvme_links {
        let ((cg, cw), (mg, mw)) = (l.current, l.max);
        if cg == 0 || cw == 0 || mg == 0 || mw == 0 {
            continue;
        }
        seen.push(format!("{}: PCIe {cg}.0 x{cw}", l.device));
        let slot_steps = |what: &str| {
            vec![
                Step::search(
                    format!(
                        "Check the motherboard manual for which M.2 slot is wired to the CPU at full speed (usually the top one) and move the drive there{what}."
                    ),
                    manual_query("M.2 slot PCIe lanes"),
                ),
                Step::new(
                    "Some M.2 slots share lanes with SATA ports or a PCIe slot; the manual lists what disables or slows what.",
                ),
            ]
        };
        if cw < mw {
            out.push(guided(
                Level::Warning,
                Category::Storage,
                format!("{} runs at PCIe x{cw} instead of x{mw}", l.device),
                format!(
                    "The drive supports {mw} PCIe lanes but its link uses {cw}, roughly {} its maximum speed. The M.2 slot is probably wired with fewer lanes or shares them with another device.",
                    if cw * 2 <= mw { "halving" } else { "reducing" }
                ),
                slot_steps(""),
            ));
        } else if cg < mg {
            out.push(guided(
                Level::Info,
                Category::Storage,
                format!("{} runs at PCIe {cg}.0 (supports {mg}.0)", l.device),
                format!(
                    "The drive supports PCIe {mg}.0 but the link runs at {cg}.0, which caps sequential speed. Common when the drive sits in a slot wired to the chipset or an older CPU generation."
                ),
                slot_steps(" if you want the full speed"),
            ));
        }
    }
    for d in &ctx.info.drives {
        let Some(s) = d.sata_link else { continue };
        seen.push(format!(
            "{}: SATA {}",
            d.name(),
            sata_gen_speed(s.current_gen)
        ));
        if s.current_gen < s.max_gen {
            out.push(guided(
                Level::Warning,
                Category::Storage,
                format!(
                    "{} links at {} instead of {}",
                    d.name(),
                    sata_gen_speed(s.current_gen),
                    sata_gen_speed(s.max_gen)
                ),
                format!(
                    "The drive supports SATA {} but negotiated {}. For an SSD that halves its speed or worse.",
                    sata_gen_speed(s.max_gen),
                    sata_gen_speed(s.current_gen)
                ),
                vec![
                    Step::search(
                        "Connect it to a SATA 6 Gb/s port: some boards also have slower ports or ports on an extra controller (see the manual).",
                        manual_query("SATA ports"),
                    ),
                    Step::new("Replace the SATA data cable (use one rated SATA III / 6 Gb/s)."),
                ],
            ));
        }
    }
    summarize(
        out,
        &seen,
        "No NVMe link data, and SATA link speed needs root (Read as root on the Storage page)",
    )
}

#[cfg(test)]
mod tests {
    use super::super::testutil::*;
    use super::*;
    use crate::SystemInfo;
    use crate::sensors::{SensorTable, Unit};
    use crate::storage::{DiskLayout, Drive, NvmeHealth, Volume};

    #[test]
    fn storage_checks() {
        let mut info = SystemInfo::default();
        info.drives.push(Drive {
            index: 0,
            model: "Test SSD".into(),
            bus: BusType::Nvme,
            volumes: vec![
                Volume {
                    mount: "C:".into(),
                    total_bytes: 1000,
                    free_bytes: 30,
                    ..Default::default()
                },
                Volume {
                    mount: "D:".into(),
                    total_bytes: 1000,
                    free_bytes: 500,
                    ..Default::default()
                },
            ],
            nvme_health: Some(NvmeHealth {
                percentage_used: 35,
                available_spare_pct: 100,
                spare_threshold_pct: 10,
                media_errors: 2,
                ..Default::default()
            }),
            ..Default::default()
        });
        let mut s = SensorTable::default();
        let c = ctx(&info, &s);
        assert_eq!(levels(&drive_free_space(&c)), vec![Level::Critical]);
        assert_eq!(
            levels(&drive_health(&c)),
            vec![Level::Warning, Level::Warning]
        );
        feed(&mut s, "disk/0/temp", Unit::Celsius, &[74.0; 30]);
        assert_eq!(
            levels(&drive_temperature(&ctx(&info, &s))),
            vec![Level::Warning]
        );
    }

    #[test]
    fn sata_smart_findings() {
        use crate::smart::{parse_ata_smart, tests::sector};
        let s = SensorTable::default();
        let drive = |smart| Drive {
            index: 1,
            model: "Test SATA SSD".into(),
            bus: BusType::Sata,
            ata_smart: smart,
            ..Default::default()
        };
        let healthy = parse_ata_smart(&sector(&[
            (5, 100, 100, 0),
            (9, 99, 99, 1200),
            (231, 95, 95, 0),
        ]));
        let mut info = SystemInfo {
            drives: vec![drive(healthy)],
            ..Default::default()
        };
        let Outcome::Pass(p) = sata_smart(&ctx(&info, &s)) else {
            panic!("expected pass")
        };
        assert!(p.contains("95 % life left") && p.contains("1200 h"));

        let bad = parse_ata_smart(&sector(&[
            (5, 90, 90, 8),
            (197, 100, 100, 25),
            (199, 100, 100, 3),
            (231, 8, 8, 0),
        ]));
        info.drives = vec![drive(bad)];
        assert_eq!(
            levels(&sata_smart(&ctx(&info, &s))),
            vec![Level::Critical, Level::Critical, Level::Info]
        );

        let worn = parse_ata_smart(&sector(&[(5, 100, 100, 1), (177, 15, 15, 0)]));
        info.drives = vec![drive(worn)];
        assert_eq!(
            levels(&sata_smart(&ctx(&info, &s))),
            vec![Level::Warning, Level::Warning]
        );

        info.drives = vec![drive(None)];
        let Outcome::Skipped(why) = sata_smart(&ctx(&info, &s)) else {
            panic!("expected skip")
        };
        assert!(why.contains("root"));
        info.drives[0].bus = BusType::Nvme;
        assert!(
            matches!(sata_smart(&ctx(&info, &s)), Outcome::Skipped(w) if w == "No SATA drives")
        );
    }

    #[test]
    fn drive_setup_checks() {
        const G: u64 = 1 << 30;
        let s = SensorTable::default();
        let mut info = SystemInfo {
            drives: vec![
                Drive {
                    index: 0,
                    model: "Big HDD".into(),
                    size_bytes: 2000 * G,
                    ssd: Some(false),
                    volumes: vec![Volume {
                        mount: "C:".into(),
                        ..Default::default()
                    }],
                    layout: Some(DiskLayout {
                        style: PartitionStyle::Gpt,
                        partitions: 3,
                        unallocated_bytes: G / 100,
                    }),
                    ..Default::default()
                },
                Drive {
                    index: 1,
                    model: "New SSD".into(),
                    size_bytes: 1000 * G,
                    ssd: Some(true),
                    layout: Some(DiskLayout {
                        style: PartitionStyle::Raw,
                        partitions: 0,
                        unallocated_bytes: 1000 * G,
                    }),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let c = ctx(&info, &s);
        let o = drive_setup(&c);
        assert_eq!(levels(&o), vec![Level::Warning]);
        let Outcome::Findings(f) = o else {
            unreachable!()
        };
        assert!(f[0].title.contains("New SSD"));

        info.drives[1].layout = Some(DiskLayout {
            style: PartitionStyle::Gpt,
            partitions: 2,
            unallocated_bytes: 500 * G,
        });
        info.drives[0].ssd = Some(true);
        let c = ctx(&info, &s);
        assert_eq!(levels(&drive_setup(&c)), vec![Level::Info]);
    }
}
