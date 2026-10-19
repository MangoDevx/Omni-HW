//! Reads the Event Log page's events (see [`crate::eventlog`]) from the System and Application
//! logs. Blocking: call it on a worker thread, never the UI thread. No administrator rights
//! needed.

use crate::eventlog::EventLog;

/// The last [`crate::eventlog::VIEW_DAYS`] days of categorized and other critical/error
/// events, newest first. Failed queries are listed in `errors`; the rest are still returned.
#[cfg(windows)]
pub fn read() -> EventLog {
    use super::eventlog::query_channel;
    use crate::eventlog::{
        Channel, LIVE_KERNEL_CAP, NAMED_CAP, OTHER_CAP, VIEW_DAYS, dump_local_time,
        live_kernel_filter, named_filter, other_filter, parse_named, parse_other, report_id,
        sort_and_cap,
    };

    let mut log = EventLog::default();
    let mut add = |what: &str, r: Result<Vec<_>, String>| match r {
        Ok(v) => log.entries.extend(v),
        Err(e) => log.errors.push(format!("{what}: {}", e.trim())),
    };
    add(
        "System log",
        query_channel("System", &named_filter(VIEW_DAYS), NAMED_CAP, |x| {
            parse_named(x, Channel::System)
        }),
    );
    add(
        "System log (other errors)",
        query_channel("System", &other_filter(VIEW_DAYS), OTHER_CAP, parse_other),
    );
    let mut seen = std::collections::HashSet::new();
    let oldest = now_unix() - i64::from(VIEW_DAYS) * 86_400;
    add(
        "Application log",
        query_channel(
            "Application",
            &live_kernel_filter(VIEW_DAYS),
            LIVE_KERNEL_CAP,
            |x| {
                // WER can log one report more than once.
                if report_id(x).is_some_and(|id| !seen.insert(id)) {
                    return None;
                }
                let mut e = parse_named(x, Channel::Application)?;
                // Reports are often sent days later: date them by their dump.
                if let Some(t) = dump_local_time(x).and_then(local_to_unix) {
                    e.unix = t;
                }
                (e.unix >= oldest).then_some(e)
            },
        ),
    );
    sort_and_cap(&mut log.entries);
    for e in &mut log.entries {
        e.utc_offset = utc_offset(e.unix);
    }
    log
}

#[cfg(not(windows))]
pub fn read() -> EventLog {
    EventLog {
        errors: vec!["The event log is only available on Windows".into()],
        ..Default::default()
    }
}

#[cfg(windows)]
pub(crate) fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

#[cfg(windows)]
fn system_time(unix: i64) -> Option<windows::Win32::Foundation::SYSTEMTIME> {
    let (days, rem) = (unix.div_euclid(86_400), unix.rem_euclid(86_400));
    let (y, m, d) = crate::civil_from_days(days);
    Some(windows::Win32::Foundation::SYSTEMTIME {
        wYear: u16::try_from(y).ok()?,
        wMonth: m as u16,
        wDay: d as u16,
        wHour: (rem / 3600) as u16,
        wMinute: (rem / 60 % 60) as u16,
        wSecond: (rem % 60) as u16,
        ..Default::default()
    })
}

#[cfg(windows)]
fn unix_of(t: &windows::Win32::Foundation::SYSTEMTIME) -> i64 {
    crate::recording::days_from_civil(i64::from(t.wYear), u32::from(t.wMonth), u32::from(t.wDay))
        * 86_400
        + i64::from(t.wHour) * 3600
        + i64::from(t.wMinute) * 60
        + i64::from(t.wSecond)
}

/// Local time minus UTC, in seconds, at Unix time `unix` (daylight saving as it was then).
#[cfg(windows)]
fn utc_offset(unix: i64) -> Option<i32> {
    use windows::Win32::System::Time::SystemTimeToTzSpecificLocalTime;
    let utc = system_time(unix)?;
    let mut local = Default::default();
    // SAFETY: both pointers refer to live SYSTEMTIME values; `None` means the current time zone.
    unsafe { SystemTimeToTzSpecificLocalTime(None, &utc, &mut local) }.ok()?;
    i32::try_from(unix_of(&local) - unix).ok()
}

/// Unix time of a local (year, month, day, hour, minute) in the current time zone.
#[cfg(windows)]
pub(crate) fn local_to_unix((y, m, d, h, mi): (i64, u32, u32, u32, u32)) -> Option<i64> {
    use windows::Win32::System::Time::TzSpecificLocalTimeToSystemTime;
    let days = crate::recording::days_from_civil(y, m, d);
    let local = system_time(days * 86_400 + i64::from(h) * 3600 + i64::from(mi) * 60)?;
    let mut utc = Default::default();
    // SAFETY: both pointers refer to live SYSTEMTIME values; `None` means the current time zone.
    unsafe { TzSpecificLocalTimeToSystemTime(None, &local, &mut utc) }.ok()?;
    Some(unix_of(&utc))
}

#[cfg(all(test, windows))]
mod tests {
    /// Runs the real queries against this PC's event logs (checks the XPath filters are
    /// accepted). `cargo test -p omnihw-core live_event_log -- --ignored --nocapture`
    #[test]
    #[ignore = "reads this PC's event logs"]
    fn live_event_log() {
        use crate::eventlog::*;
        let log = super::read();
        println!("errors: {:?}", log.errors);
        for c in EventCategory::ALL {
            let n = log.entries.iter().filter(|e| e.category == c).count();
            println!("{}: {n}", c.name());
        }
        for e in log.entries.iter().take(15) {
            println!(
                "{} {} {} | {} | {}",
                e.time_text(),
                e.level.name(),
                e.source(),
                e.title,
                e.detail
            );
        }
        let tdr = crate::win::checkup::gather().events.map(|e| e.tdr);
        println!("Health Check TDR events: {tdr:?}");
        assert!(log.errors.is_empty(), "{:?}", log.errors);
        // An invalid filter is reported, not silently empty.
        let bad = crate::win::eventlog::query_channel("System", "*[System[", 1, |_| Some(()));
        println!("invalid XPath: {bad:?}");
        assert!(bad.is_err());
    }

    #[test]
    fn local_time_round_trip() {
        let t = super::local_to_unix((2026, 10, 15, 9, 15)).unwrap();
        let off = super::utc_offset(t).unwrap();
        assert_eq!((t + i64::from(off)).rem_euclid(86_400), 9 * 3600 + 15 * 60);
    }
}
