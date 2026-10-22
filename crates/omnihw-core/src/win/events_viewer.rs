//! Reads the Event Log page's events (see [`crate::eventlog`]). Blocking: call it on a worker
//! thread, never the UI thread.

use crate::eventlog::EventLog;

/// The last [`crate::eventlog::VIEW_DAYS`] days of categorized and other critical/error
/// events, newest first. Failed queries are listed in `errors`; the rest are still returned.
pub fn read() -> EventLog {
    EventLog {
        errors: vec!["The event log isn't read on Linux yet".into()],
        ..Default::default()
    }
}
