//! Crash checks from the system journal (read for the Event Log page, see
//! [`crate::eventlog`]): kernel crashes and hangs, hardware errors, GPU hangs, disk and file
//! system errors, out-of-memory kills and overheating in the last 30 days.

use super::helpers::{finding, plural};
use super::{Category, Context, Level, Outcome};
use crate::eventlog::{EventCategory, EventLevel, LogEntry, VIEW_DAYS};

/// Up to three distinct logged messages, for a finding's detail.
fn examples(events: &[&LogEntry]) -> String {
    let mut seen: Vec<&str> = Vec::new();
    for e in events {
        let text = if e.detail.is_empty() {
            e.title
        } else {
            &e.detail
        };
        if !seen.contains(&text) {
            seen.push(text);
        }
        if seen.len() == 3 {
            break;
        }
    }
    seen.join(" | ")
}

/// The events of `category`, or the outcome when there are none or the journal wasn't read.
fn events_of<'a>(ctx: &Context<'a>, category: EventCategory) -> Result<Vec<&'a LogEntry>, Outcome> {
    let Some(log) = ctx.events else {
        return Err(Outcome::Skipped(
            "The system journal hasn't been read yet".into(),
        ));
    };
    if log.entries.is_empty() && !log.errors.is_empty() {
        return Err(Outcome::Skipped(format!(
            "Couldn't read the system journal: {}",
            log.errors.join("; ")
        )));
    }
    let found: Vec<&LogEntry> = log
        .entries
        .iter()
        .filter(|e| e.category == category)
        .collect();
    if found.is_empty() {
        Err(Outcome::Pass(format!("None in the last {VIEW_DAYS} days")))
    } else {
        Ok(found)
    }
}

const EVENT_LOG_STEP: &str =
    "Open the Event Log page and pick this category for each logged message and when it happened.";

pub(super) fn kernel_crashes(ctx: &Context) -> Outcome {
    let found = match events_of(ctx, EventCategory::KernelCrash) {
        Ok(f) => f,
        Err(o) => return o,
    };
    let critical = found.iter().any(|e| e.level == EventLevel::Critical);
    Outcome::Findings(vec![finding(
        if critical {
            Level::Critical
        } else {
            Level::Warning
        },
        Category::Crashes,
        format!(
            "{} in the last {VIEW_DAYS} days",
            plural(
                found.len(),
                "kernel crash or hang",
                "kernel crashes or hangs"
            )
        ),
        examples(&found),
        &[
            EVENT_LOG_STEP,
            "Install all updates (sudo apt update && sudo apt full-upgrade): kernel and driver fixes often cure crashes.",
            "If you overclock, undervolt or use XMP/EXPO, go back to stock settings and see whether the crashes stop.",
            "Test the memory: choose Memtest86+ in the boot menu (GRUB), or run MemTest86 from a USB stick, for a few passes.",
        ],
    )])
}

pub(super) fn hardware_errors(ctx: &Context) -> Outcome {
    let found = match events_of(ctx, EventCategory::HardwareError) {
        Ok(f) => f,
        Err(o) => return o,
    };
    let uncorrected = found.iter().any(|e| e.level >= EventLevel::Error);
    Outcome::Findings(vec![finding(
        if uncorrected {
            Level::Critical
        } else {
            Level::Warning
        },
        Category::Crashes,
        format!(
            "{} logged in the last {VIEW_DAYS} days{}",
            plural(found.len(), "hardware error", "hardware errors"),
            if uncorrected { "" } else { " (all corrected)" }
        ),
        examples(&found),
        &[
            EVENT_LOG_STEP,
            "Memory (EDAC) and CPU (machine check) errors under load usually mean unstable settings: turn off XMP/EXPO and any overclock or undervolt, then test again.",
            "PCIe errors point at a card, riser or slot: reseat the device, try another slot, and update the BIOS.",
            "Errors that continue at stock settings mean failing hardware (memory module, CPU or board): test each memory module on its own.",
        ],
    )])
}

pub(super) fn gpu_hangs(ctx: &Context) -> Outcome {
    let found = match events_of(ctx, EventCategory::GpuTimeout) {
        Ok(f) => f,
        Err(o) => return o,
    };
    let level = if found.len() >= 3 || found.iter().any(|e| e.level == EventLevel::Critical) {
        Level::Critical
    } else {
        Level::Warning
    };
    Outcome::Findings(vec![finding(
        level,
        Category::Crashes,
        format!(
            "{} in the last {VIEW_DAYS} days",
            plural(found.len(), "GPU hang or reset", "GPU hangs or resets")
        ),
        examples(&found),
        &[
            EVENT_LOG_STEP,
            "Update the graphics driver (NVIDIA: the newest recommended driver in Additional Drivers; AMD/Intel: the newest kernel and Mesa).",
            "Remove any GPU overclock or undervolt.",
            "Check the GPU's power cables and the power supply; a GPU that drops off the bus often lacks stable power.",
        ],
    )])
}

pub(super) fn disk_errors(ctx: &Context) -> Outcome {
    let found = match events_of(ctx, EventCategory::Disk) {
        Ok(f) => f,
        Err(o) => return o,
    };
    let level = if found.iter().any(|e| e.level == EventLevel::Critical) {
        Level::Critical
    } else {
        Level::Warning
    };
    Outcome::Findings(vec![finding(
        level,
        Category::Crashes,
        format!(
            "{} in the last {VIEW_DAYS} days",
            plural(
                found.len(),
                "disk or file system error",
                "disk or file system errors"
            )
        ),
        examples(&found),
        &[
            "Back up your important files now.",
            "Check the drive's health on the Storage page (Read as root adds SATA SMART data).",
            "Reseat or replace the drive's cable (SATA) or reseat the NVMe drive.",
            "File system errors: run a file system check from a live USB or at boot (fsck).",
        ],
    )])
}

pub(super) fn out_of_memory(ctx: &Context) -> Outcome {
    let found = match events_of(ctx, EventCategory::OutOfMemory) {
        Ok(f) => f,
        Err(o) => return o,
    };
    Outcome::Findings(vec![finding(
        Level::Warning,
        Category::Crashes,
        format!(
            "Ran out of memory {} in the last {VIEW_DAYS} days",
            plural(found.len(), "time", "times")
        ),
        examples(&found),
        &[
            "Close memory-hungry apps or browser tabs you don't need.",
            "Add swap or zram (Ubuntu: a larger /swap.img, or install zram-tools) so the system slows down instead of killing apps.",
            "If it keeps happening, add more RAM.",
        ],
    )])
}

pub(super) fn overheating(ctx: &Context) -> Outcome {
    let found = match events_of(ctx, EventCategory::Overheating) {
        Ok(f) => f,
        Err(o) => return o,
    };
    let critical = found.iter().any(|e| e.level == EventLevel::Critical);
    Outcome::Findings(vec![finding(
        if critical {
            Level::Critical
        } else {
            Level::Warning
        },
        Category::Crashes,
        format!(
            "Overheated {} in the last {VIEW_DAYS} days",
            plural(found.len(), "time", "times")
        ),
        examples(&found),
        &[
            "Clean dust out of the fans, heatsinks and filters.",
            "Check that every fan spins (Sensors page) and that the CPU cooler is mounted firmly.",
            "On a laptop, keep the vents clear and use it on a hard surface.",
        ],
    )])
}

#[cfg(test)]
mod tests {
    use super::super::testutil::*;
    use super::*;
    use crate::SystemInfo;
    use crate::eventlog::EventLog;
    use crate::sensors::SensorTable;

    fn entry(category: EventCategory, level: EventLevel, detail: &str) -> LogEntry {
        LogEntry {
            unix: 1_792_000_000,
            utc_offset: None,
            source: "kernel".into(),
            level,
            category,
            title: "t",
            detail: detail.into(),
        }
    }

    #[test]
    fn journal_checks() {
        let info = SystemInfo::default();
        let t = SensorTable::default();
        let mut c = ctx(&info, &t);
        assert!(matches!(kernel_crashes(&c), Outcome::Skipped(_)));

        let empty = EventLog::default();
        c.events = Some(&empty);
        for check in [
            kernel_crashes,
            hardware_errors,
            gpu_hangs,
            disk_errors,
            out_of_memory,
            overheating,
        ] {
            assert!(matches!(check(&c), Outcome::Pass(_)));
        }

        let log = EventLog {
            entries: vec![
                entry(
                    EventCategory::HardwareError,
                    EventLevel::Warning,
                    "EDAC MC0: 1 CE",
                ),
                entry(
                    EventCategory::HardwareError,
                    EventLevel::Warning,
                    "EDAC MC0: 1 CE",
                ),
                entry(
                    EventCategory::GpuTimeout,
                    EventLevel::Error,
                    "amdgpu: ring gfx timeout",
                ),
                entry(
                    EventCategory::KernelCrash,
                    EventLevel::Critical,
                    "Kernel panic",
                ),
            ],
            errors: Vec::new(),
        };
        c.events = Some(&log);
        assert_eq!(
            levels(&hardware_errors(&c)),
            [Level::Warning],
            "corrected only"
        );
        assert_eq!(levels(&gpu_hangs(&c)), [Level::Warning]);
        assert_eq!(levels(&kernel_crashes(&c)), [Level::Critical]);
        match hardware_errors(&c) {
            Outcome::Findings(f) => {
                assert_eq!(
                    f[0].title,
                    "2 hardware errors logged in the last 30 days (all corrected)"
                );
                assert_eq!(f[0].detail, "EDAC MC0: 1 CE", "repeats are listed once");
            }
            o => panic!("{o:?}"),
        }

        let failed = EventLog {
            entries: Vec::new(),
            errors: vec!["kernel messages: journalctl failed".into()],
        };
        c.events = Some(&failed);
        assert!(matches!(disk_errors(&c), Outcome::Skipped(_)));
    }
}
