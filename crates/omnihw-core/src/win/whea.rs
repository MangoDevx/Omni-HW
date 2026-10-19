//! Live WHEA-Logger events: a pull subscription to the System log for hardware errors logged
//! while the app runs. Polling never blocks: a signal event says whether anything arrived.

use super::eventlog::{own, render_xml};
use crate::whea::{WheaEvent, parse_whea_xml};
use windows::Win32::Foundation::{HANDLE, WAIT_OBJECT_0};
use windows::Win32::System::EventLog::{
    EVT_HANDLE, EvtNext, EvtSubscribe, EvtSubscribeToFutureEvents,
};
use windows::Win32::System::Threading::{CreateEventW, ResetEvent, SetEvent, WaitForSingleObject};
use windows::core::{Owned, PCWSTR, w};

/// The same event IDs the startup query reads (see `checkup::events`).
const QUERY: PCWSTR = w!(
    "*[System[Provider[@Name='Microsoft-Windows-WHEA-Logger'] and (EventID=17 or EventID=18 or EventID=19 or EventID=47)]]"
);
/// Events fetched per `EvtNext` call.
const BATCH: usize = 16;
/// Most events taken in one poll; the rest wait for the next one.
const POLL_CAP: usize = 256;

/// Subscription to new WHEA-Logger events in the System log.
pub struct WheaWatcher {
    // Field order: the subscription closes before its signal event.
    sub: Owned<EVT_HANDLE>,
    signal: Owned<HANDLE>,
}

impl WheaWatcher {
    /// Subscribe to future events. `None` if the event log service can't be reached.
    pub fn new() -> Option<WheaWatcher> {
        // SAFETY: plain call; a manual-reset event, initially clear, unnamed.
        let signal = unsafe { Owned::new(CreateEventW(None, true, false, PCWSTR::null()).ok()?) };
        // SAFETY: the channel and query are static NUL-terminated strings; no callback or
        // context (pull model), and `signal` is a valid event that outlives the subscription.
        let sub = own(unsafe {
            EvtSubscribe(
                None,
                Some(*signal),
                w!("System"),
                QUERY,
                None,
                None,
                None,
                EvtSubscribeToFutureEvents.0,
            )
        }
        .ok()?);
        Some(WheaWatcher { sub, signal })
    }

    /// Events logged since the last poll, oldest first. Never waits.
    pub fn poll(&mut self) -> Vec<WheaEvent> {
        let mut out = Vec::new();
        // SAFETY: `signal` is a valid event handle; timeout 0 only tests its state.
        if unsafe { WaitForSingleObject(*self.signal, 0) } != WAIT_OBJECT_0 {
            return out;
        }
        // Clear before draining, so an event logged meanwhile sets it again.
        // SAFETY: `signal` is a valid event handle.
        let _ = unsafe { ResetEvent(*self.signal) };
        let mut taken = 0;
        while taken < POLL_CAP {
            let mut batch = [0isize; BATCH];
            let mut got = 0u32;
            // SAFETY: the array length is passed with the slice and `got` is a valid out
            // pointer; timeout 0 returns at once (ERROR_NO_MORE_ITEMS when drained).
            if unsafe { EvtNext(*self.sub, &mut batch, 0, 0, &mut got) }.is_err() || got == 0 {
                break;
            }
            for ev in batch[..(got as usize).min(BATCH)]
                .iter()
                .map(|&h| own(EVT_HANDLE(h)))
            {
                taken += 1;
                if let Some(e) = render_xml(*ev).as_deref().and_then(parse_whea_xml) {
                    out.push(e);
                }
            }
        }
        if taken >= POLL_CAP {
            // More may be waiting: look again next poll.
            // SAFETY: `signal` is a valid event handle.
            let _ = unsafe { SetEvent(*self.signal) };
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn subscribes_and_polls_without_blocking() {
        let mut w = WheaWatcher::new().expect("subscribe to the System log");
        let t = Instant::now();
        // Future events only, so normally nothing; either way it must return at once.
        let _ = w.poll();
        let _ = w.poll();
        assert!(t.elapsed() < Duration::from_millis(500));
    }
}
