//! The Health Check data model: findings, fix steps and links, check results and the
//! context every check reads.

use std::fmt::Write;

use crate::SystemInfo;
use crate::network::NetworkInfo;
use crate::power::{Action, PowerInfo};
use crate::sensors::SensorTable;
use crate::whea::WheaEvent;

/// How serious a finding is; ordered so the most severe sorts last.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    Info,
    Warning,
    Critical,
}

impl Level {
    /// Display name.
    pub fn name(self) -> &'static str {
        match self {
            Level::Info => "Info",
            Level::Warning => "Warning",
            Level::Critical => "Critical",
        }
    }
}

/// Which Health Check section a check or finding belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Category {
    Cpu,
    Memory,
    Gpu,
    Storage,
    /// Network adapters: addresses, Wi-Fi signal and link speed.
    Network,
    System,
    /// Settings that mainly matter for games; shown in their own section.
    Gaming,
    /// Power plan, sleep and device power-saving settings.
    Power,
    /// Blue screens, unexpected restarts and hardware (WHEA) errors.
    Crashes,
}

impl Category {
    /// Display name.
    pub fn name(self) -> &'static str {
        match self {
            Category::Cpu => "CPU",
            Category::Memory => "Memory",
            Category::Gpu => "GPU",
            Category::Storage => "Storage",
            Category::Network => "Network",
            Category::System => "System",
            Category::Gaming => "Gaming",
            Category::Power => "Power Management",
            Category::Crashes => "Crashes (BSOD)",
        }
    }

    /// Every category, in the order the Health Check tab lists them.
    pub const ALL: [Category; 9] = [
        Category::Crashes,
        Category::System,
        Category::Cpu,
        Category::Memory,
        Category::Gpu,
        Category::Storage,
        Category::Network,
        Category::Power,
        Category::Gaming,
    ];
}

/// Something a fix step can open. The app never fetches these itself: they are only handed to
/// the browser or Windows when the user clicks them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Link {
    /// A Google search for this query (built from the detected hardware).
    Search(String),
    /// A web page.
    Url(String),
    /// A Windows Settings page, e.g. `ms-settings:display`.
    Settings(String),
}

impl Link {
    /// The URL or URI to open.
    pub fn url(&self) -> String {
        match self {
            Link::Search(q) => format!("https://www.google.com/search?q={}", query_encode(q)),
            Link::Url(u) | Link::Settings(u) => u.clone(),
        }
    }

    /// Button text for the link.
    pub fn label(&self) -> &'static str {
        match self {
            Link::Search(_) => "Search Google",
            Link::Url(_) => "Open website",
            Link::Settings(_) => "Open Settings",
        }
    }
}

/// Encode a search query for a URL: spaces become '+', anything but unreserved ASCII is %-escaped.
pub fn query_encode(q: &str) -> String {
    let mut out = String::with_capacity(q.len());
    for (i, word) in q.split_whitespace().enumerate() {
        if i > 0 {
            out.push('+');
        }
        for b in word.bytes() {
            match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    out.push(char::from(b));
                }
                // Writing to a String can't fail.
                _ => _ = write!(out, "%{b:02X}"),
            }
        }
    }
    out
}

/// One numbered "how to fix" step, optionally with a link that helps carry it out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub text: String,
    pub link: Option<Link>,
}

impl Step {
    /// A plain-text step.
    pub fn new(text: impl Into<String>) -> Step {
        Step {
            text: text.into(),
            link: None,
        }
    }

    /// A step with a "Search Google" link for `query`.
    pub fn search(text: impl Into<String>, query: impl Into<String>) -> Step {
        Step {
            text: text.into(),
            link: Some(Link::Search(query.into())),
        }
    }

    /// A step with an "Open Settings" link (`ms-settings:` URI).
    pub fn settings(text: impl Into<String>, uri: &str) -> Step {
        Step {
            text: text.into(),
            link: Some(Link::Settings(uri.into())),
        }
    }

    /// A step with a link to a web page.
    pub fn url(text: impl Into<String>, url: &str) -> Step {
        Step {
            text: text.into(),
            link: Some(Link::Url(url.into())),
        }
    }
}

impl From<&str> for Step {
    fn from(s: &str) -> Step {
        Step::new(s)
    }
}

impl From<String> for Step {
    fn from(s: String) -> Step {
        Step::new(s)
    }
}

/// One problem a check found, with how to fix it.
#[derive(Debug, Clone, PartialEq)]
pub struct Finding {
    pub level: Level,
    pub category: Category,
    pub title: String,
    /// What was measured.
    pub detail: String,
    /// How to fix or investigate, in order.
    pub steps: Vec<Step>,
    /// The fix involves a security setting: the UI warns to change it only if the user knows
    /// what they are doing.
    pub security: bool,
    /// The fix is made in the UEFI/BIOS setup: the UI offers "Restart into UEFI" on UEFI PCs.
    pub bios: bool,
    /// A change the app can apply itself (offered as a one-click "Fix").
    pub fix: Option<Action>,
}

impl Finding {
    /// Mark this finding as being about a security setting.
    pub(super) fn security(mut self) -> Finding {
        self.security = true;
        self
    }

    /// Mark this finding as fixed in the UEFI/BIOS setup.
    pub(super) fn bios(mut self) -> Finding {
        self.bios = true;
        self
    }

    /// Offer a one-click fix.
    pub(super) fn with_fix(mut self, fix: Action) -> Finding {
        self.fix = Some(fix);
        self
    }
}

/// What a single check concluded.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// Problems found (an empty list counts as a pass).
    Findings(Vec<Finding>),
    /// Nothing wrong; the text says what was examined.
    Pass(String),
    /// Could not judge; the text says why.
    Skipped(String),
}

/// A check's name, section and outcome, as listed on the Health Check tab.
#[derive(Debug, Clone, PartialEq)]
pub struct CheckResult {
    pub name: &'static str,
    pub category: Category,
    pub outcome: Outcome,
}

/// Everything the checks may look at.
pub struct Context<'a> {
    pub info: &'a SystemInfo,
    pub sensors: &'a SensorTable,
    /// Today's date (year, month, day).
    pub today: (i64, u32, u32),
    pub uptime_secs: Option<u64>,
    /// Power settings, once gathered.
    pub power: Option<&'a PowerInfo>,
    /// Network adapters, once gathered.
    pub network: Option<&'a NetworkInfo>,
    /// WHEA hardware errors logged while the app runs (the event log query covers earlier ones).
    pub whea_live: &'a [WheaEvent],
}

/// How many recent samples count as "sustained" for live checks (~2 minutes at 1 s).
pub const RECENT: usize = 120;
/// Minimum samples before live checks give a verdict.
pub(super) const MIN_SAMPLES: usize = 10;

impl Context<'_> {
    /// Mean of the last `n` samples of a sensor, if it has at least `MIN_SAMPLES`.
    pub(super) fn recent_avg(&self, id: &str, n: usize) -> Option<f64> {
        let h = &self.sensors.get(id)?.history;
        if h.len() < MIN_SAMPLES {
            return None;
        }
        let (sum, count) = h
            .iter()
            .rev()
            .take(n)
            .fold((0.0, 0usize), |(s, c), &v| (s + f64::from(v), c + 1));
        Some(sum / count as f64)
    }

    /// Latest reading of a sensor.
    pub(super) fn current(&self, id: &str) -> Option<f64> {
        self.sensors.get(id).map(|t| t.reading.value)
    }
}
