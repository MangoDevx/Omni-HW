//! Memory checks: XMP/EXPO, channel layout, pressure, capacity and memory test results.

use super::helpers::{finding, guided};
use super::{Category, Context, Level, Outcome, RECENT, Step};
use crate::bios_help::{self, BiosSetting};
use crate::cpuid::Vendor;

/// Speed rating embedded in common part numbers (e.g. "CMW32GX4M2C3200C16" -> 3200,
/// "F5-6000J3038F16G" -> 6000): the first 4-digit number in a plausible DDR range.
pub fn rated_speed_from_part(part: &str) -> Option<u32> {
    part.split(|c: char| !c.is_ascii_digit())
        .filter(|r| r.len() == 4)
        .filter_map(|r| r.parse::<u32>().ok())
        .find(|v| (1600..=9600).contains(v))
}

pub(super) fn memory_speed(ctx: &Context) -> Outcome {
    let Some(sm) = &ctx.info.smbios else {
        return Outcome::Skipped("SMBIOS not available".into());
    };
    let dimms: Vec<_> = sm.populated_memory().collect();
    let Some(d) = dimms.first() else {
        return Outcome::Skipped("No memory modules reported".into());
    };
    let configured = dimms
        .iter()
        .map(|d| d.configured_speed_mts)
        .filter(|&s| s > 0)
        .min()
        .unwrap_or(0);
    if configured == 0 {
        return Outcome::Skipped("Configured memory speed not reported".into());
    }
    let rated = dimms
        .iter()
        .filter_map(|d| rated_speed_from_part(&d.part_number))
        .min()
        .into_iter()
        .chain(dimms.iter().map(|d| d.speed_mts))
        .max()
        .unwrap_or(0);
    // Allow small differences (e.g. 3200 vs 3196 reported).
    if rated <= configured + 50 {
        return Outcome::Pass(format!("Running at its rated {configured} MT/s"));
    }
    let (profile, short) = match ctx.info.cpu.vendor {
        Vendor::Amd => ("EXPO (or DOCP)", "EXPO"),
        _ => ("XMP", "XMP"),
    };
    let mut steps = bios_help::bios_steps(
        ctx.info,
        BiosSetting::MemoryProfile,
        &format!("Enable the {profile} memory profile."),
        short,
    );
    steps.push(Step::new(
        "If the system becomes unstable afterwards, update the BIOS or try the next lower profile.",
    ));
    Outcome::Findings(vec![
        guided(
            Level::Warning,
            Category::Memory,
            format!("Memory runs at {configured} MT/s but is rated for {rated} MT/s"),
            format!(
                "{} ({}) appears to be rated for {rated} MT/s, but it's configured at {configured} MT/s. \
                 The {profile} memory profile is probably not enabled, which costs performance.",
                d.part_number, d.memory_type
            ),
            steps,
        )
        .bios(),
    ])
}

/// Memory channel letter from a slot name such as "DIMM_A1", "ChannelB-DIMM0" or "Controller1-DIMMB2".
pub fn channel_letter(locator: &str) -> Option<char> {
    let upper = locator.to_ascii_uppercase();
    let after = |key: &str| -> Option<char> {
        let i = upper.find(key)? + key.len();
        let c = upper[i..]
            .trim_start_matches([' ', '_', '-'])
            .chars()
            .next()?;
        c.is_ascii_uppercase().then_some(c)
    };
    after("CHANNEL").or_else(|| {
        let c = after("DIMM")?;
        // "DIMM0" has no letter; "DIMMA2" / "DIMM_A1" do.
        let i = upper.find("DIMM")? + 4;
        let rest = upper[i..].trim_start_matches([' ', '_', '-']);
        rest.chars()
            .nth(1)
            .filter(|n| n.is_ascii_digit())
            .map(|_| c)
    })
}

pub(super) fn memory_channels(ctx: &Context) -> Outcome {
    let Some(sm) = &ctx.info.smbios else {
        return Outcome::Skipped("SMBIOS not available".into());
    };
    let dimms: Vec<_> = sm.populated_memory().collect();
    if dimms.is_empty() {
        return Outcome::Skipped("No memory modules reported".into());
    }
    // Soldered memory (laptops) reports a single "module"; only judge socketed DIMMs.
    let socketed = dimms
        .iter()
        .all(|d| d.form_factor == "DIMM" || d.form_factor == "SO-DIMM");
    let mut out = Vec::new();
    if dimms.len() == 1 && socketed && sm.memory_slots >= 2 {
        out.push(finding(
            Level::Warning,
            Category::Memory,
            "Single memory module (single-channel)",
            format!("Only one module is installed ({}), so memory runs in single-channel mode with about half the bandwidth.", dimms[0].locator),
            &[
                "Add a second identical module to run dual-channel.",
                "Use the slots the motherboard manual recommends for two modules (usually A2 and B2).",
            ],
        ));
    }
    let letters: Option<Vec<char>> = dimms
        .iter()
        .map(|d| channel_letter(&d.locator).or_else(|| channel_letter(&d.bank_locator)))
        .collect();
    if let Some(letters) = &letters
        && dimms.len() >= 2
        && letters.iter().all(|&c| c == letters[0])
    {
        out.push(finding(
            Level::Warning,
            Category::Memory,
            "All memory modules are in the same channel",
            format!(
                "Modules are installed in {} - all on channel {}, so they share one channel's bandwidth.",
                dimms.iter().map(|d| d.locator.as_str()).collect::<Vec<_>>().join(", "),
                letters[0]
            ),
            &["Move the modules so each channel has one (check the motherboard manual; usually A2 and B2)."],
        ));
    }
    let first = dimms[0];
    if dimms
        .iter()
        .any(|d| d.size_mb != first.size_mb || d.part_number != first.part_number)
    {
        out.push(finding(
            Level::Info,
            Category::Memory,
            "Mixed memory modules",
            "Installed modules differ in size or part number. Mixed kits often have to run at a lower common speed.",
            &["For best stability and speed, use a matched kit."],
        ));
    }
    if out.is_empty() {
        let layout = match &letters {
            Some(l) => format!(
                "{} modules across channels {}",
                dimms.len(),
                l.iter().map(char::to_string).collect::<Vec<_>>().join(", ")
            ),
            None => format!("{} matching modules", dimms.len()),
        };
        return Outcome::Pass(layout);
    }
    Outcome::Findings(out)
}

pub(super) fn memory_pressure(ctx: &Context) -> Outcome {
    let Some(load) = ctx.recent_avg("mem/load", RECENT) else {
        return Outcome::Skipped("Collecting samples".into());
    };
    let level = match load {
        l if l >= 97.0 => Level::Critical,
        l if l >= 90.0 => Level::Warning,
        _ => return Outcome::Pass(format!("{load:.0} % used on average recently")),
    };
    Outcome::Findings(vec![finding(
        level,
        Category::Memory,
        "Memory is nearly full",
        format!(
            "Physical memory has averaged {load:.0} % used over the last couple of minutes. The system is likely swapping to disk, which causes stutters."
        ),
        &[
            "Close unused applications and browser tabs.",
            "Check System Monitor (or \"top\", sorted by memory with Shift+M) for anything using an unusual amount.",
            "If this is normal for your workload, consider adding more RAM.",
        ],
    )])
}

pub(super) fn memory_capacity(ctx: &Context) -> Outcome {
    let Some(m) = ctx.info.memory else {
        return Outcome::Skipped("Memory size not available".into());
    };
    let gb = m.total_bytes as f64 / (1u64 << 30) as f64;
    if gb >= 15.0 {
        return Outcome::Pass(format!("{gb:.0} GB usable"));
    }
    Outcome::Findings(vec![finding(
        Level::Info,
        Category::Memory,
        format!("Only {gb:.1} GB of usable memory"),
        "Modern games and browsers commonly use more than 8 GB; 16 GB or more is recommended.",
        &["Consider upgrading to at least 16 GB (2 x 8 GB for dual-channel)."],
    )])
}

#[cfg(test)]
mod tests {
    use super::super::testutil::*;
    use super::*;
    use crate::SystemInfo;
    use crate::sensors::{SensorTable, Unit};

    #[test]
    fn part_number_speeds() {
        assert_eq!(rated_speed_from_part("CMW32GX4M2C3200C16"), Some(3200));
        assert_eq!(rated_speed_from_part("F5-6000J3038F16G"), Some(6000));
        assert_eq!(rated_speed_from_part("KF432C16BB/8"), None);
        assert_eq!(rated_speed_from_part(""), None);
    }

    #[test]
    fn channel_letters() {
        assert_eq!(channel_letter("DIMM_A1"), Some('A'));
        assert_eq!(channel_letter("Controller1-DIMMB2"), Some('B'));
        assert_eq!(channel_letter("ChannelB-DIMM0"), Some('B'));
        assert_eq!(channel_letter("P0 CHANNEL A"), Some('A'));
        assert_eq!(channel_letter("DIMM0"), None);
        assert_eq!(channel_letter("BANK 0"), None);
    }

    #[test]
    fn xmp_disabled_is_flagged() {
        let info = system(vec![
            dimm("DIMM_A2", 16384, "F4-3600C16-16GVKC", 2133, 2133),
            dimm("DIMM_B2", 16384, "F4-3600C16-16GVKC", 2133, 2133),
        ]);
        let s = SensorTable::default();
        let o = memory_speed(&ctx(&info, &s));
        assert_eq!(levels(&o), vec![Level::Warning]);
        let Outcome::Findings(f) = o else {
            unreachable!()
        };
        assert!(f[0].title.contains("2133") && f[0].title.contains("3600"));
    }

    #[test]
    fn rated_memory_passes() {
        let info = system(vec![
            dimm(
                "Controller0-DIMMA2",
                16384,
                "CMW32GX4M2C3200C16",
                3200,
                3200,
            ),
            dimm(
                "Controller1-DIMMB2",
                16384,
                "CMW32GX4M2C3200C16",
                3200,
                3200,
            ),
        ]);
        let s = SensorTable::default();
        let c = ctx(&info, &s);
        assert!(matches!(memory_speed(&c), Outcome::Pass(_)));
        assert!(matches!(memory_channels(&c), Outcome::Pass(_)));
    }

    #[test]
    fn channel_problems() {
        let s = SensorTable::default();
        let single = system(vec![dimm("DIMM_A2", 16384, "X", 3200, 3200)]);
        assert_eq!(
            levels(&memory_channels(&ctx(&single, &s))),
            vec![Level::Warning]
        );
        let same = system(vec![
            dimm("DIMM_A1", 8192, "X", 3200, 3200),
            dimm("DIMM_A2", 16384, "Y", 3200, 3200),
        ]);
        assert_eq!(
            levels(&memory_channels(&ctx(&same, &s))),
            vec![Level::Warning, Level::Info]
        );
    }

    #[test]
    fn memory_pressure_levels() {
        let info = SystemInfo::default();
        let mut s = SensorTable::default();
        feed(&mut s, "mem/load", Unit::Percent, &[50.0; 5]);
        assert!(matches!(
            memory_pressure(&ctx(&info, &s)),
            Outcome::Skipped(_)
        ));
        feed(&mut s, "mem/load", Unit::Percent, &[98.0; 200]);
        assert_eq!(
            levels(&memory_pressure(&ctx(&info, &s))),
            vec![Level::Critical]
        );
    }
}
