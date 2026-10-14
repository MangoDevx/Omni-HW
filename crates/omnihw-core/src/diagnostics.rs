//! Health Check: rules over the collected static info and live sensors that flag common
//! problems and suggest fixes. Every check reports a finding, a pass, or why it was skipped,
//! so the user can see what was examined.

mod setup;

use crate::bios_help::{self, BiosSetting, Board};
use crate::cpuid::Vendor;
use crate::gpu::ReBar;
use crate::sensors::SensorTable;
use crate::storage::BusType;
use crate::win::PlanKind;
use crate::{SystemInfo, format_bytes};

mod power;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    Info,
    Warning,
    Critical,
}

impl Level {
    pub fn name(self) -> &'static str {
        match self {
            Level::Info => "Info",
            Level::Warning => "Warning",
            Level::Critical => "Critical",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Category {
    Cpu,
    Memory,
    Gpu,
    Storage,
    System,
    /// Settings that mainly matter for games; shown in their own section.
    Gaming,
    /// Power plan, sleep and device power-saving settings.
    Power,
}

impl Category {
    pub fn name(self) -> &'static str {
        match self {
            Category::Cpu => "CPU",
            Category::Memory => "Memory",
            Category::Gpu => "GPU",
            Category::Storage => "Storage",
            Category::System => "System",
            Category::Gaming => "Gaming",
            Category::Power => "Power",
        }
    }
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
    let mut out = String::new();
    for b in q.split_whitespace().collect::<Vec<_>>().join(" ").bytes() {
        match b {
            b' ' => out.push('+'),
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
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
    pub fix: Option<crate::power::Action>,
}

impl Finding {
    /// Mark this finding as being about a security setting.
    fn security(mut self) -> Finding {
        self.security = true;
        self
    }

    /// Mark this finding as fixed in the UEFI/BIOS setup.
    fn bios(mut self) -> Finding {
        self.bios = true;
        self
    }

    /// Offer a one-click fix.
    fn with_fix(mut self, fix: crate::power::Action) -> Finding {
        self.fix = Some(fix);
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Findings(Vec<Finding>),
    Pass(String),
    Skipped(String),
}

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
    pub power: Option<&'a crate::power::PowerInfo>,
}

/// How many recent samples count as "sustained" for live checks (~2 minutes at 1 s).
pub const RECENT: usize = 120;
/// Minimum samples before live checks give a verdict.
const MIN_SAMPLES: usize = 10;

impl Context<'_> {
    /// Mean of the last `n` samples of a sensor, if it has at least `MIN_SAMPLES`.
    fn recent_avg(&self, id: &str, n: usize) -> Option<f64> {
        let h = &self.sensors.get(id)?.history;
        if h.len() < MIN_SAMPLES {
            return None;
        }
        let tail: Vec<f64> = h.iter().rev().take(n).map(|&v| v as f64).collect();
        Some(tail.iter().sum::<f64>() / tail.len() as f64)
    }

    fn current(&self, id: &str) -> Option<f64> {
        self.sensors.get(id).map(|t| t.reading.value)
    }
}

type Check = fn(&Context) -> Outcome;

const CHECKS: &[(&str, Category, Check)] = &[
    (
        "CPU microcode (13th/14th Gen Intel stability)",
        Category::Cpu,
        intel_raptor_microcode,
    ),
    ("CPU clocks under load", Category::Cpu, cpu_throttling),
    ("Memory speed (XMP / EXPO)", Category::Memory, memory_speed),
    ("Memory channel layout", Category::Memory, memory_channels),
    ("Memory pressure", Category::Memory, memory_pressure),
    ("Installed memory", Category::Memory, memory_capacity),
    (
        "Memory diagnostic results",
        Category::Memory,
        setup::memory_diagnostic,
    ),
    (
        "Monitor connected to the graphics card",
        Category::Gpu,
        monitor_connection,
    ),
    ("GPU PCIe link", Category::Gpu, gpu_pcie_link),
    ("GPU temperature", Category::Gpu, gpu_temperature),
    ("GPU memory (VRAM) pressure", Category::Gpu, gpu_vram),
    (
        "Monitor refresh rate",
        Category::Gpu,
        setup::monitor_refresh,
    ),
    (
        "Graphics driver installed",
        Category::Gpu,
        setup::gpu_driver_missing,
    ),
    (
        "GPU power and thermal slowdown",
        Category::Gpu,
        setup::gpu_hw_slowdown,
    ),
    (
        "Leftover graphics drivers",
        Category::Gpu,
        setup::leftover_gpu_drivers,
    ),
    // ReBAR and Windows settings.
    ("Resizable BAR", Category::Gaming, resizable_bar),
    ("GPU driver age", Category::Gpu, gpu_driver_age),
    ("Drive free space", Category::Storage, drive_free_space),
    ("Drive health (NVMe SMART)", Category::Storage, drive_health),
    ("Drive health (SATA SMART)", Category::Storage, sata_smart),
    ("Drive temperature", Category::Storage, drive_temperature),
    ("Drives set up", Category::Storage, setup::drive_setup),
    (
        "Windows drive type",
        Category::Storage,
        setup::system_drive_type,
    ),
    ("Drive link speed", Category::Storage, setup::storage_link),
    ("Battery wear", Category::System, battery_wear),
    ("Power plan", Category::Power, power_plan),
    ("BIOS age", Category::System, bios_age),
    ("Uptime", Category::System, uptime),
    ("Pending restart", Category::System, pending_restart),
    ("Memory Integrity (VBS)", Category::Gaming, memory_integrity),
    (
        "Hardware-accelerated GPU scheduling",
        Category::Gaming,
        gpu_scheduling,
    ),
    ("Game Mode", Category::Gaming, game_mode),
    ("Pagefile", Category::System, pagefile),
    ("Device drivers", Category::System, setup::device_problems),
    (
        "Hardware errors (WHEA)",
        Category::System,
        setup::whea_errors,
    ),
    (
        "Blue screens and unexpected restarts",
        Category::System,
        setup::crashes,
    ),
    (
        "Ethernet link speed",
        Category::System,
        setup::ethernet_speed,
    ),
    (
        "Secure Boot, TPM and UEFI boot",
        Category::System,
        setup::firmware_security,
    ),
    ("Windows activation", Category::System, setup::activation),
    ("Startup apps", Category::System, setup::startup_apps),
];

/// Run every check. Findings within a result are ordered most severe first.
pub fn run(ctx: &Context) -> Vec<CheckResult> {
    CHECKS
        .iter()
        .chain(power::CHECKS)
        .map(|&(name, category, check)| {
            let outcome = match check(ctx) {
                Outcome::Findings(mut f) if !f.is_empty() => {
                    // The check's category is authoritative (e.g. Gaming groups GPU/System settings).
                    for x in &mut f {
                        x.category = category;
                    }
                    f.sort_by_key(|f| std::cmp::Reverse(f.level));
                    Outcome::Findings(f)
                }
                Outcome::Findings(_) => Outcome::Pass("No problems found".into()),
                o => o,
            };
            CheckResult {
                name,
                category,
                outcome,
            }
        })
        .collect()
}

/// All findings across results, most severe first.
pub fn findings(results: &[CheckResult]) -> Vec<&Finding> {
    let mut all: Vec<&Finding> = results
        .iter()
        .filter_map(|r| match &r.outcome {
            Outcome::Findings(f) => Some(f.iter()),
            _ => None,
        })
        .flatten()
        .collect();
    all.sort_by_key(|f| std::cmp::Reverse(f.level));
    all
}

/// A finding whose steps are plain text.
fn finding(
    level: Level,
    category: Category,
    title: impl Into<String>,
    detail: impl Into<String>,
    tips: &[&str],
) -> Finding {
    guided(
        level,
        category,
        title,
        detail,
        tips.iter().map(|&t| Step::new(t)).collect(),
    )
}

/// A finding with guided steps (links to searches, Settings pages or websites).
fn guided(
    level: Level,
    category: Category,
    title: impl Into<String>,
    detail: impl Into<String>,
    steps: Vec<Step>,
) -> Finding {
    Finding {
        level,
        category,
        title: title.into(),
        detail: detail.into(),
        steps,
        security: false,
        bios: false,
        fix: None,
    }
}

/// Step with a Google search for this board's BIOS downloads.
fn bios_update_step(ctx: &Context, text: &str) -> Step {
    let name = Board::detect(ctx.info).search_name();
    let query = if name.is_empty() {
        "motherboard BIOS update download".to_string()
    } else {
        format!("{name} BIOS download")
    };
    Step::search(text, query)
}

// ---- CPU ------------------------------------------------------------------------------------

/// Intel's fix for the 13th/14th Gen desktop "Vmin shift" instability ships in microcode 0x12B+.
const RAPTOR_FIXED_MICROCODE: u32 = 0x12B;

fn intel_raptor_microcode(ctx: &Context) -> Outcome {
    let c = &ctx.info.cpu;
    // Model B7h is the large Raptor Lake-S die used by affected 65 W+ desktop parts.
    let affected_die = c.vendor == Vendor::Intel && c.family == 6 && c.model == 0xB7;
    let low_power = c
        .brand
        .split_whitespace()
        .any(|w| w.starts_with("i") && w.ends_with('T'));
    if !affected_die || low_power {
        return Outcome::Skipped(
            "Only applies to 13th/14th Gen Intel Core desktop (Raptor Lake-S) CPUs".into(),
        );
    }
    let Some(rev) = ctx.info.microcode else {
        return Outcome::Skipped("Microcode revision not available".into());
    };
    if rev >= RAPTOR_FIXED_MICROCODE {
        return Outcome::Pass(format!(
            "Microcode 0x{rev:X} includes Intel's Vmin shift fix"
        ));
    }
    Outcome::Findings(vec![
        guided(
            Level::Critical,
            Category::Cpu,
            "CPU microcode predates Intel's instability fix",
            format!(
                "{} is running microcode 0x{rev:X}. 13th/14th Gen desktop CPUs can degrade over time \
                 (Vmin shift) without microcode 0x{RAPTOR_FIXED_MICROCODE:X} or newer.",
                c.brand
            ),
            vec![
                bios_update_step(
                    ctx,
                    "Update the motherboard BIOS to a version with microcode 0x12B or newer (check the board vendor's support page).",
                ),
                Step::new(
                    "After updating, load the BIOS's \"Intel Default Settings\" profile instead of unlimited power limits.",
                ),
                Step::new(
                    "If you see crashes or shader-compilation errors in games, contact Intel about the extended warranty.",
                ),
            ],
        )
        .bios(),
    ])
}

/// Under sustained load, the average clock should stay at or above base clock.
fn cpu_throttling(ctx: &Context) -> Outcome {
    let (Some(base), Some(load), Some(clock)) = (
        ctx.info.base_mhz,
        ctx.sensors.get("cpu/load/total"),
        ctx.sensors.get("cpu/clock/avg"),
    ) else {
        return Outcome::Skipped("Needs base clock and live CPU sensors".into());
    };
    // Both histories are appended on every sample, so align them from the newest end.
    let busy: Vec<f64> = load
        .history
        .iter()
        .rev()
        .zip(clock.history.iter().rev())
        .filter(|(l, _)| **l >= 70.0)
        .map(|(_, c)| *c as f64)
        .collect();
    if busy.len() < MIN_SAMPLES {
        return Outcome::Skipped(
            "Needs some time with the CPU under heavy load (e.g. a game or render) to judge".into(),
        );
    }
    let mean = busy.iter().sum::<f64>() / busy.len() as f64;
    if mean >= base as f64 * 0.9 {
        return Outcome::Pass(format!(
            "Averaged {mean:.0} MHz under load (base {base} MHz)"
        ));
    }
    Outcome::Findings(vec![finding(
        Level::Warning,
        Category::Cpu,
        "CPU slows below base clock under load",
        format!(
            "Under heavy load the CPU averaged {mean:.0} MHz, below its {base} MHz base clock. This usually means thermal or power throttling."
        ),
        &[
            "Check that the CPU cooler is seated properly and its fans/pump are running.",
            "Clean dust from heatsinks and filters; improve case airflow.",
            "On laptops, use the plugged-in performance mode and keep vents clear.",
            "Check the BIOS for overly low power limits (PL1/PL2 or PPT).",
        ],
    )])
}

// ---- Memory ---------------------------------------------------------------------------------

/// Speed rating embedded in common part numbers (e.g. "CMW32GX4M2C3200C16" -> 3200,
/// "F5-6000J3038F16G" -> 6000): the first 4-digit number in a plausible DDR range.
pub fn rated_speed_from_part(part: &str) -> Option<u32> {
    let mut digits = String::new();
    let mut runs = Vec::new();
    for ch in part.chars().chain(std::iter::once(' ')) {
        if ch.is_ascii_digit() {
            digits.push(ch);
        } else if !digits.is_empty() {
            runs.push(std::mem::take(&mut digits));
        }
    }
    runs.iter()
        .filter(|r| r.len() == 4)
        .filter_map(|r| r.parse::<u32>().ok())
        .find(|v| (1600..=9600).contains(v))
}

fn memory_speed(ctx: &Context) -> Outcome {
    let Some(sm) = &ctx.info.smbios else {
        return Outcome::Skipped("SMBIOS not available".into());
    };
    let dimms: Vec<_> = sm.populated_memory().collect();
    let Some(d) = dimms.first() else {
        return Outcome::Skipped("No memory modules reported".into());
    };
    let configured = dimms
        .iter()
        .map(|d| d.configured_speed_mts)
        .filter(|&s| s > 0)
        .min()
        .unwrap_or(0);
    if configured == 0 {
        return Outcome::Skipped("Configured memory speed not reported".into());
    }
    let rated = dimms
        .iter()
        .filter_map(|d| rated_speed_from_part(&d.part_number))
        .min()
        .into_iter()
        .chain(dimms.iter().map(|d| d.speed_mts))
        .max()
        .unwrap_or(0);
    // Allow small differences (e.g. 3200 vs 3196 reported).
    if rated <= configured + 50 {
        return Outcome::Pass(format!("Running at its rated {configured} MT/s"));
    }
    let (profile, short) = match ctx.info.cpu.vendor {
        Vendor::Amd => ("EXPO (or DOCP)", "EXPO"),
        _ => ("XMP", "XMP"),
    };
    let mut steps = bios_help::bios_steps(
        ctx.info,
        BiosSetting::MemoryProfile,
        &format!("Enable the {profile} memory profile."),
        short,
    );
    steps.push(Step::new(
        "If the system becomes unstable afterwards, update the BIOS or try the next lower profile.",
    ));
    Outcome::Findings(vec![
        guided(
            Level::Warning,
            Category::Memory,
            format!("Memory runs at {configured} MT/s but is rated for {rated} MT/s"),
            format!(
                "{} ({}) appears to be rated for {rated} MT/s, but it's configured at {configured} MT/s. \
                 The {profile} memory profile is probably not enabled, which costs performance.",
                d.part_number, d.memory_type
            ),
            steps,
        )
        .bios(),
    ])
}

/// Memory channel letter from a slot name such as "DIMM_A1", "ChannelB-DIMM0" or "Controller1-DIMMB2".
pub fn channel_letter(locator: &str) -> Option<char> {
    let upper = locator.to_ascii_uppercase();
    let after = |key: &str| -> Option<char> {
        let i = upper.find(key)? + key.len();
        let c = upper[i..]
            .trim_start_matches([' ', '_', '-'])
            .chars()
            .next()?;
        c.is_ascii_uppercase().then_some(c)
    };
    after("CHANNEL").or_else(|| {
        let c = after("DIMM")?;
        // "DIMM0" has no letter; "DIMMA2" / "DIMM_A1" do.
        let i = upper.find("DIMM")? + 4;
        let rest = upper[i..].trim_start_matches([' ', '_', '-']);
        rest.chars()
            .nth(1)
            .filter(|n| n.is_ascii_digit())
            .map(|_| c)
    })
}

fn memory_channels(ctx: &Context) -> Outcome {
    let Some(sm) = &ctx.info.smbios else {
        return Outcome::Skipped("SMBIOS not available".into());
    };
    let dimms: Vec<_> = sm.populated_memory().collect();
    if dimms.is_empty() {
        return Outcome::Skipped("No memory modules reported".into());
    }
    // Soldered memory (laptops) reports a single "module"; only judge socketed DIMMs.
    let socketed = dimms
        .iter()
        .all(|d| d.form_factor == "DIMM" || d.form_factor == "SO-DIMM");
    let mut out = Vec::new();
    if dimms.len() == 1 && socketed && sm.memory_slots >= 2 {
        out.push(finding(
            Level::Warning,
            Category::Memory,
            "Single memory module (single-channel)",
            format!("Only one module is installed ({}), so memory runs in single-channel mode with about half the bandwidth.", dimms[0].locator),
            &[
                "Add a second identical module to run dual-channel.",
                "Use the slots the motherboard manual recommends for two modules (usually A2 and B2).",
            ],
        ));
    }
    let letters: Option<Vec<char>> = dimms
        .iter()
        .map(|d| channel_letter(&d.locator).or_else(|| channel_letter(&d.bank_locator)))
        .collect();
    if let Some(letters) = &letters
        && dimms.len() >= 2
        && letters.iter().all(|&c| c == letters[0])
    {
        out.push(finding(
            Level::Warning,
            Category::Memory,
            "All memory modules are in the same channel",
            format!(
                "Modules are installed in {} - all on channel {}, so they share one channel's bandwidth.",
                dimms.iter().map(|d| d.locator.as_str()).collect::<Vec<_>>().join(", "),
                letters[0]
            ),
            &["Move the modules so each channel has one (check the motherboard manual; usually A2 and B2)."],
        ));
    }
    let first = dimms[0];
    if dimms
        .iter()
        .any(|d| d.size_mb != first.size_mb || d.part_number != first.part_number)
    {
        out.push(finding(
            Level::Info,
            Category::Memory,
            "Mixed memory modules",
            "Installed modules differ in size or part number. Mixed kits often have to run at a lower common speed.",
            &["For best stability and speed, use a matched kit."],
        ));
    }
    if out.is_empty() {
        let layout = match &letters {
            Some(l) => format!(
                "{} modules across channels {}",
                dimms.len(),
                l.iter()
                    .map(|c| c.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            None => format!("{} matching modules", dimms.len()),
        };
        return Outcome::Pass(layout);
    }
    Outcome::Findings(out)
}

fn memory_pressure(ctx: &Context) -> Outcome {
    let Some(load) = ctx.recent_avg("mem/load", RECENT) else {
        return Outcome::Skipped("Collecting samples".into());
    };
    let level = match load {
        l if l >= 97.0 => Level::Critical,
        l if l >= 90.0 => Level::Warning,
        _ => return Outcome::Pass(format!("{load:.0} % used on average recently")),
    };
    Outcome::Findings(vec![finding(
        level,
        Category::Memory,
        "Memory is nearly full",
        format!(
            "Physical memory has averaged {load:.0} % used over the last couple of minutes. Windows is likely paging to disk, which causes stutters."
        ),
        &[
            "Close unused applications and browser tabs.",
            "Check Task Manager > Processes (sort by Memory) for anything using an unusual amount.",
            "If this is normal for your workload, consider adding more RAM.",
        ],
    )])
}

fn memory_capacity(ctx: &Context) -> Outcome {
    let Some(m) = ctx.info.memory else {
        return Outcome::Skipped("Memory size not available".into());
    };
    let gb = m.total_bytes as f64 / (1u64 << 30) as f64;
    if gb >= 15.0 {
        return Outcome::Pass(format!("{gb:.0} GB usable"));
    }
    Outcome::Findings(vec![finding(
        Level::Info,
        Category::Memory,
        format!("Only {gb:.1} GB of usable memory"),
        "Modern games and browsers commonly use more than 8 GB; 16 GB or more is recommended.",
        &["Consider upgrading to at least 16 GB (2 x 8 GB for dual-channel)."],
    )])
}

// ---- GPU ------------------------------------------------------------------------------------

fn display_list(g: &crate::gpu::GpuAdapter) -> String {
    g.displays
        .iter()
        .map(|d| d.summary())
        .collect::<Vec<_>>()
        .join(", ")
}

/// On a desktop with a graphics card, monitors plugged into the motherboard's video outputs
/// run on the CPU's integrated graphics: games render slowly or on the wrong GPU.
fn monitor_connection(ctx: &Context) -> Outcome {
    let gpus = &ctx.info.gpus;
    let discrete: Vec<_> = gpus.iter().filter(|g| !g.is_integrated()).collect();
    let integrated: Vec<_> = gpus.iter().filter(|g| g.is_integrated()).collect();
    if discrete.is_empty() {
        return Outcome::Skipped("No separate graphics card".into());
    }
    if ctx.info.battery.is_some() {
        return Outcome::Skipped(
            "Laptops route displays through the integrated GPU by design".into(),
        );
    }
    let on_igpu: usize = integrated.iter().map(|g| g.displays.len()).sum();
    let on_card: usize = discrete.iter().map(|g| g.displays.len()).sum();
    let card = discrete[0];
    if on_igpu == 0 {
        if on_card == 0 {
            return Outcome::Skipped("No connected displays found".into());
        }
        return Outcome::Pass(format!(
            "{on_card} display{} connected to {}",
            if on_card == 1 { "" } else { "s" },
            discrete
                .iter()
                .filter(|g| !g.displays.is_empty())
                .map(|g| g.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let igpu = integrated
        .iter()
        .find(|g| !g.displays.is_empty())
        .copied()
        .unwrap_or(integrated[0]);
    let tips: &[&str] = &[
        "Plug the monitor cable into the graphics card's ports (the horizontal row of ports lower on the back of the case, below the motherboard's I/O panel).",
        "Use the motherboard's HDMI/DisplayPort only if you have no graphics card.",
        "Optionally disable integrated graphics in the BIOS (often \"iGPU Multi-Monitor\" / \"Primary Display\") so the motherboard ports can't be used by mistake.",
    ];
    if on_card == 0 {
        return Outcome::Findings(vec![finding(
            Level::Critical,
            Category::Gpu,
            "Your monitor is plugged into the motherboard, not the graphics card",
            format!(
                "All displays ({}) are connected to {} (integrated graphics). The {} is not driving any display, so games and video run on the much slower integrated GPU or pay a copy penalty.",
                display_list(igpu),
                igpu.name,
                card.name
            ),
            tips,
        )]);
    }
    Outcome::Findings(vec![finding(
        Level::Info,
        Category::Gpu,
        "A display is connected to the motherboard's video output",
        format!(
            "{} is on {} (integrated graphics) while other displays use {}. Fine for a secondary screen, but games shown on it render through the integrated GPU.",
            display_list(igpu),
            igpu.name,
            card.name
        ),
        &["Connect any gaming monitor to the graphics card's ports."],
    )])
}

fn gpu_pcie_link(ctx: &Context) -> Outcome {
    let mut out = Vec::new();
    let mut checked = Vec::new();
    for g in &ctx.info.gpus {
        let Some(n) = &g.nvml else { continue };
        let (Some((cur_gen, cur_w)), Some((max_gen, max_w))) = (n.pcie_current, n.pcie_max) else {
            continue;
        };
        checked.push(format!("{}: x{cur_w} {cur_gen}.0", g.name));
        if cur_w < max_w {
            out.push(finding(
                Level::Warning,
                Category::Gpu,
                format!("{} runs at PCIe x{cur_w} instead of x{max_w}", g.name),
                format!("The card supports x{max_w} but the link trained at x{cur_w}, reducing bandwidth."),
                &[
                    "Make sure the card is in the top full-length (CPU) slot and fully seated.",
                    "Check whether an M.2 drive or other card is sharing the slot's lanes (see the motherboard manual).",
                    "Reseat the card and clean the slot contacts; check riser cables if one is used.",
                ],
            ));
        } else if cur_gen < max_gen {
            // Lower generation at idle is normal power saving; only flag it while the GPU is busy.
            let busy = ctx
                .current(&format!("{}/nv/load", g.sensor_prefix()))
                .is_some_and(|l| l >= 50.0);
            if busy {
                let tip = format!(
                    "Check the BIOS PCIe slot setting (set it to Auto or Gen {max_gen}) and any riser cable's rating."
                );
                out.push(
                    finding(
                        Level::Info,
                        Category::Gpu,
                        format!("{} is at PCIe {cur_gen}.0 under load", g.name),
                        format!("The card supports PCIe {max_gen}.0 but is running at {cur_gen}.0 while busy."),
                        &[&tip],
                    )
                    .bios(),
                );
            }
        }
    }
    if checked.is_empty() {
        return Outcome::Skipped("Needs PCIe link data (NVIDIA GPUs via NVML)".into());
    }
    if out.is_empty() {
        return Outcome::Pass(checked.join("; "));
    }
    Outcome::Findings(out)
}

fn gpu_temperature(ctx: &Context) -> Outcome {
    let mut out = Vec::new();
    let mut seen = Vec::new();
    for g in &ctx.info.gpus {
        let Some(t) = ctx.recent_avg(&format!("{}/temp", g.sensor_prefix()), RECENT) else {
            continue;
        };
        seen.push(format!("{}: {t:.0} °C", g.name));
        let level = match t {
            t if t >= 90.0 => Level::Critical,
            t if t >= 83.0 => Level::Warning,
            _ => continue,
        };
        out.push(finding(
            level,
            Category::Gpu,
            format!("{} is running hot", g.name),
            format!("GPU temperature has averaged {t:.0} °C over the last couple of minutes. Most GPUs start reducing clocks in the low-to-mid 80s."),
            &[
                "Clean dust from the card's heatsink and fans and from case filters.",
                "Improve case airflow (intake at the front, exhaust at the back/top).",
                "Set a more aggressive fan curve, or cap the frame rate in games.",
                "On older cards, replacing thermal paste and pads can lower temperatures noticeably.",
            ],
        ));
    }
    if seen.is_empty() {
        return Outcome::Skipped("No GPU temperature sensor".into());
    }
    if out.is_empty() {
        return Outcome::Pass(seen.join("; "));
    }
    Outcome::Findings(out)
}

fn gpu_vram(ctx: &Context) -> Outcome {
    let mut out = Vec::new();
    let mut seen = Vec::new();
    for g in &ctx.info.gpus {
        let pre = g.sensor_prefix();
        let used = ctx
            .recent_avg(&format!("{pre}/vram/used"), RECENT)
            .or_else(|| ctx.recent_avg(&format!("{pre}/vram/dedicated"), RECENT));
        let (Some(used_mb), true) = (used, g.dedicated_vram > 0) else {
            continue;
        };
        let frac = used_mb * 1_048_576.0 / g.dedicated_vram as f64;
        seen.push(format!("{}: {:.0} %", g.name, frac * 100.0));
        if frac >= 0.95 {
            out.push(finding(
                Level::Warning,
                Category::Gpu,
                format!("{} video memory is full", g.name),
                format!(
                    "{:.0} % of {} VRAM has been in use. When VRAM runs out, games stutter as data spills to system RAM.",
                    frac * 100.0,
                    format_bytes(g.dedicated_vram)
                ),
                &[
                    "Lower texture quality or resolution in games.",
                    "Close other GPU-heavy apps (browsers with video, streaming tools).",
                ],
            ));
        }
    }
    if seen.is_empty() {
        return Outcome::Skipped("Collecting samples".into());
    }
    if out.is_empty() {
        return Outcome::Pass(seen.join("; "));
    }
    Outcome::Findings(out)
}

// ---- Storage --------------------------------------------------------------------------------

fn drive_free_space(ctx: &Context) -> Outcome {
    let mut out = Vec::new();
    let mut n = 0;
    for v in ctx.info.drives.iter().flat_map(|d| &d.volumes) {
        if v.total_bytes == 0 {
            continue;
        }
        n += 1;
        let free = v.free_bytes as f64 / v.total_bytes as f64;
        let level = match free {
            f if f < 0.05 => Level::Critical,
            f if f < 0.10 => Level::Warning,
            _ => continue,
        };
        let system = v.mount.eq_ignore_ascii_case("C:");
        out.push(guided(
            level,
            Category::Storage,
            format!("{} is almost full", v.mount),
            format!(
                "{} free of {} ({:.0} %).{}",
                format_bytes(v.free_bytes),
                format_bytes(v.total_bytes),
                free * 100.0,
                if system {
                    " A full system drive can block Windows updates and slow the PC."
                } else {
                    ""
                }
            ),
            vec![
                Step::settings(
                    "Run Disk Cleanup or Settings > System > Storage > Temporary files.",
                    "ms-settings:storagesense",
                ),
                Step::new("Uninstall unused games/apps or move them to another drive."),
                Step::new("SSDs also slow down when nearly full; keep 10-20 % free."),
            ],
        ));
    }
    if n == 0 {
        return Outcome::Skipped("No volumes found".into());
    }
    if out.is_empty() {
        return Outcome::Pass(format!("All {n} volumes have at least 10 % free"));
    }
    Outcome::Findings(out)
}

fn drive_health(ctx: &Context) -> Outcome {
    let mut out = Vec::new();
    let mut seen = Vec::new();
    for d in &ctx.info.drives {
        let Some(h) = &d.nvme_health else { continue };
        seen.push(format!("{}: {} %", d.name(), h.health_pct()));
        let name = d.name();
        let warnings = h.warnings();
        if !warnings.is_empty() || h.available_spare_pct < h.spare_threshold_pct {
            out.push(finding(
                Level::Critical,
                Category::Storage,
                format!("{name} reports a critical health warning"),
                format!(
                    "The drive's SMART log flags: {}.",
                    if warnings.is_empty() {
                        "spare capacity below threshold".to_string()
                    } else {
                        warnings.join(", ")
                    }
                ),
                &[
                    "Back up important data from this drive now.",
                    "Plan to replace the drive.",
                ],
            ));
            continue;
        }
        let level = match h.health_pct() {
            p if p < 50 => Some(Level::Critical),
            p if p < 80 => Some(Level::Warning),
            _ => None,
        };
        if let Some(level) = level {
            out.push(finding(
                level,
                Category::Storage,
                format!("{name} is {} % worn", h.percentage_used),
                format!(
                    "The drive estimates {} % of its rated write endurance is used ({:.1} TB written).",
                    h.percentage_used,
                    h.data_written_bytes as f64 / 1e12
                ),
                &["Keep backups current.", "Plan a replacement as health approaches 10-20 %."],
            ));
        }
        if h.media_errors > 0 {
            out.push(finding(
                Level::Warning,
                Category::Storage,
                format!("{name} has recorded media errors"),
                format!(
                    "{} unrecovered data integrity errors are logged.",
                    h.media_errors
                ),
                &[
                    "Back up important data and watch whether the count grows.",
                    "Check the manufacturer's tool for firmware updates.",
                ],
            ));
        }
    }
    if seen.is_empty() {
        let has_nvme = ctx.info.drives.iter().any(|d| d.bus == BusType::Nvme);
        return Outcome::Skipped(if has_nvme {
            "NVMe health log unavailable (try running as administrator)".into()
        } else {
            "No NVMe drives (SATA drives are covered by the SATA SMART check)".into()
        });
    }
    if out.is_empty() {
        return Outcome::Pass(seen.join("; "));
    }
    Outcome::Findings(out)
}

/// SATA drives that can carry ATA SMART (not NVMe, USB, virtual or card readers).
fn sata_like(bus: BusType) -> bool {
    !matches!(
        bus,
        BusType::Nvme
            | BusType::Usb
            | BusType::Virtual
            | BusType::Sd
            | BusType::Mmc
            | BusType::Spaces
    )
}

fn sata_smart(ctx: &Context) -> Outcome {
    let mut out = Vec::new();
    let mut seen = Vec::new();
    for d in &ctx.info.drives {
        let Some(s) = &d.ata_smart else { continue };
        let name = d.name();
        let mut summary = vec![];
        if let Some(life) = s.life_remaining_pct() {
            summary.push(format!("{life} % life left"));
        }
        if let Some(h) = s.power_on_hours() {
            summary.push(format!("{h} h"));
        }
        seen.push(if summary.is_empty() {
            format!("{name}: OK")
        } else {
            format!("{name}: {}", summary.join(", "))
        });

        let realloc = s.reallocated_sectors().unwrap_or(0);
        let pending = s.pending_sectors().unwrap_or(0);
        let uncorrectable = s.uncorrectable().unwrap_or(0);
        if realloc + pending + uncorrectable > 0 {
            let level = if pending > 10 || uncorrectable > 10 {
                Level::Critical
            } else {
                Level::Warning
            };
            out.push(finding(
                level,
                Category::Storage,
                format!("{name} has bad or unstable sectors"),
                format!(
                    "SMART reports {realloc} reallocated, {pending} pending and {uncorrectable} uncorrectable sectors. \
                     These counts normally stay at 0; rising values mean the drive's media is failing."
                ),
                &[
                    "Back up important data from this drive now.",
                    "Check again in a few days; if any count grows, replace the drive.",
                    "Run the drive maker's diagnostic tool for an extended self-test.",
                ],
            ));
        }
        if let Some(life) = s.life_remaining_pct() {
            let level = match life {
                l if l < 10 => Some(Level::Critical),
                l if l < 20 => Some(Level::Warning),
                _ => None,
            };
            if let Some(level) = level {
                out.push(finding(
                    level,
                    Category::Storage,
                    format!("{name} is nearly worn out ({life} % life left)"),
                    format!("The SSD estimates {life} % of its rated write endurance remains."),
                    &["Keep backups current.", "Plan to replace the drive soon."],
                ));
            }
        }
        if let Some(crc) = s.crc_errors().filter(|&c| c > 0) {
            out.push(finding(
                Level::Info,
                Category::Storage,
                format!("{name} has recorded cable (CRC) errors"),
                format!(
                    "{crc} interface CRC errors are logged. These come from the SATA cable or port, not the drive itself; \
                     the count never resets, so only an increase means an ongoing problem."
                ),
                &[
                    "Reseat or replace the SATA data cable, or move it to another SATA port.",
                    "Watch whether the count keeps growing.",
                ],
            ));
        }
    }
    if seen.is_empty() {
        let has_sata = ctx.info.drives.iter().any(|d| sata_like(d.bus));
        return Outcome::Skipped(if has_sata {
            "SATA SMART data unavailable (try running as administrator)".into()
        } else {
            "No SATA drives".into()
        });
    }
    if out.is_empty() {
        return Outcome::Pass(seen.join("; "));
    }
    Outcome::Findings(out)
}

fn drive_temperature(ctx: &Context) -> Outcome {
    let mut out = Vec::new();
    let mut seen = Vec::new();
    for d in &ctx.info.drives {
        let Some(t) = ctx.recent_avg(&format!("{}/temp", d.sensor_prefix()), RECENT) else {
            continue;
        };
        seen.push(format!("{}: {t:.0} °C", d.name()));
        let level = match t {
            t if t >= 80.0 => Level::Critical,
            t if t >= 70.0 => Level::Warning,
            _ => continue,
        };
        out.push(finding(
            level,
            Category::Storage,
            format!("{} is running hot", d.name()),
            format!("Drive temperature has averaged {t:.0} °C. NVMe drives typically throttle around 70-80 °C."),
            &[
                "Use the motherboard's M.2 heatsink (remove its protective film) or add one.",
                "Make sure the drive isn't directly under a hot GPU without airflow.",
            ],
        ));
    }
    if seen.is_empty() {
        return Outcome::Skipped("No drive temperature sensor".into());
    }
    if out.is_empty() {
        return Outcome::Pass(seen.join("; "));
    }
    Outcome::Findings(out)
}

// ---- System ---------------------------------------------------------------------------------

fn battery_wear(ctx: &Context) -> Outcome {
    let Some(b) = &ctx.info.battery_health else {
        return Outcome::Skipped(if ctx.info.battery.is_some() {
            "Battery capacity not reported".into()
        } else {
            "No battery".into()
        });
    };
    let wear = b.wear_pct();
    let level = match wear {
        w if w > 50.0 => Level::Critical,
        w if w > 30.0 => Level::Warning,
        _ => return Outcome::Pass(b.summary()),
    };
    Outcome::Findings(vec![finding(
        level,
        Category::System,
        format!("Battery has lost {wear:.0} % of its capacity"),
        format!(
            "The battery now holds {}. Runtime is reduced accordingly.",
            b.summary()
        ),
        &[
            "If the reading seems wrong, calibrate: charge to 100 %, run down to a few percent, then charge fully again.",
            "Consider a replacement battery from the laptop maker.",
            "To slow further wear, enable the maker's charge limit (e.g. stop at 80 %) if you're mostly plugged in.",
        ],
    )])
}

fn power_plan(ctx: &Context) -> Outcome {
    // Prefer the live power info: the user may have switched plans since startup.
    let live = ctx
        .power
        .and_then(|p| p.active_scheme())
        .map(|s| (s.name.as_str(), s.kind));
    let Some((name, kind)) = live.or(ctx
        .info
        .power_plan
        .as_ref()
        .map(|p| (p.name.as_str(), p.kind)))
    else {
        return Outcome::Skipped("Power plan not available".into());
    };
    let name = if name.is_empty() { "Custom plan" } else { name };
    if kind != PlanKind::PowerSaver {
        return Outcome::Pass(format!("Active plan: {name}"));
    }
    let laptop = ctx.info.battery.is_some();
    Outcome::Findings(vec![finding(
        if laptop { Level::Info } else { Level::Warning },
        Category::System,
        "Power saver plan is active",
        "Windows is limiting CPU performance to save power.",
        &["Switch to Balanced (or High performance on desktops) in Control Panel > Power Options."],
    )
    .with_fix(crate::power::Action::SetScheme(crate::power::BALANCED.into()))])
}

/// Parse SMBIOS BIOS dates ("MM/DD/YYYY", sometimes "MM/DD/YY").
pub fn parse_bios_date(s: &str) -> Option<(i64, u32, u32)> {
    let mut it = s.trim().split('/');
    let m: u32 = it.next()?.parse().ok()?;
    let d: u32 = it.next()?.parse().ok()?;
    let mut y: i64 = it.next()?.parse().ok()?;
    if y < 100 {
        y += if y < 80 { 2000 } else { 1900 };
    }
    ((1..=12).contains(&m) && (1..=31).contains(&d)).then_some((y, m, d))
}

fn bios_age(ctx: &Context) -> Outcome {
    let Some(b) = ctx.info.smbios.as_ref().and_then(|s| s.bios.as_ref()) else {
        return Outcome::Skipped("BIOS information not available".into());
    };
    let Some((y, m, _)) = parse_bios_date(&b.release_date) else {
        return Outcome::Skipped(format!("Unrecognized BIOS date \"{}\"", b.release_date));
    };
    let (ty, tm, _) = ctx.today;
    let months = (ty - y) * 12 + tm as i64 - m as i64;
    if months < 36 {
        return Outcome::Pass(format!("BIOS {} from {}", b.version, b.release_date));
    }
    Outcome::Findings(vec![
        guided(
            Level::Info,
            Category::System,
            format!("BIOS is {} years old", months / 12),
            format!(
                "BIOS {} was released {}. Newer versions often fix stability, security and compatibility issues.",
                b.version, b.release_date
            ),
            vec![
                bios_update_step(
                    ctx,
                    "Check the motherboard or laptop vendor's support page for a newer BIOS, and read its release notes before updating.",
                ),
                Step::new(
                    "Follow the vendor's update instructions exactly (many boards update from a USB stick inside the BIOS setup), and don't turn the PC off while it updates.",
                ),
            ],
        )
        .bios(),
    ])
}

fn uptime(ctx: &Context) -> Outcome {
    let Some(s) = ctx.uptime_secs else {
        return Outcome::Skipped("Uptime not available".into());
    };
    let days = s / 86400;
    if days < 14 {
        return Outcome::Pass(crate::format_uptime(s));
    }
    Outcome::Findings(vec![finding(
        Level::Info,
        Category::System,
        format!("Windows has been running for {days} days"),
        format!(
            "Windows has been running for {days} days without a restart. Over long sessions, apps that leak memory or handles, stuck background processes and misbehaving drivers can build up and slow things down."
        ),
        &[
            "Restart to clear them out: Start > Power > Restart.",
            "With Fast Startup on, \"Shut down\" resumes the previous session instead of starting fresh, so this counter keeps running. Restart always starts fresh.",
        ],
    )])
}

// ---- ReBAR and Windows settings -------------------------------------------------------------

fn resizable_bar(ctx: &Context) -> Outcome {
    let mut out = Vec::new();
    let mut seen = Vec::new();
    for g in ctx.info.gpus.iter().filter(|g| !g.is_integrated()) {
        match g.rebar() {
            ReBar::Enabled { bar_bytes } => {
                seen.push(format!("{}: on ({} BAR)", g.name, format_bytes(bar_bytes)))
            }
            ReBar::Disabled { bar_bytes } => {
                if g.supports_rebar() == Some(false) {
                    seen.push(format!("{}: not supported by this GPU", g.name));
                    continue;
                }
                seen.push(format!("{}: off", g.name));
                let mut steps = bios_help::bios_steps(
                    ctx.info,
                    BiosSetting::ResizableBar,
                    "Enable \"Above 4G Decoding\" and \"Re-Size BAR Support\".",
                    "",
                );
                steps.push(Step::new(
                    "If the option is missing or greyed out, disable CSM / Legacy boot so the PC boots in UEFI mode (Windows must be on a GPT disk).",
                ));
                steps.push(Step::new(
                    "Update the motherboard BIOS and GPU driver; RTX 30 cards also need a ReBAR-capable VBIOS from the card maker.",
                ));
                out.push(
                    guided(
                        Level::Warning,
                        Category::Gpu,
                        format!("Resizable BAR is off for {}", g.name),
                        format!(
                            "The CPU can only see a {} window of the GPU's {} memory. With Resizable BAR on, many games gain a few percent (some 10 % or more).",
                            format_bytes(bar_bytes),
                            format_bytes(g.dedicated_vram)
                        ),
                        steps,
                    )
                    .bios(),
                );
            }
            ReBar::Unknown => {}
        }
    }
    if seen.is_empty() {
        return Outcome::Skipped("ReBAR status is only available for NVIDIA GPUs so far".into());
    }
    if out.is_empty() {
        return Outcome::Pass(seen.join("; "));
    }
    Outcome::Findings(out)
}

fn gpu_driver_age(ctx: &Context) -> Outcome {
    let (ty, tm, _) = ctx.today;
    let mut out = Vec::new();
    let mut seen = Vec::new();
    for g in ctx.info.gpus.iter().filter(|g| !g.is_integrated()) {
        let Some((y, m, d)) = g.driver_date else {
            continue;
        };
        seen.push(format!("{}: {y}-{m:02}-{d:02}", g.name));
        let months = (ty - y) * 12 + tm as i64 - m as i64;
        if months >= 12 {
            out.push(guided(
                Level::Info,
                Category::Gpu,
                format!("{} driver is over a year old", g.name),
                format!("The installed driver is dated {y}-{m:02}-{d:02}. Newer drivers often bring game fixes and performance improvements."),
                vec![setup::driver_step(g.vendor_id, "the latest driver")],
            ));
        }
    }
    if seen.is_empty() {
        return Outcome::Skipped("Driver date not available".into());
    }
    if out.is_empty() {
        return Outcome::Pass(seen.join("; "));
    }
    Outcome::Findings(out)
}

fn pending_restart(ctx: &Context) -> Outcome {
    let reasons = &ctx.info.windows.pending_reboot;
    if reasons.is_empty() {
        return Outcome::Pass("No restart pending".into());
    }
    Outcome::Findings(vec![guided(
        Level::Info,
        Category::System,
        "Windows is waiting for a restart",
        format!(
            "A restart is pending for: {}. Updates aren't fully applied until then.",
            reasons.join(", ")
        ),
        vec![Step::settings(
            "Restart (not Shut down) when convenient, or schedule it in Settings > Windows Update.",
            "ms-settings:windowsupdate",
        )],
    )])
}

fn memory_integrity(ctx: &Context) -> Outcome {
    let w = &ctx.info.windows;
    match w.hvci_running {
        None => Outcome::Skipped("Memory Integrity status not available".into()),
        Some(false) => Outcome::Pass(
            if w.vbs_configured == Some(true) {
                "Off (VBS configured)"
            } else {
                "Off"
            }
            .into(),
        ),
        Some(true) => Outcome::Findings(vec![guided(
            Level::Info,
            Category::System,
            "Memory Integrity (VBS) is on",
            "Core isolation's Memory Integrity protects the kernel against malware, but costs some performance in CPU-heavy games (commonly a few percent, up to about 10 %).",
            vec![
                Step::new(
                    "Keep it on unless you need the last few percent of gaming performance; it is an important security feature.",
                ),
                Step::settings(
                    "To change it: Windows Security > Device security > Core isolation > Memory integrity (restart required).",
                    "windowsdefender://coreisolation",
                ),
            ],
        )
        .security()]),
    }
}

fn gpu_scheduling(ctx: &Context) -> Outcome {
    match ctx.info.windows.hags {
        None => Outcome::Skipped("Not configured; Windows uses the driver's default".into()),
        Some(true) => Outcome::Pass("On".into()),
        Some(false) => Outcome::Findings(vec![guided(
            Level::Info,
            Category::System,
            "Hardware-accelerated GPU scheduling is off",
            "Hardware GPU scheduling can lower latency slightly and is required for DLSS Frame Generation on NVIDIA GPUs.",
            vec![Step::settings(
                "Turn it on in Settings > System > Display > Graphics > Change default graphics settings (restart required).",
                "ms-settings:display-advancedgraphics",
            )],
        )]),
    }
}

fn game_mode(ctx: &Context) -> Outcome {
    match ctx.info.windows.game_mode {
        None => Outcome::Skipped("Game Mode setting not available".into()),
        Some(true) => Outcome::Pass("On".into()),
        Some(false) => Outcome::Findings(vec![guided(
            Level::Info,
            Category::System,
            "Game Mode is off",
            "Game Mode stops Windows Update from installing drivers or showing restart prompts during games, and prioritizes the game.",
            vec![Step::settings(
                "Turn it on in Settings > Gaming > Game Mode.",
                "ms-settings:gaming-gamemode",
            )],
        )]),
    }
}

fn pagefile(ctx: &Context) -> Outcome {
    match ctx.info.windows.pagefile_disabled {
        None => Outcome::Skipped("Pagefile setting not available".into()),
        Some(false) => Outcome::Pass("Enabled".into()),
        Some(true) => Outcome::Findings(vec![finding(
            Level::Warning,
            Category::System,
            "The pagefile is disabled",
            "Without a pagefile, apps and games crash with out-of-memory errors once RAM fills up, even when plenty of disk space is free.",
            &[
                "Re-enable it: System Properties > Advanced > Performance Settings > Advanced > Virtual memory > \"Automatically manage paging file size\".",
            ],
        )]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpuid::CpuInfo;
    use crate::sensors::{Reading, Unit};
    use crate::smbios::{Bios, MemoryDevice, Smbios};
    use crate::storage::{Drive, NvmeHealth, Volume};
    use crate::win::{MemoryStatus, PowerPlan};

    fn dimm(locator: &str, size_mb: u64, part: &str, speed: u32, configured: u32) -> MemoryDevice {
        MemoryDevice {
            locator: locator.into(),
            size_mb,
            part_number: part.into(),
            speed_mts: speed,
            configured_speed_mts: configured,
            memory_type: "DDR4",
            form_factor: "DIMM",
            ..Default::default()
        }
    }

    fn system(dimms: Vec<MemoryDevice>) -> SystemInfo {
        SystemInfo {
            smbios: Some(Smbios {
                memory_slots: 4,
                memory: dimms,
                bios: Some(Bios {
                    vendor: "AMI".into(),
                    version: "1.0".into(),
                    release_date: "10/21/2024".into(),
                }),
                ..Default::default()
            }),
            memory: Some(MemoryStatus {
                load_percent: 30,
                total_bytes: 32 << 30,
                available_bytes: 20 << 30,
            }),
            ..Default::default()
        }
    }

    pub(super) fn ctx<'a>(info: &'a SystemInfo, sensors: &'a SensorTable) -> Context<'a> {
        Context {
            info,
            sensors,
            today: (2026, 10, 12),
            uptime_secs: Some(3600),
            power: None,
        }
    }

    fn feed(t: &mut SensorTable, id: &str, unit: Unit, values: &[f64]) {
        for &v in values {
            t.update([Reading {
                id: id.into(),
                label: id.into(),
                group: "g".into(),
                unit,
                value: v,
            }]);
        }
    }

    pub(super) fn levels(o: &Outcome) -> Vec<Level> {
        match o {
            Outcome::Findings(f) => f.iter().map(|f| f.level).collect(),
            _ => Vec::new(),
        }
    }

    #[test]
    fn part_number_speeds() {
        assert_eq!(rated_speed_from_part("CMW32GX4M2C3200C16"), Some(3200));
        assert_eq!(rated_speed_from_part("F5-6000J3038F16G"), Some(6000));
        assert_eq!(rated_speed_from_part("KF432C16BB/8"), None);
        assert_eq!(rated_speed_from_part(""), None);
    }

    #[test]
    fn channel_letters() {
        assert_eq!(channel_letter("DIMM_A1"), Some('A'));
        assert_eq!(channel_letter("Controller1-DIMMB2"), Some('B'));
        assert_eq!(channel_letter("ChannelB-DIMM0"), Some('B'));
        assert_eq!(channel_letter("P0 CHANNEL A"), Some('A'));
        assert_eq!(channel_letter("DIMM0"), None);
        assert_eq!(channel_letter("BANK 0"), None);
    }

    #[test]
    fn xmp_disabled_is_flagged() {
        let info = system(vec![
            dimm("DIMM_A2", 16384, "F4-3600C16-16GVKC", 2133, 2133),
            dimm("DIMM_B2", 16384, "F4-3600C16-16GVKC", 2133, 2133),
        ]);
        let s = SensorTable::default();
        let o = memory_speed(&ctx(&info, &s));
        assert_eq!(levels(&o), vec![Level::Warning]);
        let Outcome::Findings(f) = o else {
            unreachable!()
        };
        assert!(f[0].title.contains("2133") && f[0].title.contains("3600"));
    }

    #[test]
    fn rated_memory_passes() {
        let info = system(vec![
            dimm(
                "Controller0-DIMMA2",
                16384,
                "CMW32GX4M2C3200C16",
                3200,
                3200,
            ),
            dimm(
                "Controller1-DIMMB2",
                16384,
                "CMW32GX4M2C3200C16",
                3200,
                3200,
            ),
        ]);
        let s = SensorTable::default();
        let c = ctx(&info, &s);
        assert!(matches!(memory_speed(&c), Outcome::Pass(_)));
        assert!(matches!(memory_channels(&c), Outcome::Pass(_)));
    }

    #[test]
    fn channel_problems() {
        let s = SensorTable::default();
        let single = system(vec![dimm("DIMM_A2", 16384, "X", 3200, 3200)]);
        assert_eq!(
            levels(&memory_channels(&ctx(&single, &s))),
            vec![Level::Warning]
        );
        let same = system(vec![
            dimm("DIMM_A1", 8192, "X", 3200, 3200),
            dimm("DIMM_A2", 16384, "Y", 3200, 3200),
        ]);
        assert_eq!(
            levels(&memory_channels(&ctx(&same, &s))),
            vec![Level::Warning, Level::Info]
        );
    }

    #[test]
    fn raptor_microcode() {
        let s = SensorTable::default();
        let mut info = SystemInfo {
            cpu: CpuInfo {
                vendor: Vendor::Intel,
                family: 6,
                model: 0xB7,
                brand: "13th Gen Intel(R) Core(TM) i9-13900K".into(),
                ..Default::default()
            },
            microcode: Some(0x11F),
            ..Default::default()
        };
        assert_eq!(
            levels(&intel_raptor_microcode(&ctx(&info, &s))),
            vec![Level::Critical]
        );
        info.microcode = Some(0x12B);
        assert!(matches!(
            intel_raptor_microcode(&ctx(&info, &s)),
            Outcome::Pass(_)
        ));
        info.cpu.brand = "13th Gen Intel(R) Core(TM) i9-13900T".into();
        info.microcode = Some(0x11F);
        assert!(matches!(
            intel_raptor_microcode(&ctx(&info, &s)),
            Outcome::Skipped(_)
        ));
        info.cpu.model = 0x97;
        assert!(matches!(
            intel_raptor_microcode(&ctx(&info, &s)),
            Outcome::Skipped(_)
        ));
    }

    #[test]
    fn throttling_needs_load_and_flags_low_clocks() {
        let info = SystemInfo {
            base_mhz: Some(3500),
            ..Default::default()
        };
        let mut s = SensorTable::default();
        feed(&mut s, "cpu/load/total", Unit::Percent, &[5.0; 20]);
        feed(&mut s, "cpu/clock/avg", Unit::Mhz, &[4800.0; 20]);
        assert!(matches!(
            cpu_throttling(&ctx(&info, &s)),
            Outcome::Skipped(_)
        ));
        feed(&mut s, "cpu/load/total", Unit::Percent, &[95.0; 15]);
        feed(&mut s, "cpu/clock/avg", Unit::Mhz, &[2400.0; 15]);
        assert_eq!(
            levels(&cpu_throttling(&ctx(&info, &s))),
            vec![Level::Warning]
        );
    }

    #[test]
    fn memory_pressure_levels() {
        let info = SystemInfo::default();
        let mut s = SensorTable::default();
        feed(&mut s, "mem/load", Unit::Percent, &[50.0; 5]);
        assert!(matches!(
            memory_pressure(&ctx(&info, &s)),
            Outcome::Skipped(_)
        ));
        feed(&mut s, "mem/load", Unit::Percent, &[98.0; 200]);
        assert_eq!(
            levels(&memory_pressure(&ctx(&info, &s))),
            vec![Level::Critical]
        );
    }

    #[test]
    fn storage_checks() {
        let mut info = SystemInfo::default();
        info.drives.push(Drive {
            index: 0,
            model: "Test SSD".into(),
            bus: BusType::Nvme,
            volumes: vec![
                Volume {
                    mount: "C:".into(),
                    total_bytes: 1000,
                    free_bytes: 30,
                    ..Default::default()
                },
                Volume {
                    mount: "D:".into(),
                    total_bytes: 1000,
                    free_bytes: 500,
                    ..Default::default()
                },
            ],
            nvme_health: Some(NvmeHealth {
                percentage_used: 35,
                available_spare_pct: 100,
                spare_threshold_pct: 10,
                media_errors: 2,
                ..Default::default()
            }),
            ..Default::default()
        });
        let mut s = SensorTable::default();
        let c = ctx(&info, &s);
        assert_eq!(levels(&drive_free_space(&c)), vec![Level::Critical]);
        assert_eq!(
            levels(&drive_health(&c)),
            vec![Level::Warning, Level::Warning]
        );
        feed(&mut s, "disk/0/temp", Unit::Celsius, &[74.0; 30]);
        assert_eq!(
            levels(&drive_temperature(&ctx(&info, &s))),
            vec![Level::Warning]
        );
    }

    #[test]
    fn system_checks() {
        let s = SensorTable::default();
        let mut info = system(vec![]);
        info.power_plan = Some(PowerPlan {
            name: "Power saver".into(),
            guid: String::new(),
            kind: PlanKind::PowerSaver,
        });
        assert_eq!(levels(&power_plan(&ctx(&info, &s))), vec![Level::Warning]);
        assert!(matches!(bios_age(&ctx(&info, &s)), Outcome::Pass(_)));
        info.smbios
            .as_mut()
            .unwrap()
            .bios
            .as_mut()
            .unwrap()
            .release_date = "03/15/2021".into();
        assert_eq!(levels(&bios_age(&ctx(&info, &s))), vec![Level::Info]);
        let mut c = ctx(&info, &s);
        c.uptime_secs = Some(20 * 86400);
        assert_eq!(levels(&uptime(&c)), vec![Level::Info]);
        assert_eq!(parse_bios_date("06/15/24"), Some((2024, 6, 15)));
    }

    #[test]
    fn monitor_on_motherboard() {
        use crate::gpu::{Display, GpuAdapter};
        let s = SensorTable::default();
        let disp = |n: &str| Display {
            name: format!(r"\\.\{n}"),
            width: 2560,
            height: 1440,
        };
        let card = |displays| GpuAdapter {
            name: "RTX".into(),
            vendor_id: 0x10DE,
            device_id: 0x2204,
            dedicated_vram: 24 << 30,
            displays,
            ..Default::default()
        };
        let igpu = |displays| GpuAdapter {
            name: "UHD 770".into(),
            vendor_id: 0x8086,
            device_id: 0xA780,
            dedicated_vram: 128 << 20,
            displays,
            ..Default::default()
        };

        let mut info = SystemInfo {
            gpus: vec![card(vec![]), igpu(vec![disp("DISPLAY1")])],
            ..Default::default()
        };
        let o = monitor_connection(&ctx(&info, &s));
        assert_eq!(levels(&o), vec![Level::Critical]);
        let Outcome::Findings(f) = o else {
            unreachable!()
        };
        assert!(f[0].detail.contains("DISPLAY1 (2560x1440)"));

        info.gpus = vec![card(vec![disp("DISPLAY1")]), igpu(vec![disp("DISPLAY2")])];
        assert_eq!(
            levels(&monitor_connection(&ctx(&info, &s))),
            vec![Level::Info]
        );

        info.gpus = vec![card(vec![disp("DISPLAY1")]), igpu(vec![])];
        assert!(matches!(
            monitor_connection(&ctx(&info, &s)),
            Outcome::Pass(_)
        ));

        info.gpus = vec![igpu(vec![disp("DISPLAY1")])];
        assert!(matches!(
            monitor_connection(&ctx(&info, &s)),
            Outcome::Skipped(_)
        ));

        info.gpus = vec![card(vec![]), igpu(vec![disp("DISPLAY1")])];
        info.battery = Some(Default::default());
        assert!(matches!(
            monitor_connection(&ctx(&info, &s)),
            Outcome::Skipped(_)
        ));
    }

    #[test]
    fn rebar_check() {
        use crate::gpu::{GpuAdapter, NvmlDetails};
        let s = SensorTable::default();
        let card = |bar| GpuAdapter {
            name: "RTX 3090".into(),
            vendor_id: 0x10DE,
            device_id: 0x2204,
            dedicated_vram: 24 << 30,
            nvml: Some(NvmlDetails {
                bar1_total: Some(bar),
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut info = SystemInfo {
            gpus: vec![card(256 << 20)],
            ..Default::default()
        };
        assert_eq!(
            levels(&resizable_bar(&ctx(&info, &s))),
            vec![Level::Warning]
        );
        info.gpus = vec![card(32 << 30)];
        assert!(matches!(resizable_bar(&ctx(&info, &s)), Outcome::Pass(_)));
        // Turing doesn't support ReBAR, so a legacy-size BAR is expected.
        info.gpus[0].device_id = 0x1E04;
        info.gpus[0].nvml.as_mut().unwrap().bar1_total = Some(256 << 20);
        assert!(matches!(resizable_bar(&ctx(&info, &s)), Outcome::Pass(_)));
        info.gpus[0].nvml = None;
        assert!(matches!(
            resizable_bar(&ctx(&info, &s)),
            Outcome::Skipped(_)
        ));
    }

    #[test]
    fn driver_age_check() {
        use crate::gpu::GpuAdapter;
        let s = SensorTable::default();
        let g = GpuAdapter {
            name: "RTX".into(),
            vendor_id: 0x10DE,
            device_id: 0x2204,
            dedicated_vram: 24 << 30,
            driver_date: Some((2026, 9, 30)),
            ..Default::default()
        };
        let mut info = SystemInfo {
            gpus: vec![g],
            ..Default::default()
        };
        assert!(matches!(gpu_driver_age(&ctx(&info, &s)), Outcome::Pass(_)));
        info.gpus[0].driver_date = Some((2025, 6, 1));
        assert_eq!(levels(&gpu_driver_age(&ctx(&info, &s))), vec![Level::Info]);
    }

    #[test]
    fn windows_setting_checks() {
        use crate::win::settings::WindowsSettings;
        let s = SensorTable::default();
        let run = |i: &SystemInfo| {
            let c = ctx(i, &s);
            [
                pending_restart(&c),
                memory_integrity(&c),
                gpu_scheduling(&c),
                game_mode(&c),
                pagefile(&c),
            ]
        };
        let mut info = SystemInfo::default();
        let r = run(&info);
        assert!(matches!(r[0], Outcome::Pass(_)));
        assert!(r[1..].iter().all(|o| matches!(o, Outcome::Skipped(_))));
        info.windows = WindowsSettings {
            pending_reboot: vec!["Windows Update"],
            hvci_running: Some(true),
            vbs_configured: Some(true),
            hags: Some(false),
            game_mode: Some(false),
            pagefile_disabled: Some(true),
        };
        let r = run(&info);
        let Outcome::Findings(f) = &r[1] else {
            panic!("memory integrity finding")
        };
        assert!(
            f[0].security,
            "Memory Integrity is flagged as a security setting"
        );
        let got: Vec<Vec<Level>> = r.iter().map(levels).collect();
        assert_eq!(
            got,
            vec![
                vec![Level::Info],
                vec![Level::Info],
                vec![Level::Info],
                vec![Level::Info],
                vec![Level::Warning]
            ]
        );
    }

    #[test]
    fn sata_smart_findings() {
        use crate::smart::{parse_ata_smart, tests::sector};
        let s = SensorTable::default();
        let drive = |smart| Drive {
            index: 1,
            model: "Test SATA SSD".into(),
            bus: BusType::Sata,
            ata_smart: smart,
            ..Default::default()
        };
        let healthy = parse_ata_smart(&sector(&[
            (5, 100, 100, 0),
            (9, 99, 99, 1200),
            (231, 95, 95, 0),
        ]));
        let mut info = SystemInfo {
            drives: vec![drive(healthy)],
            ..Default::default()
        };
        let Outcome::Pass(p) = sata_smart(&ctx(&info, &s)) else {
            panic!("expected pass")
        };
        assert!(p.contains("95 % life left") && p.contains("1200 h"));

        let bad = parse_ata_smart(&sector(&[
            (5, 90, 90, 8),
            (197, 100, 100, 25),
            (199, 100, 100, 3),
            (231, 8, 8, 0),
        ]));
        info.drives = vec![drive(bad)];
        assert_eq!(
            levels(&sata_smart(&ctx(&info, &s))),
            vec![Level::Critical, Level::Critical, Level::Info]
        );

        let worn = parse_ata_smart(&sector(&[(5, 100, 100, 1), (177, 15, 15, 0)]));
        info.drives = vec![drive(worn)];
        assert_eq!(
            levels(&sata_smart(&ctx(&info, &s))),
            vec![Level::Warning, Level::Warning]
        );

        info.drives = vec![drive(None)];
        let Outcome::Skipped(why) = sata_smart(&ctx(&info, &s)) else {
            panic!("expected skip")
        };
        assert!(why.contains("administrator"));
        info.drives[0].bus = BusType::Nvme;
        assert!(
            matches!(sata_smart(&ctx(&info, &s)), Outcome::Skipped(w) if w == "No SATA drives")
        );
    }

    #[test]
    fn battery_wear_levels() {
        use crate::battery::BatteryHealth;
        let s = SensorTable::default();
        let mut info = SystemInfo::default();
        assert!(matches!(battery_wear(&ctx(&info, &s)), Outcome::Skipped(w) if w == "No battery"));
        let health = |full| BatteryHealth {
            design_mwh: 60_000,
            full_charge_mwh: full,
            cycle_count: Some(400),
            ..Default::default()
        };
        info.battery_health = Some(health(54_000));
        assert!(matches!(battery_wear(&ctx(&info, &s)), Outcome::Pass(_)));
        info.battery_health = Some(health(39_000));
        assert_eq!(levels(&battery_wear(&ctx(&info, &s))), vec![Level::Warning]);
        info.battery_health = Some(health(24_000));
        assert_eq!(
            levels(&battery_wear(&ctx(&info, &s))),
            vec![Level::Critical]
        );
    }

    #[test]
    fn gaming_checks_are_grouped() {
        let info = SystemInfo {
            windows: crate::win::settings::WindowsSettings {
                game_mode: Some(false),
                ..Default::default()
            },
            ..Default::default()
        };
        let s = SensorTable::default();
        let results = run(&ctx(&info, &s));
        let gm = results.iter().find(|r| r.name == "Game Mode").unwrap();
        assert_eq!(gm.category, Category::Gaming);
        let Outcome::Findings(f) = &gm.outcome else {
            panic!("game mode off is a finding")
        };
        assert_eq!(f[0].category, Category::Gaming);
    }

    #[test]
    fn run_reports_every_check() {
        let info = SystemInfo::default();
        let s = SensorTable::default();
        let results = run(&ctx(&info, &s));
        assert_eq!(results.len(), CHECKS.len() + power::CHECKS.len());
        assert!(findings(&results).is_empty());
    }

    #[test]
    fn links() {
        assert_eq!(
            Link::Search("ASUS ROG STRIX B650E-F  enable EXPO".into()).url(),
            "https://www.google.com/search?q=ASUS+ROG+STRIX+B650E-F+enable+EXPO"
        );
        assert_eq!(query_encode("a&b/c"), "a%26b%2Fc");
        assert_eq!(
            Link::Settings("ms-settings:display".into()).url(),
            "ms-settings:display"
        );
        assert_eq!(Step::from("x"), Step::new("x"));
    }
}
