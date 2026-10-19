//! Data for the "post-build" Health Check rules: firmware security state, monitor refresh
//! rates, Device Manager problems, crash and hardware-error events, network link speeds, NVMe
//! PCIe links, Windows activation, leftover GPU driver packages and startup apps.
//!
//! Gathered once at startup on the background thread (see [`crate::win::checkup`]); the checks
//! in [`crate::diagnostics`] only read it. Parsers here are platform-agnostic and unit-tested.

/// Everything the setup checks look at. Empty/`None` means "couldn't tell".
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Checkup {
    pub firmware: Firmware,
    /// Active monitors with their current and best refresh rate.
    pub monitors: Vec<Monitor>,
    /// Present devices that are display adapters, or that have a problem in Device Manager.
    pub devices: Vec<Device>,
    /// Hardware-error and crash events from the System event log.
    pub events: Option<Events>,
    /// Connected wired network adapters.
    pub nics: Vec<NicLink>,
    /// PCIe links of NVMe drives.
    pub nvme_links: Vec<PcieLink>,
    pub activation: Option<Activation>,
    /// Installed graphics driver packages (Apps & features names).
    pub gpu_driver_packages: Vec<String>,
    /// Enabled startup apps (Run keys and Startup folders), `None` if they couldn't be read.
    pub startup_apps: Option<Vec<String>>,
}

/// Boot mode and platform security state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Firmware {
    /// Booted through UEFI (false = legacy BIOS / CSM).
    pub uefi: Option<bool>,
    pub secure_boot: Option<bool>,
    pub tpm: Tpm,
}

/// TPM presence and version.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Tpm {
    #[default]
    Unknown,
    Missing,
    V12,
    V20,
}

/// How a monitor is connected.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Connection {
    Vga,
    Dvi,
    Hdmi,
    DisplayPort,
    /// Built-in laptop panel.
    Internal,
    /// Wireless, virtual or indirect displays.
    Virtual,
    #[default]
    Other,
}

impl Connection {
    /// From `DISPLAYCONFIG_VIDEO_OUTPUT_TECHNOLOGY`.
    pub fn from_output_technology(v: i32) -> Connection {
        match v {
            0 => Connection::Vga,
            4 => Connection::Dvi,
            5 => Connection::Hdmi,
            10 | 18 => Connection::DisplayPort,
            6 | 11 | 13 | i32::MIN => Connection::Internal,
            15..=17 => Connection::Virtual,
            _ => Connection::Other,
        }
    }

    /// Display name, e.g. "HDMI".
    pub fn name(self) -> &'static str {
        match self {
            Connection::Vga => "VGA",
            Connection::Dvi => "DVI",
            Connection::Hdmi => "HDMI",
            Connection::DisplayPort => "DisplayPort",
            Connection::Internal => "built-in",
            Connection::Virtual => "virtual",
            Connection::Other => "unknown",
        }
    }
}

/// An active monitor and its refresh rates.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Monitor {
    /// Friendly name from the monitor's EDID, e.g. "LG ULTRAGEAR".
    pub name: String,
    /// GDI source name, e.g. `\\.\DISPLAY1`.
    pub source: String,
    pub connection: Connection,
    pub width: u32,
    pub height: u32,
    /// Current refresh rate in Hz.
    pub current_hz: f64,
    /// Highest refresh rate Windows offers at the current resolution.
    pub max_hz: u32,
}

impl Monitor {
    /// "LG ULTRAGEAR (DisplayPort)" or the source name.
    pub fn label(&self) -> String {
        let name = if self.name.trim().is_empty() {
            self.source
                .rsplit(['\\', '.'])
                .next()
                .unwrap_or(&self.source)
        } else {
            self.name.trim()
        };
        match self.connection {
            Connection::Other => name.to_string(),
            c => format!("{name} ({})", c.name()),
        }
    }

    /// Runs noticeably below its best available refresh rate (e.g. 60 Hz on a 144 Hz panel).
    pub fn below_max(&self) -> bool {
        self.max_hz >= 75 && self.current_hz + 3.0 < self.max_hz as f64
    }
}

/// A Device Manager entry.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Device {
    pub name: String,
    /// Setup class name, e.g. "Display", "Net", "MEDIA", "Other".
    pub class: String,
    /// Driver service, e.g. "nvlddmkm", "BasicDisplay" (empty without a driver).
    pub service: String,
    /// First hardware id, e.g. `PCI\VEN_10DE&DEV_2204&SUBSYS_...`.
    pub hardware_id: String,
    /// Device Manager problem code (`CM_PROB_*`), if any.
    pub problem: Option<u32>,
}

impl Device {
    /// In the "Display" (display adapter) setup class.
    pub fn is_display(&self) -> bool {
        self.class.eq_ignore_ascii_case("Display")
    }

    /// Running on Windows' fallback driver instead of the vendor's.
    pub fn is_basic_display(&self) -> bool {
        self.service.eq_ignore_ascii_case("BasicDisplay")
            || self.name.contains("Microsoft Basic Display")
    }

    /// PCI vendor id from the hardware id (`VEN_xxxx`).
    pub fn pci_vendor(&self) -> Option<u32> {
        hex_after(&self.hardware_id, "VEN_")
    }
}

fn hex_after(s: &str, key: &str) -> Option<u32> {
    let upper = s.to_ascii_uppercase();
    let i = upper.find(key)? + key.len();
    u32::from_str_radix(upper.get(i..i + 4)?, 16).ok()
}

/// Problem codes that don't need fixing (disabled on purpose, or not plugged in).
pub fn problem_is_benign(code: u32) -> bool {
    matches!(code, 22 | 24 | 45 | 47)
}

/// Problem codes meaning the device has no (working) driver installed.
pub fn problem_is_missing_driver(code: u32) -> bool {
    matches!(code, 1 | 18 | 28)
}

/// Plain-language meaning of common Device Manager problem codes.
pub fn problem_text(code: u32) -> &'static str {
    match code {
        c if problem_is_missing_driver(c) => "no driver installed",
        3 => "driver may be corrupted or the system is low on memory",
        10 => "device cannot start",
        12 => "not enough free resources",
        14 => "needs a restart",
        19 | 32 | 39 => "driver configuration is damaged",
        21 => "Windows is removing this device",
        31 | 37 | 38 | 40 | 41 => "driver failed to load",
        43 => "stopped by Windows after it reported problems",
        48 => "driver blocked for known compatibility problems",
        52 => "driver signature could not be verified",
        _ => "has a problem",
    }
}

/// Event log hits from the last [`EVENT_DAYS`] days (memory diagnostic: [`MEMDIAG_DAYS`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Events {
    /// WHEA-Logger 17/18/19/47 (hardware errors).
    pub whea: Vec<Event>,
    /// Kernel-Power 41 (rebooted without cleanly shutting down); `code` = bugcheck or 0.
    pub unexpected_shutdowns: Vec<Event>,
    /// BugCheck 1001 (blue screens), with the stop code.
    pub bugchecks: Vec<Event>,
    /// Most recent Windows Memory Diagnostic result.
    pub memory_diagnostic: Option<Event>,
    /// GPU driver timeouts: Display 4101 (`code` `None`) and Windows Error Reporting
    /// LiveKernelEvent 141/117 (id 1001, `code` 0x141 / 0x117).
    pub tdr: Vec<Event>,
}

/// Days of event log read for crashes and hardware errors.
pub const EVENT_DAYS: u32 = 30;
/// Days of event log searched for the last Windows Memory Diagnostic result.
pub const MEMDIAG_DAYS: u32 = 90;
/// Most events read per query; counts at the cap mean "this many or more".
pub const EVENT_CAP: usize = 200;

/// Memory diagnostic event ids that mean errors were found (1101/1201 mean none were).
pub const MEMDIAG_FAILED: [u32; 2] = [1102, 1202];

/// One event log entry.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Event {
    pub id: u32,
    /// (year, month, day), UTC.
    pub date: (i64, u32, u32),
    /// Bugcheck / stop code, when the event carries one.
    pub code: Option<u32>,
}

/// Text between `<tag ...>` and `</tag>`, first occurrence.
fn element<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
    let start = xml.find(&format!("<{tag}"))?;
    let rest = &xml[start..];
    let open_end = rest.find('>')?;
    if rest[..open_end].ends_with('/') {
        return Some("");
    }
    let body = &rest[open_end + 1..];
    Some(&body[..body.find(&format!("</{tag}>"))?])
}

/// Value of `<Data Name='name'>...</Data>` in an event's XML, unescaped; empty for
/// `<Data Name='name'/>`.
pub fn event_data(xml: &str, name: &str) -> Option<String> {
    for q in ['\'', '"'] {
        let tag = format!("<Data Name={q}{name}{q}>");
        if let Some(i) = xml.find(&tag) {
            let rest = &xml[i + tag.len()..];
            let end = rest.find("</Data>")?;
            return Some(unescape_xml(&rest[..end]));
        }
        let empty = format!("<Data Name={q}{name}{q}/>");
        if xml.contains(&empty) {
            return Some(String::new());
        }
    }
    None
}

pub(crate) fn unescape_xml(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// "0x0000001a (0x..., ...)" or "209" -> stop code.
fn parse_code(s: &str) -> Option<u32> {
    let s = s.trim();
    let first = s.split([' ', '(']).next()?;
    match first
        .strip_prefix("0x")
        .or_else(|| first.strip_prefix("0X"))
    {
        Some(hex) => u64::from_str_radix(hex, 16).ok().map(|v| v as u32),
        None => first.parse::<u64>().ok().map(|v| v as u32),
    }
}

/// Parse an event rendered as XML (`EvtRender`): id, date and any bugcheck code.
pub fn parse_event_xml(xml: &str) -> Option<Event> {
    let id: u32 = element(xml, "EventID")?.trim().parse().ok()?;
    let t = xml.find("SystemTime=")? + "SystemTime=".len() + 1;
    let date = xml.get(t..t + 10)?;
    let mut it = date.split('-');
    let y: i64 = it.next()?.parse().ok()?;
    let m: u32 = it.next()?.parse().ok()?;
    let d: u32 = it.next()?.parse().ok()?;
    let code = event_data(xml, "BugcheckCode")
        .filter(|s| !s.is_empty())
        .or_else(|| event_data(xml, "param1"))
        .and_then(|s| parse_code(&s));
    Some(Event {
        id,
        date: (y, m, d),
        code,
    })
}

/// Normalize "_M" stop code variants (0x1000007E is 0x7E).
pub fn base_stop_code(code: u32) -> u32 {
    code & !0x1000_0000
}

/// Name of a common stop code.
pub fn stop_code_name(code: u32) -> Option<&'static str> {
    Some(match base_stop_code(code) {
        0x0A => "IRQL_NOT_LESS_OR_EQUAL",
        0x19 => "BAD_POOL_HEADER",
        0x1A => "MEMORY_MANAGEMENT",
        0x1E => "KMODE_EXCEPTION_NOT_HANDLED",
        0x3B => "SYSTEM_SERVICE_EXCEPTION",
        0x4E => "PFN_LIST_CORRUPT",
        0x50 => "PAGE_FAULT_IN_NONPAGED_AREA",
        0x7A => "KERNEL_DATA_INPAGE_ERROR",
        0x7E => "SYSTEM_THREAD_EXCEPTION_NOT_HANDLED",
        0x8E => "KERNEL_MODE_EXCEPTION_NOT_HANDLED",
        0x9F => "DRIVER_POWER_STATE_FAILURE",
        0xC2 => "BAD_POOL_CALLER",
        0xD1 => "DRIVER_IRQL_NOT_LESS_OR_EQUAL",
        0xEF => "CRITICAL_PROCESS_DIED",
        0xF4 => "CRITICAL_OBJECT_TERMINATION",
        0x101 => "CLOCK_WATCHDOG_TIMEOUT",
        0x109 => "CRITICAL_STRUCTURE_CORRUPTION",
        0x116 => "VIDEO_TDR_FAILURE",
        0x117 => "VIDEO_TDR_TIMEOUT_DETECTED",
        0x124 => "WHEA_UNCORRECTABLE_ERROR",
        0x133 => "DPC_WATCHDOG_VIOLATION",
        0x139 => "KERNEL_SECURITY_CHECK_FAILURE",
        0x141 => "VIDEO_ENGINE_TIMEOUT_DETECTED",
        0x154 => "UNEXPECTED_STORE_EXCEPTION",
        _ => return None,
    })
}

/// What usually causes a stop code, as a group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrashCause {
    /// Unstable memory or overclock (XMP/EXPO, PBO, undervolt).
    MemoryOrOverclock,
    /// CPU/platform hardware error (often an unstable overclock or undervolt).
    Hardware,
    /// Graphics driver or GPU.
    Graphics,
    /// Storage (drive, cable or controller).
    Storage,
    /// A driver (chipset, network, audio, ...).
    Driver,
    Unknown,
}

/// The usual cause group of a stop code.
pub fn crash_cause(code: u32) -> CrashCause {
    match base_stop_code(code) {
        0x1A | 0x50 | 0x4E | 0x19 | 0xC2 | 0x0A | 0x109 => CrashCause::MemoryOrOverclock,
        0x124 | 0x101 => CrashCause::Hardware,
        0x116 | 0x117 | 0x141 => CrashCause::Graphics,
        0x7A | 0xF4 | 0x154 => CrashCause::Storage,
        0xD1 | 0x9F | 0x133 | 0x7E | 0x1E | 0x3B | 0x8E | 0x139 | 0xEF => CrashCause::Driver,
        _ => CrashCause::Unknown,
    }
}

/// "MEMORY_MANAGEMENT (0x1A)" or "0x0000DEAD".
pub fn stop_code_label(code: u32) -> String {
    match stop_code_name(code) {
        Some(n) => format!("{n} (0x{:X})", base_stop_code(code)),
        None => format!("0x{code:08X}"),
    }
}

/// A connected wired network adapter.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NicLink {
    pub name: String,
    /// Negotiated link speed in bits per second.
    pub speed_bps: u64,
}

/// "100 Mbps", "1 Gbps", "2.5 Gbps".
pub fn format_link_speed(bps: u64) -> String {
    if bps >= 1_000_000_000 {
        let g = bps as f64 / 1e9;
        if g.fract() == 0.0 {
            format!("{g:.0} Gbps")
        } else {
            format!("{g:.1} Gbps")
        }
    } else {
        format!("{} Mbps", bps / 1_000_000)
    }
}

/// A PCIe link as (generation, lanes).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PcieLink {
    /// Drive model.
    pub device: String,
    pub current: (u32, u32),
    pub max: (u32, u32),
}

/// Windows license state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activation {
    Activated,
    NotActivated,
    /// Windows reports the license as tampered with.
    Tampered,
}

/// Map `SL_GENUINE_STATE` to activation.
pub fn activation_from_genuine_state(state: i32) -> Option<Activation> {
    match state {
        0 => Some(Activation::Activated),
        1 => Some(Activation::NotActivated),
        2 => Some(Activation::Tampered),
        _ => None,
    }
}

/// PCI vendor of a graphics driver package, from its Apps & features name.
pub fn gpu_package_vendor(display_name: &str) -> Option<u32> {
    let n = display_name.to_ascii_lowercase();
    if n.starts_with("nvidia graphics driver") {
        Some(crate::gpu::VENDOR_NVIDIA)
    } else if n == "amd software"
        || n.starts_with("amd radeon software")
        || n.starts_with("amd software:")
        || n.starts_with("amd catalyst")
    {
        Some(crate::gpu::VENDOR_AMD)
    } else if n.starts_with("intel(r) graphics driver")
        || n.starts_with("intel® graphics driver")
        || n.starts_with("intel graphics driver")
    {
        Some(crate::gpu::VENDOR_INTEL)
    } else {
        None
    }
}

/// `StartupApproved` values: the first byte is even when the entry is enabled, odd when the
/// user disabled it in Task Manager or Settings.
pub fn startup_approved_disabled(value: &[u8]) -> bool {
    value.first().is_some_and(|b| b & 1 == 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_from_xml() {
        let kp = "<Event xmlns='http://schemas.microsoft.com/win/2004/08/events/event'><System>\
            <Provider Name='Microsoft-Windows-Kernel-Power'/><EventID>41</EventID>\
            <TimeCreated SystemTime='2026-10-01T12:34:56.1234567Z'/></System><EventData>\
            <Data Name='BugcheckCode'>209</Data><Data Name='BugcheckParameter1'>0x0</Data></EventData></Event>";
        assert_eq!(
            parse_event_xml(kp),
            Some(Event {
                id: 41,
                date: (2026, 10, 1),
                code: Some(0xD1)
            })
        );
        let bc = "<Event><System><EventID Qualifiers='16384'>1001</EventID>\
            <TimeCreated SystemTime=\"2026-09-30T01:02:03Z\"/></System><EventData>\
            <Data Name=\"param1\">0x0000001a (0x0000000000041792, 0xffff)</Data></EventData></Event>";
        let e = parse_event_xml(bc).unwrap();
        assert_eq!((e.id, e.date, e.code), (1001, (2026, 9, 30), Some(0x1A)));
        let whea = "<Event><System><EventID>19</EventID><TimeCreated SystemTime='2026-01-02T00:00:00Z'/></System></Event>";
        assert_eq!(parse_event_xml(whea).unwrap().code, None);
        assert_eq!(parse_event_xml("garbage"), None);
        // An empty BugcheckCode falls back to param1.
        let empty = "<Event><System><EventID>41</EventID><TimeCreated SystemTime='2026-01-02T00:00:00Z'/></System>\
            <EventData><Data Name='BugcheckCode'/><Data Name='param1'>0x7E</Data></EventData></Event>";
        assert_eq!(parse_event_xml(empty).unwrap().code, Some(0x7E));
        assert_eq!(
            event_data("<Data Name=\"x\">a &amp; b</Data>", "x").as_deref(),
            Some("a & b")
        );
        assert_eq!(event_data("<Data Name='x'/>", "x").as_deref(), Some(""));
        assert_eq!(event_data("<Data Name='y'>1</Data>", "x"), None);
    }

    #[test]
    fn stop_codes() {
        assert_eq!(stop_code_label(0x1A), "MEMORY_MANAGEMENT (0x1A)");
        assert_eq!(
            stop_code_label(0x1000_007E),
            "SYSTEM_THREAD_EXCEPTION_NOT_HANDLED (0x7E)"
        );
        assert_eq!(stop_code_label(0xDEAD), "0x0000DEAD");
        assert_eq!(crash_cause(0x124), CrashCause::Hardware);
        assert_eq!(crash_cause(0x116), CrashCause::Graphics);
        assert_eq!(crash_cause(0x1A), CrashCause::MemoryOrOverclock);
    }

    #[test]
    fn devices() {
        let d = Device {
            name: "Microsoft Basic Display Adapter".into(),
            class: "Display".into(),
            service: "BasicDisplay".into(),
            hardware_id: r"PCI\VEN_10DE&DEV_2204&SUBSYS_161319DA".into(),
            problem: None,
        };
        assert!(d.is_display() && d.is_basic_display());
        assert_eq!(d.pci_vendor(), Some(0x10DE));
        assert!(problem_is_benign(22) && !problem_is_benign(28));
        assert_eq!(problem_text(28), "no driver installed");
    }

    #[test]
    fn monitors() {
        let mut m = Monitor {
            name: "LG ULTRAGEAR".into(),
            source: r"\\.\DISPLAY1".into(),
            connection: Connection::from_output_technology(10),
            width: 2560,
            height: 1440,
            current_hz: 59.95,
            max_hz: 144,
        };
        assert!(m.below_max());
        assert_eq!(m.label(), "LG ULTRAGEAR (DisplayPort)");
        m.current_hz = 143.9;
        assert!(!m.below_max());
        m.max_hz = 60;
        m.current_hz = 59.0;
        assert!(!m.below_max());
        m.name.clear();
        m.connection = Connection::Other;
        assert_eq!(m.label(), "DISPLAY1");
        assert_eq!(
            Connection::from_output_technology(i32::MIN),
            Connection::Internal
        );
    }

    #[test]
    fn misc() {
        assert_eq!(format_link_speed(100_000_000), "100 Mbps");
        assert_eq!(format_link_speed(1_000_000_000), "1 Gbps");
        assert_eq!(format_link_speed(2_500_000_000), "2.5 Gbps");
        assert_eq!(
            activation_from_genuine_state(0),
            Some(Activation::Activated)
        );
        assert_eq!(
            activation_from_genuine_state(1),
            Some(Activation::NotActivated)
        );
        assert_eq!(
            gpu_package_vendor("NVIDIA Graphics Driver 581.57"),
            Some(0x10DE)
        );
        assert_eq!(gpu_package_vendor("AMD Software"), Some(0x1002));
        assert_eq!(gpu_package_vendor("AMD Chipset Software"), None);
        assert_eq!(gpu_package_vendor("NVIDIA PhysX System Software"), None);
        assert!(startup_approved_disabled(&[3, 0, 0]));
        assert!(!startup_approved_disabled(&[2, 0, 0]));
        assert!(!startup_approved_disabled(&[]));
    }
}
