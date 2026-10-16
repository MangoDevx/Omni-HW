//! Hardware-error and crash events from the System event log (readable without administrator
//! rights): WHEA-Logger, Kernel-Power 41, BugCheck 1001 and Windows Memory Diagnostic results.

use crate::checkup::{EVENT_CAP, EVENT_DAYS, Event, Events, MEMDIAG_DAYS, parse_event_xml};
use crate::win::eventlog::query_system;

const MS_PER_DAY: u64 = 86_400_000;

/// Newest-first events matching an XPath filter on the System log, at most `cap`.
fn query(filter: &str, cap: usize) -> Option<Vec<Event>> {
    query_system(filter, cap, parse_event_xml)
}

fn within(days: u32) -> String {
    format!(
        "TimeCreated[timediff(@SystemTime) <= {}]",
        days as u64 * MS_PER_DAY
    )
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
    })
}
