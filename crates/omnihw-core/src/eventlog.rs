//! Event Log page model: the kernel and system messages that matter for PC health, from the
//! systemd journal, sorted into plain-English categories.
//!
//! The journal is read with `journalctl --output=json` on a worker thread
//! ([`crate::platform::events_viewer`]); this module turns its entries into [`LogEntry`]s.
//! Kernel messages carry no event ids, so they're recognized by their text. The patterns follow
//! the kernel's own wording: `mce` / `EDAC` / `AER` for hardware errors, `amdgpu` / `i915` /
//! `NVRM: Xid` for GPU hangs, the block layer, libata, NVMe and file system drivers for disk
//! problems, the OOM killer, and the x86 thermal throttling notices.

/// Days of journal the page can show.
pub const VIEW_DAYS: u32 = 30;
/// Most kernel messages read (newest first).
pub const KERNEL_CAP: usize = 5000;
/// Most crash and out-of-memory records read from systemd-coredump and systemd-oomd.
pub const SERVICE_CAP: usize = 500;
/// Most uncategorized entries kept.
pub const OTHER_CAP: usize = 150;
/// Most entries kept in total.
pub const TOTAL_CAP: usize = 500;

/// Severity, from the journal priority (crit and above, err, warning).
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

    /// From a syslog priority: 0-2 (emerg, alert, crit), 3 (err), 4 (warning).
    pub fn from_priority(p: u8) -> Option<EventLevel> {
        match p {
            0..=2 => Some(EventLevel::Critical),
            3 => Some(EventLevel::Error),
            4 => Some(EventLevel::Warning),
            _ => None,
        }
    }
}

/// Plain-English group of an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventCategory {
    KernelCrash,
    HardwareError,
    GpuTimeout,
    Disk,
    OutOfMemory,
    AppCrash,
    Overheating,
    Other,
}

impl EventCategory {
    pub const ALL: [EventCategory; 8] = [
        EventCategory::KernelCrash,
        EventCategory::HardwareError,
        EventCategory::GpuTimeout,
        EventCategory::Disk,
        EventCategory::OutOfMemory,
        EventCategory::AppCrash,
        EventCategory::Overheating,
        EventCategory::Other,
    ];

    pub fn name(self) -> &'static str {
        match self {
            EventCategory::KernelCrash => "Kernel crash or hang",
            EventCategory::HardwareError => "Hardware error",
            EventCategory::GpuTimeout => "GPU hang or reset",
            EventCategory::Disk => "Disk problem",
            EventCategory::OutOfMemory => "Out of memory",
            EventCategory::AppCrash => "App crash",
            EventCategory::Overheating => "Overheating",
            EventCategory::Other => "Other critical/error",
        }
    }
}

/// A known message: every pattern must occur in the lower-cased text.
struct Rule {
    /// `None` matches any sender; else the journal's `SYSLOG_IDENTIFIER`.
    source: Option<&'static str>,
    patterns: &'static [&'static str],
    category: EventCategory,
    /// Overrides the logged priority (some hardware errors are logged as warnings or info).
    level: Option<EventLevel>,
    title: &'static str,
}

const fn kernel(
    patterns: &'static [&'static str],
    category: EventCategory,
    level: Option<EventLevel>,
    title: &'static str,
) -> Rule {
    Rule {
        source: Some("kernel"),
        patterns,
        category,
        level,
        title,
    }
}

use EventCategory as C;
use EventLevel as L;

/// First match wins, so the specific patterns come before the general ones.
const RULES: &[Rule] = &[
    // Kernel crashes and hangs.
    kernel(
        &["kernel panic"],
        C::KernelCrash,
        Some(L::Critical),
        "Kernel panic",
    ),
    kernel(
        &["bug: unable to handle"],
        C::KernelCrash,
        Some(L::Critical),
        "Kernel error (bad memory access)",
    ),
    kernel(
        &["general protection fault"],
        C::KernelCrash,
        Some(L::Critical),
        "Kernel error (general protection fault)",
    ),
    kernel(
        &["kernel bug at"],
        C::KernelCrash,
        Some(L::Critical),
        "Kernel bug",
    ),
    kernel(
        &["oops:"],
        C::KernelCrash,
        Some(L::Critical),
        "Kernel error (oops)",
    ),
    kernel(
        &["soft lockup"],
        C::KernelCrash,
        Some(L::Error),
        "A CPU got stuck (soft lockup)",
    ),
    kernel(
        &["hard lockup"],
        C::KernelCrash,
        Some(L::Critical),
        "A CPU got stuck (hard lockup)",
    ),
    kernel(
        &["rcu", "stall"],
        C::KernelCrash,
        Some(L::Error),
        "The kernel stalled (RCU stall)",
    ),
    kernel(
        &["blocked for more than"],
        C::KernelCrash,
        Some(L::Warning),
        "A task hung for over two minutes",
    ),
    // Hardware errors.
    kernel(
        &["edac", " ue "],
        C::HardwareError,
        Some(L::Critical),
        "Uncorrected memory error",
    ),
    kernel(
        &["edac", " ce "],
        C::HardwareError,
        Some(L::Warning),
        "Corrected memory error",
    ),
    kernel(
        &["[hardware error]"],
        C::HardwareError,
        Some(L::Error),
        "Machine check (CPU, cache or memory hardware error)",
    ),
    kernel(
        &["machine check"],
        C::HardwareError,
        Some(L::Error),
        "Machine check (CPU, cache or memory hardware error)",
    ),
    kernel(
        &["pcie bus error", "severity=corrected"],
        C::HardwareError,
        Some(L::Warning),
        "Corrected PCI Express error",
    ),
    kernel(
        &["pcie bus error"],
        C::HardwareError,
        Some(L::Error),
        "Uncorrected PCI Express error",
    ),
    // GPU hangs and resets.
    kernel(
        &["fallen off the bus"],
        C::GpuTimeout,
        Some(L::Critical),
        "The graphics card dropped off the PCIe bus",
    ),
    kernel(
        &["nvrm: xid"],
        C::GpuTimeout,
        Some(L::Error),
        "NVIDIA graphics error (Xid)",
    ),
    kernel(
        &["amdgpu", "timeout"],
        C::GpuTimeout,
        Some(L::Error),
        "AMD graphics stopped responding",
    ),
    kernel(
        &["amdgpu", "gpu reset"],
        C::GpuTimeout,
        Some(L::Error),
        "AMD graphics was reset",
    ),
    kernel(
        &["i915", "gpu hang"],
        C::GpuTimeout,
        Some(L::Error),
        "Intel graphics hung",
    ),
    kernel(
        &["i915", "resetting"],
        C::GpuTimeout,
        Some(L::Warning),
        "Intel graphics was reset",
    ),
    kernel(
        &["xe ", "gt", "reset"],
        C::GpuTimeout,
        Some(L::Warning),
        "Intel graphics was reset",
    ),
    kernel(
        &["nouveau", "fifo"],
        C::GpuTimeout,
        Some(L::Error),
        "Nouveau graphics error",
    ),
    // Drives and file systems.
    kernel(
        &["nvme", "controller is down"],
        C::Disk,
        Some(L::Critical),
        "NVMe drive dropped out",
    ),
    kernel(
        &["nvme", "timeout"],
        C::Disk,
        Some(L::Error),
        "NVMe drive stopped responding",
    ),
    kernel(
        &["medium error"],
        C::Disk,
        Some(L::Error),
        "Bad sector on a drive",
    ),
    kernel(
        &["unrecovered read error"],
        C::Disk,
        Some(L::Error),
        "Bad sector on a drive",
    ),
    kernel(
        &["i/o error"],
        C::Disk,
        Some(L::Error),
        "Drive read/write error",
    ),
    kernel(
        &["ata", "failed command"],
        C::Disk,
        Some(L::Error),
        "SATA command failed",
    ),
    kernel(
        &["ata", "hard resetting link"],
        C::Disk,
        Some(L::Warning),
        "SATA link was reset",
    ),
    kernel(
        &["ext4-fs error"],
        C::Disk,
        Some(L::Error),
        "File system error",
    ),
    kernel(
        &["btrfs", "error"],
        C::Disk,
        Some(L::Error),
        "File system error",
    ),
    kernel(
        &["xfs", "corruption"],
        C::Disk,
        Some(L::Error),
        "File system error",
    ),
    kernel(
        &["fat-fs", "error"],
        C::Disk,
        Some(L::Error),
        "File system error",
    ),
    // Memory and temperature.
    kernel(
        &["out of memory: killed process"],
        C::OutOfMemory,
        Some(L::Error),
        "Ran out of memory; a process was killed",
    ),
    kernel(
        &["temperature above threshold"],
        C::Overheating,
        Some(L::Warning),
        "CPU overheated and slowed down",
    ),
    kernel(
        &["critical temperature reached"],
        C::Overheating,
        Some(L::Critical),
        "Critical temperature: the system shut down",
    ),
    // Services.
    Rule {
        source: Some("systemd-coredump"),
        patterns: &["dumped core"],
        category: C::AppCrash,
        level: Some(L::Error),
        title: "An app crashed",
    },
    Rule {
        source: Some("systemd-oomd"),
        patterns: &["killed"],
        category: C::OutOfMemory,
        level: Some(L::Error),
        title: "Ran out of memory; systemd-oomd stopped an app",
    },
];

/// Whether `e` is a hardware error (the memory and disk tests report these).
pub fn is_hardware_error(e: &LogEntry) -> bool {
    e.category == EventCategory::HardwareError
}

/// One event shown on the page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    /// When it was logged, Unix seconds (UTC).
    pub unix: i64,
    /// Local time offset from UTC at that moment, in seconds (`None`: show UTC).
    pub utc_offset: Option<i32>,
    /// Who logged it (`SYSLOG_IDENTIFIER`), e.g. "kernel" or "systemd-coredump".
    pub source: String,
    pub level: EventLevel,
    pub category: EventCategory,
    /// What it means, in plain English.
    pub title: &'static str,
    /// The logged message (first line, trimmed).
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
}

/// What a refresh read: the entries (newest first) and any query that failed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EventLog {
    pub entries: Vec<LogEntry>,
    /// One line per failed query, e.g. "kernel messages: ...".
    pub errors: Vec<String>,
}

// ---- Journal entries ---------------------------------------------------------------------------

/// The fields of one `journalctl --output=json` line that the page uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalEntry {
    pub unix: i64,
    pub priority: u8,
    pub identifier: String,
    pub message: String,
}

/// A JSON field that journalctl writes as a string, or as an array of bytes when it isn't
/// valid UTF-8.
fn text_field(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Array(a) => {
            let bytes: Vec<u8> = a
                .iter()
                .filter_map(|b| b.as_u64())
                .map(|b| b as u8)
                .collect();
            Some(String::from_utf8_lossy(&bytes).into_owned())
        }
        _ => None,
    }
}

/// Parse one line of `journalctl --output=json`.
pub fn parse_journal_line(line: &str) -> Option<JournalEntry> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    let micros: i64 = text_field(v.get("__REALTIME_TIMESTAMP")?)?.parse().ok()?;
    Some(JournalEntry {
        unix: micros / 1_000_000,
        priority: v
            .get("PRIORITY")
            .and_then(text_field)
            .and_then(|p| p.parse().ok())
            .unwrap_or(6),
        identifier: v
            .get("SYSLOG_IDENTIFIER")
            .and_then(text_field)
            .unwrap_or_default(),
        message: text_field(v.get("MESSAGE")?)?,
    })
}

/// The page's entry for a journal entry: a known message in its category, or an unknown one
/// that was logged as an error or worse (`Other`). `None` for anything else.
pub fn classify(j: &JournalEntry) -> Option<LogEntry> {
    let lower = j.message.to_lowercase();
    let rule = RULES.iter().find(|r| {
        r.source.is_none_or(|s| s == j.identifier) && r.patterns.iter().all(|p| lower.contains(p))
    });
    let (category, level, title) = match rule {
        Some(r) => (
            r.category,
            r.level.or(EventLevel::from_priority(j.priority))?,
            r.title,
        ),
        None if j.priority <= 3 => (
            EventCategory::Other,
            EventLevel::from_priority(j.priority)?,
            "Logged as an error",
        ),
        None => return None,
    };
    let detail = j.message.lines().next().unwrap_or_default().trim();
    Some(LogEntry {
        unix: j.unix,
        utc_offset: None,
        source: j.identifier.clone(),
        level,
        category,
        title,
        detail: truncate(detail, 300),
    })
}

fn truncate(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_string(),
    }
}

/// Classified entries of a journalctl JSON output, at most `OTHER_CAP` of them uncategorized.
pub fn parse_journal(json_lines: &str) -> Vec<LogEntry> {
    let mut others = 0;
    json_lines
        .lines()
        .filter_map(parse_journal_line)
        .filter_map(|j| classify(&j))
        .filter(|e| {
            if e.category != EventCategory::Other {
                return true;
            }
            others += 1;
            others <= OTHER_CAP
        })
        .collect()
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
                &e.source,
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
            e.source,
            e.title
        ));
        if !e.detail.is_empty() {
            out.push_str(&format!(" ({})", e.detail));
        }
        out.push('\n');
    }
    out
}

/// "2 hardware errors were logged during the test: Corrected memory error (2)." for a test's
/// result, or `None` when there were none.
pub fn during_test_text(events: &[LogEntry]) -> Option<String> {
    if events.is_empty() {
        return None;
    }
    let mut titles: Vec<(&str, usize)> = Vec::new();
    for e in events {
        match titles.iter_mut().find(|(t, _)| *t == e.title) {
            Some((_, n)) => *n += 1,
            None => titles.push((e.title, 1)),
        }
    }
    let list = titles
        .iter()
        .map(|(t, n)| {
            if *n > 1 {
                format!("{t} ({n})")
            } else {
                t.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    let n = events.len();
    Some(format!(
        "{n} hardware {} logged during the test: {list}. Errors under load point at unstable memory settings (XMP/EXPO), CPU overclocks or undervolts, or failing hardware.",
        if n == 1 { "error was" } else { "errors were" }
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(us: i64, prio: u8, id: &str, msg: &str) -> String {
        serde_json::json!({
            "__REALTIME_TIMESTAMP": us.to_string(),
            "PRIORITY": prio.to_string(),
            "SYSLOG_IDENTIFIER": id,
            "MESSAGE": msg,
        })
        .to_string()
    }

    const T: i64 = 1_791_000_000;

    #[test]
    fn journal_lines() {
        let j = parse_journal_line(&line(T * 1_000_000 + 5, 3, "kernel", "x")).unwrap();
        assert_eq!(
            j,
            JournalEntry {
                unix: T,
                priority: 3,
                identifier: "kernel".into(),
                message: "x".into()
            }
        );
        // Non-UTF-8 messages come as byte arrays.
        let bytes = r#"{"__REALTIME_TIMESTAMP":"1000000","MESSAGE":[104,105,255]}"#;
        let j = parse_journal_line(bytes).unwrap();
        assert_eq!((j.unix, j.priority), (1, 6));
        assert!(j.message.starts_with("hi"));
        assert!(parse_journal_line("not json").is_none());
    }

    #[test]
    fn categories() {
        let c = |prio, id: &str, msg: &str| {
            classify(&parse_journal_line(&line(T * 1_000_000, prio, id, msg)).unwrap())
        };
        let gpu = c(
            3,
            "kernel",
            "amdgpu 0000:03:00.0: amdgpu: ring gfx_0.0.0 timeout, signaled seq=1, emitted seq=3",
        )
        .unwrap();
        assert_eq!((gpu.category, gpu.level), (C::GpuTimeout, L::Error));
        let xid = c(
            4,
            "kernel",
            "NVRM: Xid (PCI:0000:01:00): 79, pid=1234, GPU has fallen off the bus.",
        )
        .unwrap();
        assert_eq!((xid.category, xid.level), (C::GpuTimeout, L::Critical));
        let ce = c(
            4,
            "kernel",
            "EDAC MC0: 1 CE memory read error on CPU_SrcID#0_Ha#0_Chan#1_DIMM#0",
        )
        .unwrap();
        assert_eq!(
            (ce.category, ce.title),
            (C::HardwareError, "Corrected memory error")
        );
        let mce = c(
            2,
            "kernel",
            "mce: [Hardware Error]: Machine check events logged",
        )
        .unwrap();
        assert_eq!(mce.category, C::HardwareError);
        let aer = c(4, "kernel", "pcieport 0000:00:01.1: PCIe Bus Error: severity=Corrected, type=Physical Layer, (Receiver ID)").unwrap();
        assert_eq!(aer.title, "Corrected PCI Express error");
        let io = c(
            3,
            "kernel",
            "I/O error, dev sda, sector 123456 op 0x0:(READ) flags 0x0",
        )
        .unwrap();
        assert_eq!(io.category, C::Disk);
        let oom = c(
            3,
            "kernel",
            "Out of memory: Killed process 4242 (firefox) total-vm:1kB",
        )
        .unwrap();
        assert_eq!(oom.category, C::OutOfMemory);
        let hot = c(
            4,
            "kernel",
            "CPU3: Core temperature above threshold, cpu clock throttled (total events = 1)",
        )
        .unwrap();
        assert_eq!((hot.category, hot.level), (C::Overheating, L::Warning));
        let core = c(
            2,
            "systemd-coredump",
            "Process 1234 (gnome-shell) of user 1000 dumped core.\n\nStack trace...",
        )
        .unwrap();
        assert_eq!(core.category, C::AppCrash);
        assert_eq!(
            core.detail,
            "Process 1234 (gnome-shell) of user 1000 dumped core."
        );
        let panic = c(0, "kernel", "Kernel panic - not syncing: Fatal exception").unwrap();
        assert_eq!((panic.category, panic.level), (C::KernelCrash, L::Critical));
        // Unknown errors are "Other"; unknown warnings and info are left out.
        let other = c(
            3,
            "kernel",
            "ACPI BIOS Error (bug): Could not resolve symbol",
        )
        .unwrap();
        assert_eq!(other.category, C::Other);
        assert!(c(4, "kernel", "usb 1-2: device descriptor read/64, error -71").is_none());
        assert!(c(6, "kernel", "Linux version 6.8.0").is_none());
        // A rule only matches its sender.
        assert!(c(6, "firefox", "process dumped core").is_none());
    }

    #[test]
    fn rules_are_consistent() {
        for r in RULES {
            assert!(!r.patterns.is_empty(), "{}", r.title);
            for p in r.patterns {
                assert_eq!(*p, p.to_lowercase(), "patterns are lower case: {p}");
            }
        }
    }

    #[test]
    fn filtering_and_copy() {
        let text = [
            line(T * 1_000_000, 3, "kernel", "I/O error, dev sdb, sector 8"),
            line(
                (T - 2 * 86_400) * 1_000_000,
                4,
                "kernel",
                "EDAC MC0: 1 CE memory read error",
            ),
            line(
                (T - 3 * 86_400) * 1_000_000,
                2,
                "systemd-coredump",
                "Process 7 (app) of user 1000 dumped core.",
            ),
        ]
        .join("\n");
        let mut list = parse_journal(&text);
        sort_and_cap(&mut list);
        assert_eq!(list.len(), 3);
        assert_eq!(list[0].category, C::Disk, "newest first");
        let now = T + 3600;
        let count = |f: &Filter| list.iter().filter(|e| f.matches(e, now)).count();
        assert_eq!(count(&Filter::default()), 3);
        let day = Filter {
            within_secs: 86_400,
            ..Default::default()
        };
        assert_eq!(count(&day), 1);
        let crashes = Filter {
            category: Some(C::AppCrash),
            ..Default::default()
        };
        assert_eq!(count(&crashes), 1);
        let text = Filter {
            text: "SDB".into(),
            ..Default::default()
        };
        assert_eq!(count(&text), 1);
        let t = to_text(&list[..1]);
        assert!(t.ends_with(
            "\tError\tDisk problem\tkernel\tDrive read/write error (I/O error, dev sdb, sector 8)\n"
        ));
        assert!(t.contains(" UTC\t"));
    }

    #[test]
    fn during_test() {
        assert_eq!(during_test_text(&[]), None);
        let e = classify(&JournalEntry {
            unix: T,
            priority: 4,
            identifier: "kernel".into(),
            message: "EDAC MC0: 1 CE memory read error".into(),
        })
        .unwrap();
        let t = during_test_text(&[e.clone(), e]).unwrap();
        assert!(t.starts_with(
            "2 hardware errors were logged during the test: Corrected memory error (2)."
        ));
    }

    #[test]
    fn long_details_are_cut() {
        assert_eq!(truncate("abc", 2), "ab…");
        assert_eq!(truncate("abc", 3), "abc");
    }
}
