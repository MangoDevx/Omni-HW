//! Crash checks: WHEA hardware errors, blue screens and unexpected restarts from the System
//! event log, plus WHEA errors logged while the app runs.

use super::helpers::{
    DDU_URL, board_drivers_step, count_text, format_date, guided, memory_test_steps, plural,
};
use super::{Category, Context, Level, Outcome, Step};
use crate::bios_help::{self, BiosSetting};
use crate::checkup::{
    CrashCause, EVENT_DAYS, Event, base_stop_code, crash_cause, stop_code_label, stop_code_name,
};
use crate::civil_from_days;
use crate::whea::{WheaEvent, most_common_location};
use std::time::UNIX_EPOCH;

/// Date of the most recent event, or "" when there are none.
fn last_date<'a>(events: impl IntoIterator<Item = &'a Event>) -> String {
    events
        .into_iter()
        .map(|e| e.date)
        .max()
        .map(format_date)
        .unwrap_or_default()
}

fn stability_steps(ctx: &Context) -> Vec<Step> {
    let mut steps = bios_help::bios_steps(
        ctx.info,
        BiosSetting::LoadDefaults,
        "Load optimized defaults: this turns off XMP/EXPO, PBO/overclocks and undervolts.",
        "",
    );
    steps.push(Step::new(
        "Use the PC normally for a few days. If the errors stop, re-enable only XMP/EXPO and see whether they come back; if they do, try the next lower memory speed.",
    ));
    steps.extend(memory_test_steps());
    steps
}

/// A live WHEA event as an event log entry, for counting alongside the startup query.
fn live_event(e: &WheaEvent) -> Event {
    let days = e
        .time
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| (d.as_secs() / 86_400) as i64);
    Event {
        id: e.id,
        date: civil_from_days(days),
        code: None,
    }
}

/// " 2 of them were logged while Omni-HW was running. Most errors came from CPU APIC ID 5."
/// for the live events with these IDs, or "".
fn live_text(live: &[WheaEvent], ids: &[u32]) -> String {
    let live: Vec<WheaEvent> = live
        .iter()
        .filter(|e| ids.contains(&e.id))
        .cloned()
        .collect();
    let mut s = String::new();
    if !live.is_empty() {
        s += &format!(
            " {} logged while Omni-HW was running.",
            if live.len() == 1 {
                "1 of them was".to_string()
            } else {
                format!("{} of them were", live.len())
            }
        );
    }
    if let Some((loc, _)) = most_common_location(&live) {
        s += &format!(" Most errors came from {loc}.");
    }
    s
}

pub(super) fn whea_errors(ctx: &Context) -> Outcome {
    let none = Vec::new();
    let past = match &ctx.info.checkup.events {
        Some(ev) => &ev.whea,
        None if ctx.whea_live.is_empty() => {
            return Outcome::Skipped("System event log not readable".into());
        }
        None => &none,
    };
    let live: Vec<Event> = ctx.whea_live.iter().map(live_event).collect();
    if past.is_empty() && live.is_empty() {
        return Outcome::Pass(format!(
            "No hardware errors logged in the last {EVENT_DAYS} days"
        ));
    }
    let by = |ids: &[u32]| -> Vec<&Event> {
        past.iter()
            .chain(&live)
            .filter(|e| ids.contains(&e.id))
            .collect()
    };
    let fatal = by(&[18]);
    let corrected_cpu_mem = by(&[19, 47]);
    let pcie = by(&[17]);
    let mut out = Vec::new();
    if !fatal.is_empty() || !corrected_cpu_mem.is_empty() {
        let n = fatal.len() + corrected_cpu_mem.len();
        out.push(
            guided(
                if fatal.is_empty() {
                    Level::Warning
                } else {
                    Level::Critical
                },
                Category::System,
                format!("{} hardware errors in the last {EVENT_DAYS} days", count_text(n)),
                format!(
                    "Windows logged {} CPU/memory hardware errors (WHEA-Logger{}), most recently on {}.{} On a new build this almost always means an unstable memory profile (XMP/EXPO), CPU overclock (PBO) or undervolt.",
                    count_text(n),
                    if fatal.is_empty() {
                        String::new()
                    } else {
                        format!(", {} of them fatal", fatal.len())
                    },
                    last_date(fatal.iter().chain(&corrected_cpu_mem).copied()),
                    live_text(ctx.whea_live, &[18, 19, 47])
                ),
                stability_steps(ctx),
            )
            .bios(),
        );
    }
    if !pcie.is_empty() {
        out.push(guided(
            Level::Info,
            Category::System,
            format!("{} corrected PCIe errors in the last {EVENT_DAYS} days", count_text(pcie.len())),
            format!(
                "Windows corrected {} PCI Express errors (WHEA-Logger event 17), most recently on {}.{} They are fixed automatically, but frequent ones point at a device (often the graphics card or an NVMe drive) that isn't seated well or a link that's marginal.",
                count_text(pcie.len()),
                last_date(pcie.iter().copied()),
                live_text(ctx.whea_live, &[17])
            ),
            vec![
                Step::new("Reseat the graphics card and M.2 drives, and check any riser cable."),
                board_drivers_step(ctx, "Update the motherboard BIOS and chipset driver."),
                Step::new(
                    "If the errors continue, setting the PCIe slot to the card's generation (instead of Auto) in the BIOS can help.",
                ),
            ],
        ));
    }
    Outcome::Findings(out)
}

pub(super) fn crashes(ctx: &Context) -> Outcome {
    let Some(ev) = &ctx.info.checkup.events else {
        return Outcome::Skipped("System event log not readable".into());
    };
    let hard: Vec<&Event> = ev
        .unexpected_shutdowns
        .iter()
        .filter(|e| e.code.is_none_or(|c| c == 0))
        .collect();
    if ev.bugchecks.is_empty() && hard.is_empty() {
        return Outcome::Pass(format!(
            "No blue screens or unexpected shutdowns in the last {EVENT_DAYS} days"
        ));
    }
    let mut out = Vec::new();
    if !ev.bugchecks.is_empty() {
        // Stop codes by frequency.
        let mut codes: Vec<(u32, usize)> = Vec::new();
        for e in &ev.bugchecks {
            let c = base_stop_code(e.code.unwrap_or(0));
            match codes.iter_mut().find(|(x, _)| *x == c) {
                Some((_, n)) => *n += 1,
                None => codes.push((c, 1)),
            }
        }
        codes.sort_by_key(|&(c, n)| (std::cmp::Reverse(n), c));
        let list = codes
            .iter()
            .map(|&(c, n)| {
                if n > 1 {
                    format!("{} x{n}", stop_code_label(c))
                } else {
                    stop_code_label(c)
                }
            })
            .collect::<Vec<_>>()
            .join(", ");
        let mut steps = Vec::new();
        let mut causes: Vec<CrashCause> = Vec::new();
        for &(c, _) in &codes {
            let cause = crash_cause(c);
            if !causes.contains(&cause) {
                causes.push(cause);
            }
        }
        let mut bios = false;
        for cause in causes {
            match cause {
                CrashCause::MemoryOrOverclock | CrashCause::Hardware => {
                    if !bios {
                        steps.push(Step::new(
                            "Memory and stability codes: undo memory and CPU tuning first.",
                        ));
                        steps.extend(stability_steps(ctx));
                        bios = true;
                    }
                }
                CrashCause::Graphics => {
                    steps.push(Step::url(
                        "Graphics driver crashes (VIDEO_TDR): clean-reinstall the GPU driver with Display Driver Uninstaller (DDU) in Safe Mode, then install the latest driver.",
                        DDU_URL,
                    ));
                    steps.push(Step::new(
                        "Check the graphics card's power cables are fully seated, and undo any GPU overclock.",
                    ));
                }
                CrashCause::Storage => steps.push(Step::new(
                    "Storage codes: check the drive health on the Storage tab, reseat or replace SATA cables, and update the SSD firmware.",
                )),
                CrashCause::Driver => steps.push(board_drivers_step(
                    ctx,
                    "Driver codes: update the chipset, network, audio and graphics drivers (board maker's support page first).",
                )),
                CrashCause::Unknown => {}
            }
        }
        if let Some(&(top, _)) = codes.first() {
            let q = match stop_code_name(top) {
                Some(n) => format!("{n} blue screen fix"),
                None => format!("stop code 0x{top:X} blue screen fix"),
            };
            steps.push(Step::search(
                format!(
                    "Look up the most frequent stop code, {}.",
                    stop_code_label(top)
                ),
                q,
            ));
        }
        let n = ev.bugchecks.len();
        let mut f = guided(
            if n >= 3 {
                Level::Critical
            } else {
                Level::Warning
            },
            Category::System,
            format!(
                "{} in the last {EVENT_DAYS} days",
                plural(n, "blue screen", "blue screens")
            ),
            format!(
                "Windows crashed with a blue screen {} times (most recently on {}). Stop codes: {list}.",
                count_text(n),
                last_date(&ev.bugchecks)
            ),
            steps,
        );
        f.bios = bios;
        out.push(f);
    }
    if !hard.is_empty() {
        let n = hard.len();
        out.push(guided(
            if n >= 2 { Level::Warning } else { Level::Info },
            Category::System,
            format!(
                "{} without a blue screen",
                plural(n, "unexpected restart", "unexpected restarts")
            ),
            format!(
                "Windows restarted {} times without shutting down cleanly and without a blue screen (Kernel-Power 41), most recently on {}. Ignore any you caused by holding the power button or a power cut; otherwise the PC is losing power or freezing hard.",
                count_text(n),
                last_date(hard.iter().copied())
            ),
            vec![
                Step::new(
                    "If it shuts off during games or heavy load, check the power supply: enough wattage for the GPU, and separate PCIe cables for each GPU power connector.",
                ),
                Step::new(
                    "Watch CPU and GPU temperatures on the Sensors tab under load: a PC that switches itself off can be overheating.",
                ),
                Step::new("Undo overclocks and undervolts (CPU, GPU and memory) and see whether it stops."),
            ],
        ));
    }
    Outcome::Findings(out)
}

#[cfg(test)]
mod tests {
    use super::super::memory::memory_diagnostic;
    use super::super::testutil::*;
    use super::*;
    use crate::SystemInfo;
    use crate::checkup::Events;
    use crate::sensors::SensorTable;
    use crate::whea::{PciAddr, WheaKind};
    use std::time::Duration;

    fn ev(id: u32, code: Option<u32>) -> Event {
        Event {
            id,
            date: (2026, 10, 1),
            code,
        }
    }

    #[test]
    fn event_checks() {
        let s = SensorTable::default();
        let mut info = SystemInfo::default();
        assert!(matches!(whea_errors(&ctx(&info, &s)), Outcome::Skipped(_)));
        info.checkup.events = Some(Events::default());
        let c = ctx(&info, &s);
        assert!(matches!(whea_errors(&c), Outcome::Pass(_)));
        assert!(matches!(crashes(&c), Outcome::Pass(_)));
        assert!(matches!(memory_diagnostic(&c), Outcome::Skipped(_)));

        info.checkup.events = Some(Events {
            whea: vec![ev(19, None), ev(19, None), ev(17, None)],
            unexpected_shutdowns: vec![ev(41, Some(0)), ev(41, Some(0x1A)), ev(41, None)],
            bugchecks: vec![ev(1001, Some(0x1A)), ev(1001, Some(0x116))],
            memory_diagnostic: Some(ev(1102, None)),
        });
        let c = ctx(&info, &s);
        let o = whea_errors(&c);
        assert_eq!(levels(&o), vec![Level::Warning, Level::Info]);
        let Outcome::Findings(f) = o else {
            unreachable!()
        };
        assert!(f[0].bios);
        let o = crashes(&c);
        assert_eq!(levels(&o), vec![Level::Warning, Level::Warning]);
        let Outcome::Findings(f) = o else {
            unreachable!()
        };
        assert!(f[0].detail.contains("MEMORY_MANAGEMENT (0x1A)"));
        assert!(f[0].steps.iter().any(|s| s.text.contains("DDU")));
        assert!(f[1].title.starts_with("2 unexpected restarts"));
        assert_eq!(levels(&memory_diagnostic(&c)), vec![Level::Critical]);
    }

    fn live(id: u32, apic_id: Option<u32>, pcie: Option<PciAddr>) -> WheaEvent {
        WheaEvent {
            record_id: 1,
            id,
            // 2026-10-12 00:00:10 UTC.
            time: Some(UNIX_EPOCH + Duration::from_secs(20_738 * 86_400 + 10)),
            fatal: id == 18,
            kind: if id == 17 {
                WheaKind::Pcie
            } else {
                WheaKind::Cache
            },
            source: String::new(),
            apic_id,
            mca_bank: None,
            pcie,
            device: None,
            summary: String::new(),
        }
    }

    #[test]
    fn live_whea_events() {
        let s = SensorTable::default();
        let mut info = SystemInfo::default();
        let gpu = PciAddr {
            bus: 1,
            device: 0,
            function: 0,
        };
        let events = [
            live(19, Some(5), None),
            live(19, Some(5), None),
            live(19, Some(2), None),
            live(17, None, Some(gpu)),
        ];
        // Counted even when the startup query failed.
        let c = Context {
            whea_live: &events,
            ..ctx(&info, &s)
        };
        let o = whea_errors(&c);
        assert_eq!(levels(&o), vec![Level::Warning, Level::Info]);
        let Outcome::Findings(f) = o else {
            unreachable!()
        };
        assert!(f[0].title.starts_with("3 hardware errors"));
        assert!(f[0].detail.contains("most recently on 2026-10-12"));
        assert!(
            f[0].detail
                .contains("3 of them were logged while Omni-HW was running")
        );
        assert!(f[0].detail.contains("Most errors came from CPU APIC ID 5."));
        assert!(f[1].detail.contains("1 of them was logged"));
        assert!(f[1].detail.contains("from PCIe device 01:00.0"));

        // Added to the startup query's events; a live fatal error makes it critical.
        info.checkup.events = Some(Events {
            whea: vec![ev(19, None), ev(47, None)],
            ..Default::default()
        });
        let events = [live(18, None, None)];
        let c = Context {
            whea_live: &events,
            ..ctx(&info, &s)
        };
        let o = whea_errors(&c);
        assert_eq!(levels(&o), vec![Level::Critical]);
        let Outcome::Findings(f) = o else {
            unreachable!()
        };
        assert!(f[0].title.starts_with("3 hardware errors"));
        assert!(f[0].detail.contains("1 of them fatal"));
        assert!(!f[0].detail.contains("Most errors came from"));
    }
}
