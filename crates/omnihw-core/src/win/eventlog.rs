//! Windows event log queries (wevtapi), shared by the Health Check, the power collector and
//! the Event Log page. The System and Application logs are readable without administrator
//! rights.

use super::wide::from_wide;
use windows::Win32::System::EventLog::{
    EVT_HANDLE, EvtNext, EvtQuery, EvtQueryChannelPath, EvtQueryReverseDirection, EvtRender,
    EvtRenderEventXml,
};
use windows::core::{HSTRING, Owned};

/// Events fetched per `EvtNext` call.
const BATCH: usize = 32;
/// How long `EvtNext` may wait for the next batch, in milliseconds.
const NEXT_TIMEOUT_MS: u32 = 5000;

/// Take ownership of an event log handle so it is closed with `EvtClose` on drop.
fn own(h: EVT_HANDLE) -> Owned<EVT_HANDLE> {
    // SAFETY: callers pass a handle freshly returned by EvtQuery/EvtNext, owned by nobody else.
    unsafe { Owned::new(h) }
}

/// One event rendered as XML.
fn render_xml(ev: EVT_HANDLE) -> Option<String> {
    let (mut used, mut count) = (0u32, 0u32);
    // SAFETY: size query without a buffer (fails with ERROR_INSUFFICIENT_BUFFER, filling
    // `used` with the size in bytes).
    let _ = unsafe {
        EvtRender(
            None,
            ev,
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
    // SAFETY: the buffer pointer and its size in bytes describe `buf`.
    unsafe {
        EvtRender(
            None,
            ev,
            EvtRenderEventXml.0,
            (buf.len() * 2) as u32,
            Some(buf.as_mut_ptr().cast()),
            &mut used,
            &mut count,
        )
    }
    .ok()?;
    Some(from_wide(&buf))
}

/// Newest-first events of the System log matching an XPath `filter`, each rendered as XML and
/// passed to `parse`; stops once `cap` events parsed. `None` if the query itself failed.
pub(crate) fn query_system<T>(
    filter: &str,
    cap: usize,
    parse: impl FnMut(&str) -> Option<T>,
) -> Option<Vec<T>> {
    query_channel("System", filter, cap, parse).ok()
}

/// Like [`query_system`] for any `channel` (e.g. "Application"). `Err` holds the Windows error
/// message if the query itself failed (missing channel, invalid XPath).
pub(crate) fn query_channel<T>(
    channel: &str,
    filter: &str,
    cap: usize,
    mut parse: impl FnMut(&str) -> Option<T>,
) -> Result<Vec<T>, String> {
    // SAFETY: the channel and filter are NUL-terminated strings that outlive the call.
    let results = own(unsafe {
        EvtQuery(
            None,
            &HSTRING::from(channel),
            &HSTRING::from(filter),
            EvtQueryChannelPath.0 | EvtQueryReverseDirection.0,
        )
    }
    .map_err(|e| e.message())?);
    let mut out = Vec::new();
    while out.len() < cap {
        let mut batch = [0isize; BATCH];
        let mut got = 0u32;
        // SAFETY: the array length is passed with the slice and `got` is a valid out pointer.
        if unsafe { EvtNext(*results, &mut batch, NEXT_TIMEOUT_MS, 0, &mut got) }.is_err()
            || got == 0
        {
            break;
        }
        // Every returned handle is owned (and closed) even once the cap is reached.
        for ev in batch[..(got as usize).min(BATCH)]
            .iter()
            .map(|&h| own(EVT_HANDLE(h)))
        {
            if out.len() < cap
                && let Some(e) = render_xml(*ev).as_deref().and_then(&mut parse)
            {
                out.push(e);
            }
        }
    }
    Ok(out)
}
