//! Hardware-error and crash events from the System event log (readable without administrator
//! rights): WHEA-Logger, Kernel-Power 41, BugCheck 1001 and Windows Memory Diagnostic results,
//! plus GPU driver timeouts (Display 4101, and LiveKernelEvent 141/117 from the Application log).

use crate::checkup::{EVENT_CAP, EVENT_DAYS, Event, Events, MEMDIAG_DAYS, parse_event_xml};
use crate::eventlog::{
    display_tdr_filter, dump_local_time, live_kernel_code, live_kernel_filter, report_id, within,
};
use crate::win::eventlog::{query_channel, query_system};
use crate::win::events_viewer::{local_to_unix, now_unix};

/// Newest-first events matching an XPath filter on the System log, at most `cap`.
fn query(filter: &str, cap: usize) -> Option<Vec<Event>> {
    query_system(filter, cap, parse_event_xml)
}

pub fn events() -> Option<Events> {
    let whea = query(
        &format!(
            "*[System[Provider[@Name='Microsoft-Windows-WHEA-Logger'] and \
             (EventID=17 or EventID=18 or EventID=19 or EventID=47) and {}]]",
            within(EVENT_DAYS)
        ),
        EVENT_CAP,
    )?;
    let crashes = query(
        &format!(
            "*[System[((Provider[@Name='Microsoft-Windows-Kernel-Power'] and EventID=41) or \
             (Provider[@Name='Microsoft-Windows-WER-SystemErrorReporting' or @Name='BugCheck'] and EventID=1001)) \
             and {}]]",
            within(EVENT_DAYS)
        ),
        EVENT_CAP,
    )
    .unwrap_or_default();
    let memory_diagnostic = query(
        &format!(
            "*[System[Provider[@Name='Microsoft-Windows-MemoryDiagnostics-Results'] and {}]]",
            within(MEMDIAG_DAYS)
        ),
        1,
    )
    .and_then(|mut v| v.pop());
    let (unexpected_shutdowns, bugchecks) = crashes.into_iter().partition(|e| e.id == 41);
    Some(Events {
        whea,
        unexpected_shutdowns,
        bugchecks,
        memory_diagnostic,
        tdr: tdr_events(),
    })
}

/// Display 4101 and LiveKernelEvent 141/117 reports (one per WER report), newest first.
fn tdr_events() -> Vec<Event> {
    let mut out = query(&display_tdr_filter(EVENT_DAYS), EVENT_CAP).unwrap_or_default();
    let mut seen = std::collections::HashSet::new();
    let oldest = now_unix() - i64::from(EVENT_DAYS) * 86_400;
    let live = query_channel(
        "Application",
        &live_kernel_filter(EVENT_DAYS),
        EVENT_CAP,
        |xml| {
            if report_id(xml).is_some_and(|id| !seen.insert(id)) {
                return None;
            }
            let code = live_kernel_code(xml)?;
            let mut e = parse_event_xml(xml)?;
            e.code = Some(code);
            // WER often sends reports days later: date them by their dump (UTC date, like
            // the System log events).
            if let Some(t) = dump_local_time(xml).and_then(local_to_unix) {
                if t < oldest {
                    return None;
                }
                e.date = crate::civil_from_days(t.div_euclid(86_400));
            }
            Some(e)
        },
    );
    out.extend(live.unwrap_or_default());
    out.sort_by_key(|e| std::cmp::Reverse(e.date));
    out
}
