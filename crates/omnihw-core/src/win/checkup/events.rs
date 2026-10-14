//! Hardware-error and crash events from the System event log (readable without administrator
//! rights): WHEA-Logger, Kernel-Power 41, BugCheck 1001 and Windows Memory Diagnostic results.

use crate::checkup::{EVENT_CAP, EVENT_DAYS, Event, Events, MEMDIAG_DAYS, parse_event_xml};
use windows::Win32::System::EventLog::{
    EVT_HANDLE, EvtClose, EvtNext, EvtQuery, EvtQueryChannelPath, EvtQueryReverseDirection,
    EvtRender, EvtRenderEventXml,
};
use windows::core::{HSTRING, w};

const MS_PER_DAY: u64 = 86_400_000;

/// Closes an event log handle on drop.
struct Handle(EVT_HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: handle returned by EvtQuery/EvtNext, closed once.
        let _ = unsafe { EvtClose(self.0) };
    }
}

fn render(ev: &Handle) -> Option<String> {
    let (mut used, mut count) = (0u32, 0u32);
    // SAFETY: size query without a buffer (fails with ERROR_INSUFFICIENT_BUFFER).
    let _ = unsafe {
        EvtRender(
            None,
            ev.0,
            EvtRenderEventXml.0,
            0,
            None,
            &mut used,
            &mut count,
        )
    };
    if used == 0 {
        return None;
    }
    let mut buf = vec![0u16; used as usize / 2 + 1];
    // SAFETY: buffer size passed in bytes.
    unsafe {
        EvtRender(
            None,
            ev.0,
            EvtRenderEventXml.0,
            (buf.len() * 2) as u32,
            Some(buf.as_mut_ptr().cast()),
            &mut used,
            &mut count,
        )
    }
    .ok()?;
    Some(super::utf16_z(&buf))
}

/// Newest-first events matching an XPath filter on the System log, at most `cap`.
fn query(filter: &str, cap: usize) -> Option<Vec<Event>> {
    // SAFETY: NUL-terminated strings; handle closed by `Handle`.
    let results = Handle(
        unsafe {
            EvtQuery(
                None,
                w!("System"),
                &HSTRING::from(filter),
                EvtQueryChannelPath.0 | EvtQueryReverseDirection.0,
            )
        }
        .ok()?,
    );
    let mut out = Vec::new();
    while out.len() < cap {
        let mut batch = [0isize; 32];
        let mut got = 0u32;
        // SAFETY: array length passed with the slice; returned handles closed below.
        if unsafe { EvtNext(results.0, &mut batch, 5000, 0, &mut got) }.is_err() {
            break;
        }
        for &h in &batch[..got as usize] {
            let ev = Handle(EVT_HANDLE(h));
            if out.len() < cap
                && let Some(e) = render(&ev).as_deref().and_then(parse_event_xml)
            {
                out.push(e);
            }
        }
    }
    Some(out)
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
