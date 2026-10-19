//! Event Log page model: the System/Application events that matter for PC health, sorted into
//! plain-English categories, plus the XPath filters that select them.
//!
//! Platform-neutral and unit-tested; the query runs in [`crate::win::events_viewer`] on a
//! worker thread.
//!
//! Event ids (Microsoft Learn and the event manifests on Windows 11):
//! - Kernel-Power 41: rebooted without cleanly shutting down first.
//! - BugCheck / WER-SystemErrorReporting 1001: rebooted from a bugcheck (blue screen).
//! - WHEA-Logger 17 (corrected PCIe), 18 (fatal), 19 (corrected CPU/bus), 47 (corrected memory).
//! - Display 4101: "Display driver ... stopped responding and has successfully recovered".
//! - Windows Error Reporting 1001 with `EventName` LiveKernelEvent in the Application log: a
//!   live kernel dump, `P1` holding the code in hex without "0x" (0x141
//!   VIDEO_ENGINE_TIMEOUT_DETECTED, 0x117 VIDEO_TDR_TIMEOUT_DETECTED). Sample report:
//!   <https://learn.microsoft.com/en-us/answers/questions/897557/livekernelevent141-causing-computer-to-freeze-rand>;
//!   codes: <https://learn.microsoft.com/windows-hardware/drivers/debugger/bug-check-code-reference2>.
//!   These are logged at Information level, so they're selected by data, not level.
//! - disk 7 (bad block), 11 (controller error), 51 (paging error), 52 (drive predicts failure),
//!   153 (I/O retried); Ntfs 55 (file system corruption), 98 (volume needs a disk check).
//! - EventLog 6008: the previous system shutdown was unexpected.

use crate::checkup::{base_stop_code, event_data, parse_event_xml, stop_code_label, unescape_xml};

/// Days of event log the page can show.
pub const VIEW_DAYS: u32 = 30;
/// Most named (categorized) System events read.
pub const NAMED_CAP: usize = 300;
/// Most "other" critical/error System events read.
pub const OTHER_CAP: usize = 150;
/// Most LiveKernelEvent reports read from the Application log.
pub const LIVE_KERNEL_CAP: usize = 50;
/// Most entries kept in total.
pub const TOTAL_CAP: usize = 500;

/// LiveKernelEvent codes that are GPU timeouts.
pub const GPU_LIVE_KERNEL_CODES: [u32; 2] = [0x141, 0x117];

/// Which event log an entry came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    System,
    Application,
}

impl Channel {
    pub fn name(self) -> &'static str {
        match self {
            Channel::System => "System",
            Channel::Application => "Application",
        }
    }
}

/// Severity as Windows logged it (LiveKernelEvent reports, logged as Information, count as
/// errors).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EventLevel {
    Warning,
    Error,
    Critical,
}

impl EventLevel {
    pub const ALL: [EventLevel; 3] = [EventLevel::Critical, EventLevel::Error, EventLevel::Warning];

    pub fn name(self) -> &'static str {
        match self {
            EventLevel::Critical => "Critical",
            EventLevel::Error => "Error",
            EventLevel::Warning => "Warning",
        }
    }

    /// From the `<Level>` value (1 critical, 2 error, 3 warning).
    fn from_xml(v: u32) -> Option<EventLevel> {
        match v {
            1 => Some(EventLevel::Critical),
            2 => Some(EventLevel::Error),
            3 => Some(EventLevel::Warning),
            _ => None,
        }
    }
}

/// Plain-English group of an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventCategory {
    UnexpectedShutdown,
    BlueScreen,
    HardwareError,
    GpuTimeout,
    Disk,
    DirtyShutdown,
    Other,
}

impl EventCategory {
    pub const ALL: [EventCategory; 7] = [
        EventCategory::UnexpectedShutdown,
        EventCategory::BlueScreen,
        EventCategory::HardwareError,
        EventCategory::GpuTimeout,
        EventCategory::Disk,
        EventCategory::DirtyShutdown,
        EventCategory::Other,
    ];

    pub fn name(self) -> &'static str {
        match self {
            EventCategory::UnexpectedShutdown => "Unexpected shutdown",
            EventCategory::BlueScreen => "Blue screen",
            EventCategory::HardwareError => "Hardware error",
            EventCategory::GpuTimeout => "GPU driver timeout",
            EventCategory::Disk => "Disk problem",
            EventCategory::DirtyShutdown => "Unexpected restart",
            EventCategory::Other => "Other critical/error",
        }
    }
}

/// A known event: provider names, the ids it covers and what it means.
struct Rule {
    providers: &'static [&'static str],
    id: u32,
    category: EventCategory,
    title: &'static str,
}

const KERNEL_POWER: &[&str] = &["Microsoft-Windows-Kernel-Power"];
const BUGCHECK: &[&str] = &["BugCheck", "Microsoft-Windows-WER-SystemErrorReporting"];
const WHEA: &[&str] = &["Microsoft-Windows-WHEA-Logger"];
const DISPLAY: &[&str] = &["Display"];
const DISK: &[&str] = &["disk"];
const NTFS: &[&str] = &["Ntfs", "Microsoft-Windows-Ntfs"];
const EVENTLOG: &[&str] = &["EventLog"];
/// Provider of LiveKernelEvent reports in the Application log.
pub const WER: &str = "Windows Error Reporting";

/// The categorized System events, most important first.
const RULES: &[Rule] = &[
    Rule {
        providers: KERNEL_POWER,
        id: 41,
        category: EventCategory::UnexpectedShutdown,
        title: "Restarted without shutting down cleanly",
    },
    Rule {
        providers: BUGCHECK,
        id: 1001,
        category: EventCategory::BlueScreen,
        title: "Restarted after a blue screen",
    },
    Rule {
        providers: WHEA,
        id: 18,
        category: EventCategory::HardwareError,
        title: "Fatal hardware error (CPU, memory or PCIe)",
    },
    Rule {
        providers: WHEA,
        id: 19,
        category: EventCategory::HardwareError,
        title: "Corrected CPU or bus hardware error",
    },
    Rule {
        providers: WHEA,
        id: 47,
        category: EventCategory::HardwareError,
        title: "Corrected memory error",
    },
    Rule {
        providers: WHEA,
        id: 17,
        category: EventCategory::HardwareError,
        title: "Corrected PCI Express error",
    },
    Rule {
        providers: DISPLAY,
        id: 4101,
        category: EventCategory::GpuTimeout,
        title: "Graphics driver stopped responding and recovered",
    },
    Rule {
        providers: DISK,
        id: 7,
        category: EventCategory::Disk,
        title: "Bad block found on a drive",
    },
    Rule {
        providers: DISK,
        id: 11,
        category: EventCategory::Disk,
        title: "Drive controller error",
    },
    Rule {
        providers: DISK,
        id: 51,
        category: EventCategory::Disk,
        title: "Error while paging to a drive",
    },
    Rule {
        providers: DISK,
        id: 52,
        category: EventCategory::Disk,
        title: "Drive reports it is about to fail",
    },
    Rule {
        providers: DISK,
        id: 153,
        category: EventCategory::Disk,
        title: "Drive operation had to be retried",
    },
    Rule {
        providers: NTFS,
        id: 55,
        category: EventCategory::Disk,
        title: "File system corruption found",
    },
    Rule {
        providers: NTFS,
        id: 98,
        category: EventCategory::Disk,
        title: "Volume needs a disk check (chkdsk)",
    },
    Rule {
        providers: EVENTLOG,
        id: 6008,
        category: EventCategory::DirtyShutdown,
        title: "The previous shutdown was unexpected",
    },
];

/// One event shown on the page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    /// When it was logged, Unix seconds (UTC).
    pub unix: i64,
    /// Local time offset from UTC at that moment, in seconds (`None`: show UTC).
    pub utc_offset: Option<i32>,
    pub channel: Channel,
    pub provider: String,
    pub id: u32,
    pub level: EventLevel,
    pub category: EventCategory,
    /// What it means, in plain English.
    pub title: &'static str,
    /// Key data: stop code, driver, device, ...; may be empty.
    pub detail: String,
}

impl LogEntry {
    /// (year, month, day, hour, minute, second), local when the offset is known, else UTC.
    pub fn time(&self) -> (i64, u32, u32, u32, u32, u32) {
        let t = self.unix + i64::from(self.utc_offset.unwrap_or(0));
        let (days, rem) = (t.div_euclid(86_400), t.rem_euclid(86_400));
        let (y, m, d) = crate::civil_from_days(days);
        (
            y,
            m,
            d,
            (rem / 3600) as u32,
            (rem / 60 % 60) as u32,
            (rem % 60) as u32,
        )
    }

    /// "2026-10-01 14:05:09", with " UTC" when the local offset is unknown.
    pub fn time_text(&self) -> String {
        let (y, mo, d, h, mi, s) = self.time();
        let utc = if self.utc_offset.is_some() {
            ""
        } else {
            " UTC"
        };
        format!("{y}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}{utc}")
    }

    /// "Kernel-Power 41": the provider without its "Microsoft-Windows-" prefix, and the id.
    pub fn source(&self) -> String {
        let p = self
            .provider
            .strip_prefix("Microsoft-Windows-")
            .unwrap_or(&self.provider);
        format!("{p} {}", self.id)
    }
}

/// What a refresh read: the entries (newest first) and any query that failed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EventLog {
    pub entries: Vec<LogEntry>,
    /// One line per failed query, e.g. "Application log: Access is denied."
    pub errors: Vec<String>,
}

// ---- XPath filters ---------------------------------------------------------------------------

const MS_PER_DAY: u64 = 86_400_000;

/// `TimeCreated` within the last `days` days.
pub fn within(days: u32) -> String {
    format!(
        "TimeCreated[timediff(@SystemTime) <= {}]",
        days as u64 * MS_PER_DAY
    )
}

fn providers_xpath(providers: &[&str]) -> String {
    let names: Vec<String> = providers.iter().map(|p| format!("@Name='{p}'")).collect();
    format!("Provider[{}]", names.join(" or "))
}

/// System log: every categorized event (levels 1-3) in the last `days` days.
pub fn named_filter(days: u32) -> String {
    let mut groups: Vec<(&[&str], Vec<u32>)> = Vec::new();
    for r in RULES {
        match groups.iter_mut().find(|(p, _)| *p == r.providers) {
            Some((_, ids)) => ids.push(r.id),
            None => groups.push((r.providers, vec![r.id])),
        }
    }
    let parts: Vec<String> = groups
        .iter()
        .map(|(p, ids)| {
            let ids: Vec<String> = ids.iter().map(|i| format!("EventID={i}")).collect();
            format!("({} and ({}))", providers_xpath(p), ids.join(" or "))
        })
        .collect();
    format!(
        "*[System[({}) and (Level=1 or Level=2 or Level=3) and {}]]",
        parts.join(" or "),
        within(days)
    )
}

/// System log: every critical or error event in the last `days` days (the categorized ones are
/// dropped when parsing with [`parse_other`]).
pub fn other_filter(days: u32) -> String {
    format!("*[System[(Level=1 or Level=2) and {}]]", within(days))
}

/// System log: Display 4101 in the last `days` days.
pub fn display_tdr_filter(days: u32) -> String {
    format!(
        "*[System[Provider[@Name='Display'] and EventID=4101 and {}]]",
        within(days)
    )
}

/// Application log: Windows Error Reporting LiveKernelEvent 141/117 reports.
pub fn live_kernel_filter(days: u32) -> String {
    let codes: Vec<String> = GPU_LIVE_KERNEL_CODES
        .iter()
        .map(|c| format!("Data[@Name='P1']='{c:x}'"))
        .collect();
    format!(
        "*[System[Provider[@Name='{WER}'] and EventID=1001 and {}] and \
         EventData[Data[@Name='EventName']='LiveKernelEvent'] and EventData[{}]]",
        within(days),
        codes.join(" or ")
    )
}

// ---- Parsing ---------------------------------------------------------------------------------

/// Value of attribute `attr` in the first `<tag ...>` element.
fn attribute<'a>(xml: &'a str, tag: &str, attr: &str) -> Option<&'a str> {
    let start = xml.find(&format!("<{tag} "))?;
    let open = &xml[start..start + xml[start..].find('>')?];
    let i = open.find(&format!("{attr}="))? + attr.len() + 1;
    let q = open[i..].chars().next()?;
    let rest = &open[i + 1..];
    Some(&rest[..rest.find(q)?])
}

/// Text of the first `<tag>` element (`<tag attr=...>` too).
fn element<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
    let mut from = 0;
    while let Some(i) = xml[from..].find(&format!("<{tag}")) {
        let start = from + i + tag.len() + 1;
        let next = xml[start..].chars().next()?;
        if next == '>' || next == ' ' {
            let body = &xml[start + xml[start..].find('>')? + 1..];
            return Some(&body[..body.find(&format!("</{tag}>"))?]);
        }
        from = start;
    }
    None
}

/// Every `<Data>` element of the event as (name, unescaped value), in order.
pub fn data_items(xml: &str) -> Vec<(Option<String>, String)> {
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(i) = rest.find("<Data") {
        rest = &rest[i + 5..];
        let Some(open_end) = rest.find('>') else {
            break;
        };
        let open = &rest[..open_end];
        if !(open.is_empty() || open.starts_with(' ') || open.starts_with('/')) {
            continue; // e.g. <DataItem>
        }
        let name = open.find("Name=").and_then(|n| {
            let q = open[n + 5..].chars().next()?;
            let v = &open[n + 6..];
            Some(v[..v.find(q)?].to_string())
        });
        if open.ends_with('/') {
            out.push((name, String::new()));
            rest = &rest[open_end + 1..];
            continue;
        }
        let body = &rest[open_end + 1..];
        let Some(end) = body.find("</Data>") else {
            break;
        };
        out.push((name, unescape_xml(&body[..end])));
        rest = &body[end..];
    }
    out
}

/// Value of the first unnamed `<Data>` (Display 4101 driver, disk device, ...).
fn first_unnamed(items: &[(Option<String>, String)], index: usize) -> Option<&str> {
    items
        .iter()
        .filter(|(n, _)| n.is_none())
        .nth(index)
        .map(|(_, v)| v.trim())
        .filter(|v| !v.is_empty())
}

/// Up to four "Name: value" pairs (or bare values), skipping empty and very long ones.
fn summary(items: &[(Option<String>, String)]) -> String {
    let parts: Vec<String> = items
        .iter()
        .filter(|(_, v)| !v.trim().is_empty() && v.len() <= 120)
        .take(4)
        .map(|(n, v)| match n {
            Some(n) => format!("{n}: {}", v.trim()),
            None => v.trim().to_string(),
        })
        .collect();
    parts.join("; ")
}

/// Hex code of a LiveKernelEvent report (`P1`, e.g. "141"), or `None` for other reports.
pub fn live_kernel_code(xml: &str) -> Option<u32> {
    if event_data(xml, "EventName")?.trim() != "LiveKernelEvent" {
        return None;
    }
    u32::from_str_radix(event_data(xml, "P1")?.trim(), 16).ok()
}

/// Local (year, month, day, hour, minute) from the live dump's file name, e.g.
/// `WATCHDOG-20261015-0915.dmp`. WER often sends reports days later in a batch, so this is when
/// the timeout actually happened; the event's own time is when it was reported.
pub fn dump_local_time(xml: &str) -> Option<(i64, u32, u32, u32, u32)> {
    let name = dump_name(xml)?;
    let stem = name.strip_suffix(".dmp")?;
    let mut parts = stem.rsplit('-');
    let (hm, ymd) = (parts.next()?, parts.next()?);
    if hm.len() != 4
        || ymd.len() != 8
        || !(hm.chars().chain(ymd.chars())).all(|c| c.is_ascii_digit())
    {
        return None;
    }
    let num = |s: &str| s.parse::<u32>().ok();
    let (m, d, h, mi) = (
        num(&ymd[4..6])?,
        num(&ymd[6..8])?,
        num(&hm[..2])?,
        num(&hm[2..])?,
    );
    let valid = (1..=12).contains(&m) && (1..=31).contains(&d) && h < 24 && mi < 60;
    valid.then_some((i64::from(num(&ymd[..4])?), m, d, h, mi))
}

/// File name of the first `.dmp` in a WER report's `AttachedFiles`.
fn dump_name(xml: &str) -> Option<String> {
    event_data(xml, "AttachedFiles")?
        .lines()
        .map(str::trim)
        .find(|l| l.ends_with(".dmp"))
        .and_then(|l| l.rsplit('\\').next())
        .map(str::to_string)
}

/// `ReportId` of a Windows Error Reporting event: WER can log the same report more than once.
pub fn report_id(xml: &str) -> Option<String> {
    event_data(xml, "ReportId").filter(|s| !s.is_empty())
}

/// The rule matching a provider and id.
fn rule(provider: &str, id: u32) -> Option<&'static Rule> {
    RULES
        .iter()
        .find(|r| r.id == id && r.providers.iter().any(|p| p.eq_ignore_ascii_case(provider)))
}

/// Key data of a categorized event.
fn detail(category: EventCategory, xml: &str, items: &[(Option<String>, String)]) -> String {
    let stop = || {
        parse_event_xml(xml)
            .and_then(|e| e.code)
            .filter(|&c| c != 0)
            .map(|c| format!("Stop code {}", stop_code_label(c)))
    };
    match category {
        EventCategory::UnexpectedShutdown => {
            let mut parts: Vec<String> = stop().into_iter().collect();
            if event_data(xml, "PowerButtonTimestamp").is_some_and(|v| v.trim() != "0") {
                parts.push("the power button was held down".into());
            }
            if parts.is_empty() {
                "No blue screen recorded: the PC lost power, froze or was reset.".into()
            } else {
                parts.join("; ")
            }
        }
        EventCategory::BlueScreen => stop().unwrap_or_default(),
        EventCategory::GpuTimeout => first_unnamed(items, 0)
            .map(|d| format!("Driver: {d}"))
            .unwrap_or_default(),
        EventCategory::Disk if first_unnamed(items, 0).is_some() => {
            format!("Device: {}", first_unnamed(items, 0).unwrap_or_default())
        }
        EventCategory::DirtyShutdown => {
            // Time and date of the previous shutdown, with left-to-right marks around them.
            let clean = |i| first_unnamed(items, i).map(|s| s.replace('\u{200e}', ""));
            match (clean(0), clean(1)) {
                (Some(t), Some(d)) => format!("Previous shutdown at {t} on {d}"),
                _ => String::new(),
            }
        }
        _ => summary(items),
    }
}

/// Fields every entry has: provider, id, level value and time.
fn header(xml: &str) -> Option<(String, u32, u32, i64)> {
    let provider = attribute(xml, "Provider", "Name")?.to_string();
    let id: u32 = element(xml, "EventID")?.trim().parse().ok()?;
    let level: u32 = element(xml, "Level")
        .and_then(|l| l.trim().parse().ok())
        .unwrap_or(0);
    let t = attribute(xml, "TimeCreated", "SystemTime")?;
    let unix = crate::recording::parse_iso_utc(t)? as i64;
    Some((provider, id, level, unix))
}

/// A categorized System event (or a LiveKernelEvent GPU timeout from the Application log);
/// `None` for anything else.
pub fn parse_named(xml: &str, channel: Channel) -> Option<LogEntry> {
    let (provider, id, level, unix) = header(xml)?;
    let items = data_items(xml);
    if provider == WER && id == 1001 {
        let code = live_kernel_code(xml)?;
        if !GPU_LIVE_KERNEL_CODES.contains(&base_stop_code(code)) {
            return None;
        }
        let mut detail = format!("LiveKernelEvent {}", stop_code_label(code));
        if let Some(d) = dump_name(xml) {
            detail.push_str(&format!("; dump {d}"));
        }
        return Some(LogEntry {
            unix,
            utc_offset: None,
            channel,
            provider,
            id,
            level: EventLevel::Error,
            category: EventCategory::GpuTimeout,
            title: if code == 0x141 {
                "Graphics card hung and was reset"
            } else {
                "Graphics driver timed out and was reset"
            },
            detail,
        });
    }
    let r = rule(&provider, id)?;
    Some(LogEntry {
        unix,
        utc_offset: None,
        channel,
        level: EventLevel::from_xml(level).unwrap_or(EventLevel::Error),
        category: r.category,
        title: r.title,
        detail: detail(r.category, xml, &items),
        provider,
        id,
    })
}

/// A critical/error System event that no rule covers; `None` for categorized events (read by
/// [`parse_named`]) and lower levels.
pub fn parse_other(xml: &str) -> Option<LogEntry> {
    let (provider, id, level, unix) = header(xml)?;
    if rule(&provider, id).is_some() {
        return None;
    }
    let level = EventLevel::from_xml(level).filter(|&l| l >= EventLevel::Error)?;
    Some(LogEntry {
        unix,
        utc_offset: None,
        channel: Channel::System,
        provider,
        id,
        level,
        category: EventCategory::Other,
        title: if level == EventLevel::Critical {
            "Other critical event"
        } else {
            "Other error"
        },
        detail: summary(&data_items(xml)),
    })
}

/// Newest first, at most [`TOTAL_CAP`] entries.
pub fn sort_and_cap(entries: &mut Vec<LogEntry>) {
    entries.sort_by_key(|e| std::cmp::Reverse(e.unix));
    entries.truncate(TOTAL_CAP);
}

// ---- Filtering -------------------------------------------------------------------------------

/// The page's filters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filter {
    pub category: Option<EventCategory>,
    pub level: Option<EventLevel>,
    /// Only events from the last this-many seconds.
    pub within_secs: i64,
    /// Case-insensitive text matched against title, source and detail.
    pub text: String,
}

impl Default for Filter {
    fn default() -> Self {
        Filter {
            category: None,
            level: None,
            within_secs: i64::from(VIEW_DAYS) * 86_400,
            text: String::new(),
        }
    }
}

impl Filter {
    pub fn matches(&self, e: &LogEntry, now_unix: i64) -> bool {
        if self.category.is_some_and(|c| c != e.category)
            || self.level.is_some_and(|l| l != e.level)
            || now_unix - e.unix > self.within_secs
        {
            return false;
        }
        let q = self.text.trim().to_lowercase();
        q.is_empty()
            || [
                e.title,
                e.category.name(),
                &e.source(),
                &e.detail,
                e.level.name(),
            ]
            .iter()
            .any(|s| s.to_lowercase().contains(&q))
    }
}

/// Plain-text list for the clipboard, one event per line.
pub fn to_text<'a>(entries: impl IntoIterator<Item = &'a LogEntry>) -> String {
    let mut out = String::new();
    for e in entries {
        out.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}",
            e.time_text(),
            e.level.name(),
            e.category.name(),
            e.source(),
            e.title
        ));
        if !e.detail.is_empty() {
            out.push_str(&format!(" ({})", e.detail));
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const DISPLAY_4101: &str = "<Event xmlns='http://schemas.microsoft.com/win/2004/08/events/event'><System>\
        <Provider Name='Display'/><EventID Qualifiers='0'>4101</EventID><Version>0</Version><Level>3</Level>\
        <Task>0</Task><Opcode>0</Opcode><Keywords>0x80000000000000</Keywords>\
        <TimeCreated SystemTime='2026-10-10T20:15:30.1234567Z'/><EventRecordID>1234</EventRecordID>\
        <Correlation/><Execution ProcessID='0' ThreadID='0'/><Channel>System</Channel><Computer>PC</Computer>\
        <Security/></System><EventData><Data>nvlddmkm</Data><Data></Data></EventData></Event>";

    // Recorded on Windows 11 26300 (paths shortened).
    const LIVE_KERNEL_141: &str = "<Event xmlns='http://schemas.microsoft.com/win/2004/08/events/event'><System>\
        <Provider Name='Windows Error Reporting' Guid='{0ead09bd-2157-539a-8d6d-c87f95b64d70}'/><EventID>1001</EventID>\
        <Version>0</Version><Level>4</Level><Task>0</Task><Opcode>0</Opcode><Keywords>0x8000000000000000</Keywords>\
        <TimeCreated SystemTime='2026-10-19T16:13:18.4498342Z'/><EventRecordID>306579</EventRecordID><Correlation/>\
        <Execution ProcessID='25676' ThreadID='30576'/><Channel>Application</Channel><Computer>PC</Computer>\
        <Security UserID='S-1-5-18'/></System><EventData><Data Name='Bucket'></Data><Data Name='BucketType'>0</Data>\
        <Data Name='EventName'>LiveKernelEvent</Data><Data Name='Response'>Not available</Data><Data Name='CabId'>0</Data>\
        <Data Name='P1'>141</Data><Data Name='P2'>aaaaaaaaaaaaaaaa</Data><Data Name='P6'>10_0_26300</Data>\
        <Data Name='AttachedFiles'>\n\\\\?\\C:\\WINDOWS\\LiveKernelReports\\WATCHDOG\\WATCHDOG-20261015-0915.dmp\n\
        \\\\?\\C:\\WINDOWS\\SystemTemp\\WER-85363171-0.sysdata.xml</Data>\
        <Data Name='ReportId'>994fb83c-91e6-487d-8c32-f47263dba4c0</Data><Data Name='ReportStatus'>2049</Data></EventData></Event>";

    const KERNEL_POWER_41: &str = "<Event xmlns='http://schemas.microsoft.com/win/2004/08/events/event'><System>\
        <Provider Name='Microsoft-Windows-Kernel-Power' Guid='{331c3b3a-2005-44c2-ac5e-77220c37d6b4}'/>\
        <EventID>41</EventID><Version>9</Version><Level>1</Level><Task>63</Task>\
        <TimeCreated SystemTime='2026-10-01T12:34:56.1234567Z'/><Channel>System</Channel></System><EventData>\
        <Data Name='BugcheckCode'>0</Data><Data Name='BugcheckParameter1'>0x0</Data>\
        <Data Name='SleepInProgress'>0</Data><Data Name='PowerButtonTimestamp'>0</Data></EventData></Event>";

    const DISK_153: &str = "<Event xmlns='http://schemas.microsoft.com/win/2004/08/events/event'><System>\
        <Provider Name='disk'/><EventID Qualifiers='32772'>153</EventID><Version>0</Version><Level>3</Level>\
        <TimeCreated SystemTime='2026-10-05T08:00:00.000Z'/><Channel>System</Channel></System><EventData>\
        <Data>\\Device\\Harddisk1\\DR1</Data><Data>0x2a18c00</Data><Data>1</Data><Binary>0F01</Binary></EventData></Event>";

    const EVENTLOG_6008: &str = "<Event xmlns='http://schemas.microsoft.com/win/2004/08/events/event'><System>\
        <Provider Name='EventLog'/><EventID Qualifiers='32768'>6008</EventID><Level>2</Level>\
        <TimeCreated SystemTime='2026-10-02T07:00:00.5Z'/><Channel>System</Channel></System><EventData>\
        <Data>\u{200e}9:41:07 PM</Data><Data>\u{200e}10/1/2026</Data><Data></Data><Data></Data><Data>1</Data>\
        <Data></Data></EventData></Event>";

    const UNKNOWN: &str = "<Event xmlns='http://schemas.microsoft.com/win/2004/08/events/event'><System>\
        <Provider Name='Microsoft-Windows-DistributedCOM' Guid='{1b562e86-b7aa-4131-badc-b6f3a001407e}' EventSourceName='DCOM'/>\
        <EventID Qualifiers='0'>10016</EventID><Level>2</Level>\
        <TimeCreated SystemTime='2026-10-03T10:00:00Z'/><Channel>System</Channel></System><EventData>\
        <Data Name='param1'>application-specific</Data><Data Name='param2'>Local</Data><Data Name='param3'>Activation</Data>\
        </EventData></Event>";

    #[test]
    fn gpu_timeouts() {
        let e = parse_named(DISPLAY_4101, Channel::System).unwrap();
        assert_eq!(e.category, EventCategory::GpuTimeout);
        assert_eq!(e.level, EventLevel::Warning);
        assert_eq!(e.detail, "Driver: nvlddmkm");
        assert_eq!(e.source(), "Display 4101");
        assert_eq!(e.time_text(), "2026-10-10 20:15:30 UTC");

        assert_eq!(live_kernel_code(LIVE_KERNEL_141), Some(0x141));
        assert_eq!(
            dump_local_time(LIVE_KERNEL_141),
            Some((2026, 10, 15, 9, 15))
        );
        assert_eq!(dump_local_time(DISPLAY_4101), None);
        assert_eq!(
            report_id(LIVE_KERNEL_141).as_deref(),
            Some("994fb83c-91e6-487d-8c32-f47263dba4c0")
        );
        let e = parse_named(LIVE_KERNEL_141, Channel::Application).unwrap();
        assert_eq!(e.category, EventCategory::GpuTimeout);
        assert_eq!(
            e.level,
            EventLevel::Error,
            "Information-level reports count as errors"
        );
        assert_eq!(e.title, "Graphics card hung and was reset");
        assert_eq!(
            e.detail,
            "LiveKernelEvent VIDEO_ENGINE_TIMEOUT_DETECTED (0x141); dump WATCHDOG-20261015-0915.dmp"
        );
        // Other live dumps (e.g. 0x193) aren't GPU timeouts.
        let other = LIVE_KERNEL_141.replace("<Data Name='P1'>141<", "<Data Name='P1'>193<");
        assert_eq!(parse_named(&other, Channel::Application), None);
    }

    #[test]
    fn crashes_disks_and_shutdowns() {
        let e = parse_named(KERNEL_POWER_41, Channel::System).unwrap();
        assert_eq!(
            (e.category, e.level, e.id),
            (EventCategory::UnexpectedShutdown, EventLevel::Critical, 41)
        );
        assert!(e.detail.starts_with("No blue screen recorded"));
        assert_eq!(e.source(), "Kernel-Power 41");
        let bsod = KERNEL_POWER_41.replace(
            "<Data Name='BugcheckCode'>0<",
            "<Data Name='BugcheckCode'>209<",
        );
        assert_eq!(
            parse_named(&bsod, Channel::System).unwrap().detail,
            "Stop code DRIVER_IRQL_NOT_LESS_OR_EQUAL (0xD1)"
        );

        let e = parse_named(DISK_153, Channel::System).unwrap();
        assert_eq!(
            (e.category, e.level),
            (EventCategory::Disk, EventLevel::Warning)
        );
        assert_eq!(e.detail, r"Device: \Device\Harddisk1\DR1");
        assert_eq!(e.title, "Drive operation had to be retried");

        let e = parse_named(EVENTLOG_6008, Channel::System).unwrap();
        assert_eq!(e.category, EventCategory::DirtyShutdown);
        assert_eq!(e.detail, "Previous shutdown at 9:41:07 PM on 10/1/2026");

        // Categorized events are skipped by the "other" parser, and vice versa.
        assert_eq!(parse_other(KERNEL_POWER_41), None);
        assert_eq!(parse_named(UNKNOWN, Channel::System), None);
        let e = parse_other(UNKNOWN).unwrap();
        assert_eq!(
            (e.category, e.level),
            (EventCategory::Other, EventLevel::Error)
        );
        assert_eq!(e.source(), "DistributedCOM 10016");
        assert_eq!(
            e.detail,
            "param1: application-specific; param2: Local; param3: Activation"
        );
        assert_eq!(
            parse_other(&UNKNOWN.replace("<Level>2<", "<Level>3<")),
            None
        );
        assert_eq!(parse_named("garbage", Channel::System), None);
    }

    #[test]
    fn rules_are_consistent() {
        for (i, r) in RULES.iter().enumerate() {
            assert!(!r.title.is_empty());
            assert!(
                RULES[..i]
                    .iter()
                    .all(|o| o.id != r.id || o.providers != r.providers),
                "duplicate rule {}",
                r.id
            );
        }
        assert_eq!(rule("DISK", 7).unwrap().category, EventCategory::Disk);
        assert_eq!(
            rule("BugCheck", 1001).unwrap().category,
            EventCategory::BlueScreen
        );
        assert!(rule("disk", 8).is_none());
    }

    #[test]
    fn filters() {
        let f = named_filter(30);
        assert!(f.starts_with(
            "*[System[((Provider[@Name='Microsoft-Windows-Kernel-Power'] and (EventID=41))"
        ));
        assert!(f.contains("(Provider[@Name='disk'] and (EventID=7 or EventID=11 or EventID=51 or EventID=52 or EventID=153))"));
        assert!(f.ends_with(
            "(Level=1 or Level=2 or Level=3) and TimeCreated[timediff(@SystemTime) <= 2592000000]]]"
        ));
        assert_eq!(f.matches('[').count(), f.matches(']').count());
        assert!(
            live_kernel_filter(30)
                .contains("EventData[Data[@Name='P1']='141' or Data[@Name='P1']='117']")
        );
    }

    #[test]
    fn data_and_time() {
        let items = data_items(
            "<EventData><Data Name='a'>x &amp; y</Data><Data/><DataItem>z</DataItem><Data>q</Data></EventData>",
        );
        assert_eq!(
            items,
            vec![
                (Some("a".into()), "x & y".into()),
                (None, String::new()),
                (None, "q".into())
            ]
        );
        let mut e = parse_named(DISK_153, Channel::System).unwrap();
        e.utc_offset = Some(-4 * 3600);
        assert_eq!(e.time_text(), "2026-10-05 04:00:00");
        assert_eq!(e.time(), (2026, 10, 5, 4, 0, 0));
    }

    #[test]
    fn filtering_and_copy() {
        let mut list: Vec<LogEntry> = [DISPLAY_4101, KERNEL_POWER_41, DISK_153]
            .iter()
            .filter_map(|x| parse_named(x, Channel::System))
            .collect();
        list.extend(parse_other(UNKNOWN));
        sort_and_cap(&mut list);
        assert_eq!(list[0].id, 4101, "newest first");
        let now = list[0].unix + 3600;
        let count = |f: &Filter| list.iter().filter(|e| f.matches(e, now)).count();
        assert_eq!(count(&Filter::default()), 4);
        let day = Filter {
            within_secs: 86_400,
            ..Default::default()
        };
        assert_eq!(count(&day), 1);
        let disk = Filter {
            category: Some(EventCategory::Disk),
            ..Default::default()
        };
        assert_eq!(count(&disk), 1);
        let critical = Filter {
            level: Some(EventLevel::Critical),
            ..Default::default()
        };
        assert_eq!(count(&critical), 1);
        let text = Filter {
            text: "NVLDDMKM".into(),
            ..Default::default()
        };
        assert_eq!(count(&text), 1);
        let t = to_text(&list[..1]);
        assert_eq!(
            t,
            "2026-10-10 20:15:30 UTC\tWarning\tGPU driver timeout\tDisplay 4101\tGraphics driver stopped responding and recovered (Driver: nvlddmkm)\n"
        );
    }
}
