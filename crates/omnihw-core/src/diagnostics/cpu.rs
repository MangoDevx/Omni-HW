//! CPU checks: Raptor Lake microcode and clocks under load.

use super::helpers::{bios_update_step, finding, guided};
use super::types::MIN_SAMPLES;
use super::{Category, Context, Level, Outcome, Step};
use crate::cpuid::Vendor;

/// Intel's fix for the 13th/14th Gen desktop "Vmin shift" instability ships in microcode 0x12B+.
const RAPTOR_FIXED_MICROCODE: u32 = 0x12B;

pub(super) fn intel_raptor_microcode(ctx: &Context) -> Outcome {
    let c = &ctx.info.cpu;
    // Model B7h is the large Raptor Lake-S die used by affected 65 W+ desktop parts.
    let affected_die = c.vendor == Vendor::Intel && c.family == 6 && c.model == 0xB7;
    let low_power = c
        .brand
        .split_whitespace()
        .any(|w| w.starts_with('i') && w.ends_with('T'));
    if !affected_die || low_power {
        return Outcome::Skipped(
            "Only applies to 13th/14th Gen Intel Core desktop (Raptor Lake-S) CPUs".into(),
        );
    }
    let Some(rev) = ctx.info.microcode else {
        return Outcome::Skipped("Microcode revision not available".into());
    };
    if rev >= RAPTOR_FIXED_MICROCODE {
        return Outcome::Pass(format!(
            "Microcode 0x{rev:X} includes Intel's Vmin shift fix"
        ));
    }
    Outcome::Findings(vec![
        guided(
            Level::Critical,
            Category::Cpu,
            "CPU microcode predates Intel's instability fix",
            format!(
                "{} is running microcode 0x{rev:X}. 13th/14th Gen desktop CPUs can degrade over time \
                 (Vmin shift) without microcode 0x{RAPTOR_FIXED_MICROCODE:X} or newer.",
                c.brand
            ),
            vec![
                bios_update_step(
                    ctx,
                    "Update the motherboard BIOS to a version with microcode 0x12B or newer (check the board vendor's support page).",
                ),
                Step::new(
                    "After updating, load the BIOS's \"Intel Default Settings\" profile instead of unlimited power limits.",
                ),
                Step::new(
                    "If you see crashes or shader-compilation errors in games, contact Intel about the extended warranty.",
                ),
            ],
        )
        .bios(),
    ])
}

/// Under sustained load, the average clock should stay at or above base clock.
pub(super) fn cpu_throttling(ctx: &Context) -> Outcome {
    let (Some(base), Some(load), Some(clock)) = (
        ctx.info.base_mhz,
        ctx.sensors.get("cpu/load/total"),
        ctx.sensors.get("cpu/clock/avg"),
    ) else {
        return Outcome::Skipped("Needs base clock and live CPU sensors".into());
    };
    // Both histories are appended on every sample, so align them from the newest end.
    let (sum, busy) = load
        .history
        .iter()
        .rev()
        .zip(clock.history.iter().rev())
        .filter(|(l, _)| **l >= 70.0)
        .fold((0.0, 0usize), |(s, n), (_, &c)| (s + f64::from(c), n + 1));
    if busy < MIN_SAMPLES {
        return Outcome::Skipped(
            "Needs some time with the CPU under heavy load (e.g. a game or render) to judge".into(),
        );
    }
    let mean = sum / busy as f64;
    if mean >= f64::from(base) * 0.9 {
        return Outcome::Pass(format!(
            "Averaged {mean:.0} MHz under load (base {base} MHz)"
        ));
    }
    Outcome::Findings(vec![finding(
        Level::Warning,
        Category::Cpu,
        "CPU slows below base clock under load",
        format!(
            "Under heavy load the CPU averaged {mean:.0} MHz, below its {base} MHz base clock. This usually means thermal or power throttling."
        ),
        &[
            "Check that the CPU cooler is seated properly and its fans/pump are running.",
            "Clean dust from heatsinks and filters; improve case airflow.",
            "On laptops, use the plugged-in performance mode and keep vents clear.",
            "Check the BIOS for overly low power limits (PL1/PL2 or PPT).",
        ],
    )])
}

#[cfg(test)]
mod tests {
    use super::super::testutil::*;
    use super::*;
    use crate::SystemInfo;
    use crate::cpuid::CpuInfo;
    use crate::sensors::{SensorTable, Unit};

    #[test]
    fn raptor_microcode() {
        let s = SensorTable::default();
        let mut info = SystemInfo {
            cpu: CpuInfo {
                vendor: Vendor::Intel,
                family: 6,
                model: 0xB7,
                brand: "13th Gen Intel(R) Core(TM) i9-13900K".into(),
                ..Default::default()
            },
            microcode: Some(0x11F),
            ..Default::default()
        };
        assert_eq!(
            levels(&intel_raptor_microcode(&ctx(&info, &s))),
            vec![Level::Critical]
        );
        info.microcode = Some(0x12B);
        assert!(matches!(
            intel_raptor_microcode(&ctx(&info, &s)),
            Outcome::Pass(_)
        ));
        info.cpu.brand = "13th Gen Intel(R) Core(TM) i9-13900T".into();
        info.microcode = Some(0x11F);
        assert!(matches!(
            intel_raptor_microcode(&ctx(&info, &s)),
            Outcome::Skipped(_)
        ));
        info.cpu.model = 0x97;
        assert!(matches!(
            intel_raptor_microcode(&ctx(&info, &s)),
            Outcome::Skipped(_)
        ));
    }

    #[test]
    fn throttling_needs_load_and_flags_low_clocks() {
        let info = SystemInfo {
            base_mhz: Some(3500),
            ..Default::default()
        };
        let mut s = SensorTable::default();
        feed(&mut s, "cpu/load/total", Unit::Percent, &[5.0; 20]);
        feed(&mut s, "cpu/clock/avg", Unit::Mhz, &[4800.0; 20]);
        assert!(matches!(
            cpu_throttling(&ctx(&info, &s)),
            Outcome::Skipped(_)
        ));
        feed(&mut s, "cpu/load/total", Unit::Percent, &[95.0; 15]);
        feed(&mut s, "cpu/clock/avg", Unit::Mhz, &[2400.0; 15]);
        assert_eq!(
            levels(&cpu_throttling(&ctx(&info, &s))),
            vec![Level::Warning]
        );
    }
}
