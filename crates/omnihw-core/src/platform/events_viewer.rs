//! Reads the Event Log page's events (see [`crate::eventlog`]) from the systemd journal with
//! `journalctl`. Blocking: call it on a worker thread, never the UI thread.

use crate::eventlog::{self, EventLog, KERNEL_CAP, LogEntry, SERVICE_CAP, VIEW_DAYS};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const FIELDS: &str = "--output-fields=PRIORITY,SYSLOG_IDENTIFIER,MESSAGE";
/// Kernel messages logged below warning that still report hardware errors (`--grep` is a
/// PCRE2 pattern, matched case-insensitively for lower-case patterns).
const HARDWARE_INFO: &str = "hardware error|machine check|edac|pcie bus error";
/// What journalctl prints when the user may only read their own journal.
const OWN_MESSAGES_ONLY: &str = "not seeing messages from other users";

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// The JSON lines of `journalctl <args>`, and whether the system journal was out of reach.
fn journalctl(args: &[&str]) -> Result<(String, bool), String> {
    let out = Command::new("journalctl")
        .args(["--output=json", "--no-pager", FIELDS])
        .args(args)
        .output()
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => {
                "journalctl wasn't found (the Event Log reads systemd's journal)".to_string()
            }
            _ => e.to_string(),
        })?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !out.status.success() {
        let first = stderr.lines().next().unwrap_or("journalctl failed").trim();
        return Err(first.to_string());
    }
    Ok((
        String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr.contains(OWN_MESSAGES_ONLY),
    ))
}

/// The local time's offset from UTC at `unix`, in seconds.
fn utc_offset(unix: i64) -> Option<i32> {
    let t = unix as libc::time_t;
    // SAFETY: `tm` is plain data that localtime_r fills; both pointers are valid for the call.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let ok = unsafe { !libc::localtime_r(&t, &mut tm).is_null() };
    ok.then_some(tm.tm_gmtoff as i32)
}

/// The last [`VIEW_DAYS`] days of categorized and other critical/error events, newest first.
/// Failed queries are listed in `errors`; the rest are still returned.
pub fn read() -> EventLog {
    let since = format!("--since=@{}", now_unix() - i64::from(VIEW_DAYS) * 86_400);
    let kernel_cap = KERNEL_CAP.to_string();
    let service_cap = SERVICE_CAP.to_string();
    let queries: [(&str, Vec<&str>); 3] = [
        (
            "kernel messages",
            vec![
                "_TRANSPORT=kernel",
                "--priority=0..4",
                "--reverse",
                "--lines",
                &kernel_cap,
            ],
        ),
        (
            "kernel hardware reports",
            vec![
                "_TRANSPORT=kernel",
                "--priority=5..7",
                "--grep",
                HARDWARE_INFO,
                "--reverse",
                "--lines",
                &kernel_cap,
            ],
        ),
        (
            "app crashes",
            vec![
                "SYSLOG_IDENTIFIER=systemd-coredump",
                "SYSLOG_IDENTIFIER=systemd-oomd",
                "--reverse",
                "--lines",
                &service_cap,
            ],
        ),
    ];
    let mut log = EventLog::default();
    let mut own_only = false;
    for (what, mut args) in queries {
        args.push(&since);
        match journalctl(&args) {
            Ok((text, partial)) => {
                own_only |= partial;
                log.entries.extend(eventlog::parse_journal(&text));
            }
            // journalctl built without PCRE2 has no --grep; the other queries still cover the
            // errors logged as warnings or worse.
            Err(e) if what == "kernel hardware reports" && e.contains("grep") => {}
            Err(e) => log.errors.push(format!("{what}: {e}")),
        }
    }
    if own_only {
        log.errors.push(
            "system journal: only your own messages could be read. Add yourself to the \"adm\" group (sudo usermod -aG adm $USER), then sign out and back in."
                .into(),
        );
    }
    for e in &mut log.entries {
        e.utc_offset = utc_offset(e.unix);
    }
    eventlog::sort_and_cap(&mut log.entries);
    log
}

/// Hardware errors the kernel logged at or after `since_unix`, oldest first. For the memory and
/// disk tests, which report errors logged while they ran.
pub fn hardware_errors_since(since_unix: i64) -> Vec<LogEntry> {
    let since = format!("--since=@{since_unix}");
    let Ok((text, _)) = journalctl(&["_TRANSPORT=kernel", &since]) else {
        return Vec::new();
    };
    let mut list: Vec<LogEntry> = eventlog::parse_journal(&text)
        .into_iter()
        .filter(eventlog::is_hardware_error)
        .collect();
    for e in &mut list {
        e.utc_offset = utc_offset(e.unix);
    }
    list
}
