//! In-app help: a window with one documentation tab per feature.
//!
//! Numbers that come from code (heatmap thresholds, bottleneck thresholds, the list of
//! Health Check rules) are read from the constants/code that use them, so the help stays in
//! step with the app.

use crate::heat;
use egui::{Context, RichText, Ui};
use omnihw_core::bottleneck as bn;
use omnihw_core::diagnostics::{self, Category};
use omnihw_core::sensors::SensorTable;
use std::sync::OnceLock;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum HelpTab {
    #[default]
    GettingStarted,
    Overview,
    HealthCheck,
    Hardware,
    Sensors,
    Recording,
    Alerts,
    Reports,
    Settings,
    Privacy,
    Troubleshooting,
}

const TABS: [(HelpTab, &str); 11] = [
    (HelpTab::GettingStarted, "Getting Started"),
    (HelpTab::Overview, "Overview"),
    (HelpTab::HealthCheck, "Health Check"),
    (HelpTab::Hardware, "Hardware Tabs"),
    (HelpTab::Sensors, "Sensors"),
    (HelpTab::Recording, "Recording"),
    (HelpTab::Alerts, "Alerts"),
    (HelpTab::Reports, "Reports"),
    (HelpTab::Settings, "Settings"),
    (HelpTab::Privacy, "Privacy & Safety"),
    (HelpTab::Troubleshooting, "Troubleshooting"),
];

/// Help window state: whether it's open and which tab is selected (kept while the app runs).
#[derive(Default)]
pub struct HelpState {
    pub open: bool,
    tab: HelpTab,
}

/// Show the help window if it's open.
pub fn show(ctx: &Context, state: &mut HelpState) {
    let mut open = state.open;
    egui::Window::new("Omni-HW Help")
        .open(&mut open)
        .resizable(true)
        .collapsible(false)
        .default_size([760.0, 560.0])
        .show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                for (tab, name) in TABS {
                    ui.selectable_value(&mut state.tab, tab, name);
                }
            });
            ui.separator();
            egui::ScrollArea::vertical()
                .id_salt(("help_scroll", state.tab as u8))
                .auto_shrink(false)
                .show(ui, |ui| match state.tab {
                    HelpTab::GettingStarted => getting_started(ui),
                    HelpTab::Overview => overview(ui),
                    HelpTab::HealthCheck => health_check(ui),
                    HelpTab::Hardware => hardware(ui),
                    HelpTab::Sensors => sensors(ui),
                    HelpTab::Recording => recording(ui),
                    HelpTab::Alerts => alerts(ui),
                    HelpTab::Reports => reports(ui),
                    HelpTab::Settings => settings(ui),
                    HelpTab::Privacy => privacy(ui),
                    HelpTab::Troubleshooting => troubleshooting(ui),
                });
        });
    state.open = open;
}

// ---- Formatting helpers ---------------------------------------------------------------------

fn h(ui: &mut Ui, text: &str) {
    ui.add_space(8.0);
    ui.label(RichText::new(text).size(16.0).strong());
    ui.add_space(2.0);
}

fn p(ui: &mut Ui, text: &str) {
    ui.add(egui::Label::new(text).wrap());
    ui.add_space(4.0);
}

fn bullets<S: AsRef<str>>(ui: &mut Ui, items: &[S]) {
    for t in items {
        ui.horizontal_wrapped(|ui| {
            ui.label("•");
            ui.add(egui::Label::new(t.as_ref()).wrap());
        });
    }
    ui.add_space(4.0);
}

/// Smaller, collapsed technical notes for curious readers.
fn details<S: AsRef<str>>(ui: &mut Ui, id: &str, items: &[S]) {
    egui::CollapsingHeader::new(RichText::new("Details").small().strong())
        .id_salt(("help_details", id))
        .show(ui, |ui| {
            for t in items {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("•").small().weak());
                    ui.add(egui::Label::new(RichText::new(t.as_ref()).small().weak()).wrap());
                });
            }
        });
    ui.add_space(4.0);
}

fn pct(f: f64) -> String {
    format!("{:.0} %", f * 100.0)
}

// ---- Tabs -----------------------------------------------------------------------------------

fn getting_started(ui: &mut Ui) {
    h(ui, "What is Omni-HW?");
    p(
        ui,
        "Omni-HW combines what CPU-Z, GPU-Z and HWiNFO do in one app: it identifies your hardware, shows live sensors, \
         and goes further by explaining what it finds (Health Check), alerting you, recording sessions and analyzing them.",
    );
    h(ui, "The navigation bar");
    bullets(
        ui,
        &[
            "Overview: a one-page dashboard of CPU, GPU, memory, storage and system, with live numbers and graphs.",
            "Health Check: automatic checks that flag common problems and explain how to fix them.",
            "Hardware menu: CPU, Caches, Mainboard, Memory, GPU, Storage - detailed hardware information, like CPU-Z and GPU-Z. The button shows the open page, e.g. \"Hardware: GPU\".",
            "Monitoring menu: Sensors (every live reading with min/max/average, like HWiNFO), Recording (record, compare, bottleneck analysis) and Alerts (Windows notifications when a sensor crosses a limit).",
            "On the right: Record (start/stop a recording from any tab), Report (save an HTML report or copy a text one), Info (this guide and About) and Settings.",
            "Prefer every page as its own tab? Settings > Navigation bar > Classic.",
        ],
    );
    h(ui, "Keyboard");
    p(
        ui,
        "Up and Down arrow keys move to the previous or next page within the current category: Overview and Health Check; \
         CPU through Storage; Sensors, Recording and Alerts. They stop at the first and last page, and are ignored while \
         you're typing in a text field or a menu is open.",
    );
    h(ui, "Static and live data");
    p(
        ui,
        "Hardware details (model names, sizes, versions) are read once when Omni-HW starts. Live sensors (clocks, usage, \
         temperatures, power) are read in the background once per second by default; the screen only redraws when new \
         data arrives, so Omni-HW stays light while it runs. Change the sampling interval on the Sensors tab.",
    );
    h(ui, "No driver needed");
    p(
        ui,
        "Omni-HW uses only standard Windows interfaces and the graphics driver's own monitoring library (NVIDIA). \
         It installs nothing and runs as a single program.",
    );
    h(ui, "Administrator prompt (UAC)");
    p(
        ui,
        "Reading the health (SMART) data of SATA drives needs administrator rights. If Omni-HW finds a SATA-type drive \
         and isn't already running as administrator, it asks once via the Windows UAC prompt when it starts. If you \
         click No, it simply carries on without SATA health; everything else works. PCs with only NVMe drives are \
         never asked.",
    );
    details(
        ui,
        "start",
        &[
            "Start Omni-HW with --no-elevate to skip the prompt.",
            "The About tab shows whether Omni-HW is running as administrator.",
        ],
    );
}

fn overview(ui: &mut Ui) {
    h(ui, "What it shows");
    p(
        ui,
        "The first tab puts the most useful information from every other tab on one page: your CPU and graphics card \
         names, the Health Check summary, and a card each for the processor, graphics card(s), memory, storage and system.",
    );
    bullets(
        ui,
        &[
            "Processor: average and fastest core clock, total usage, package power (where Windows reports it), a usage graph, and one tile per physical core.",
            "Graphics card: load, temperature, power and core clock, graphs of load and temperature, a VRAM bar, Resizable BAR status, fan speed, memory clock and power state.",
            "Memory: total, type and speed, the installed modules, a used/total bar and a load graph.",
            "Storage: each drive with temperature, activity, read/write speed, a health bar (NVMe) and a free-space bar per drive letter.",
            "System: Windows version, computer name, uptime, motherboard, BIOS, CPU socket and combined CPU + GPU power.",
        ],
    );
    h(ui, "Using it");
    bullets(
        ui,
        &[
            "Click a card's header to collapse or expand it.",
            "Click the Health Check line under the title to open the Health Check tab.",
            "Hover a core tile to see each of its threads' usage.",
            "Graphs show roughly the last two minutes (120 samples).",
        ],
    );
    h(ui, "Heatmap colors");
    p(
        ui,
        "Numbers, bars and tiles change color as a reading gets worse: the normal color, then orange (warning), then red (critical).",
    );
    bullets(
        ui,
        &[
            format!(
                "Core tiles: blue shade follows the core's busiest thread; orange with a warning triangle from {:.0} %, red with an exclamation mark from {:.0} %.",
                heat::CORE_LOAD.warn,
                heat::CORE_LOAD.crit
            ),
            format!(
                "GPU temperature: orange from {:.0} °C, red from {:.0} °C. Each segment of the temperature graph is colored by its own value (green, orange, red).",
                heat::GPU_TEMP.warn,
                heat::GPU_TEMP.crit
            ),
            format!(
                "Memory bar: turns orange from {} used and red from {}.",
                pct(heat::MEMORY_USED.warn),
                pct(heat::MEMORY_USED.crit)
            ),
            format!(
                "VRAM bar: turns orange from {} used and red from {}.",
                pct(heat::VRAM_USED.warn),
                pct(heat::VRAM_USED.crit)
            ),
            format!(
                "Drive health bar: green, orange below {:.0} % health, red below {:.0} %.",
                100.0 - heat::DRIVE_WEAR.warn,
                100.0 - heat::DRIVE_WEAR.crit
            ),
        ],
    );
    p(
        ui,
        "High CPU usage or warm temperatures under load are normal; the colors simply draw your eye to what's busy or hot. \
         The Health Check looks at sustained readings before calling something a problem.",
    );
}

/// Every Health Check rule with its category, taken from the diagnostics engine itself.
fn all_checks() -> &'static [(&'static str, Category)] {
    static CHECKS: OnceLock<Vec<(&'static str, Category)>> = OnceLock::new();
    CHECKS.get_or_init(|| {
        let info = omnihw_core::SystemInfo::default();
        let sensors = SensorTable::default();
        let ctx = diagnostics::Context {
            info: &info,
            sensors: &sensors,
            today: (2026, 1, 1),
            uptime_secs: None,
        };
        diagnostics::run(&ctx)
            .into_iter()
            .map(|r| (r.name, r.category))
            .collect()
    })
}

/// One-line explanation of a Health Check rule, by its name.
fn check_description(name: &str) -> &'static str {
    match name {
        "CPU microcode (13th/14th Gen Intel stability)" => {
            "13th/14th Gen Intel desktop CPUs need microcode 0x12B or newer (a BIOS update) to avoid degrading over time."
        }
        "CPU clocks under load" => {
            "While the CPU is busy, its average clock should stay near base clock; lower means thermal or power throttling."
        }
        "Memory speed (XMP / EXPO)" => {
            "Compares the speed your RAM runs at with the speed its part number is rated for (XMP/EXPO off is common)."
        }
        "Memory channel layout" => {
            "Flags a single module or modules sharing one channel (half the bandwidth), and mixed modules."
        }
        "Memory pressure" => {
            "RAM nearly full over the last couple of minutes, which causes stutters."
        }
        "Installed memory" => "Less than 16 GB is flagged as a suggestion.",
        "Monitor connected to the graphics card" => {
            "On desktops with a graphics card, flags monitors plugged into the motherboard's video ports."
        }
        "GPU PCIe link" => {
            "Checks the graphics card runs at its full PCIe width (e.g. x16), and full generation while busy (NVIDIA)."
        }
        "GPU temperature" => {
            "Graphics card running hot on average (warning from 83 °C, critical from 90 °C)."
        }
        "GPU memory (VRAM) pressure" => {
            "Video memory almost full (95 % or more), which makes games stutter."
        }
        "GPU driver age" => "Graphics driver more than a year old.",
        "Resizable BAR" => {
            "Resizable BAR off on a card that supports it (NVIDIA cards; AMD and Intel can't be read yet)."
        }
        "Drive free space" => "Drive letters with less than 10 % free (critical below 5 %).",
        "Drive health (NVMe SMART)" => {
            "NVMe wear, critical warnings and media errors from the drive's own health log."
        }
        "Drive health (SATA SMART)" => {
            "Reallocated, pending or uncorrectable sectors, SSD life left and cable errors (needs administrator)."
        }
        "Drive temperature" => "Drives averaging 70 °C or more (critical from 80 °C).",
        "Battery wear" => {
            "Laptop battery capacity lost compared with new (warning from 30 %, critical from 50 %)."
        }
        "Power plan" => "Windows Power saver plan active, which limits performance.",
        "BIOS age" => {
            "BIOS three or more years old; newer versions often fix stability and security issues."
        }
        "Uptime" => "Windows running for 14 days or more without a restart.",
        "Pending restart" => "Windows Update or a system component is waiting for a restart.",
        "Memory Integrity (VBS)" => {
            "Core isolation's Memory Integrity is on: a security feature that costs a little gaming performance."
        }
        "Hardware-accelerated GPU scheduling" => {
            "Off: turning it on can lower latency and is needed for DLSS Frame Generation."
        }
        "Game Mode" => "Windows Game Mode is off.",
        "Pagefile" => {
            "The pagefile is disabled, so programs crash instead of slowing down when RAM runs out."
        }
        _ => "",
    }
}

fn health_check(ui: &mut Ui) {
    h(ui, "What it does");
    p(
        ui,
        "The Health Check runs a set of rules over your hardware information and recent sensor readings, and reports \
         anything that looks wrong or could be better, in plain language with steps to fix it. Nothing is changed \
         automatically: Omni-HW only reads, you decide what to do.",
    );
    h(ui, "Reading the results");
    bullets(
        ui,
        &[
            "Critical (red): fix soon, e.g. a failing drive or outdated CPU microcode with a known stability problem.",
            "Warning (orange): something is holding your PC back, e.g. memory not running at its rated speed.",
            "Suggestion (blue, shown as INFO on cards): optional improvements.",
            "Passed: checks that found nothing wrong, with what was measured.",
            "Not checked: checks that don't apply to this PC, with the reason (e.g. no battery).",
        ],
    );
    h(ui, "Gaming section");
    p(
        ui,
        "Settings that mainly matter for games (Resizable BAR, Memory Integrity, hardware GPU scheduling, Game Mode) are \
         grouped in their own Gaming section, closed by default. Its header shows how many suggestions it has in blue, \
         so gamers know to open it, while the main summary only counts general findings.",
    );
    h(ui, "Security settings");
    p(
        ui,
        "Findings about a security setting carry an orange \"Security setting\" badge and a caution note. They explain \
         the trade-off; only change such a setting if you know what you are doing and understand the protection you \
         would give up.",
    );
    h(ui, "Using it");
    bullets(
        ui,
        &[
            "Click a card's header to collapse or expand it; warnings and critical issues start open, suggestions start collapsed.",
            "Expand all / Collapse all open or close every card at once.",
            "Live checks (memory pressure, temperatures, throttling) use roughly the last two minutes of readings and need at least a few seconds of data first.",
            "The throttling check needs some time with the CPU under heavy load (a game, a render) before it gives a verdict.",
            "Results also appear at the top of saved and copied reports.",
        ],
    );
    h(ui, "All checks");
    let categories = [
        Category::Cpu,
        Category::Memory,
        Category::Gpu,
        Category::Storage,
        Category::System,
        Category::Gaming,
    ];
    for cat in categories {
        let rows: Vec<String> = all_checks()
            .iter()
            .filter(|(_, c)| *c == cat)
            .map(|(name, _)| match check_description(name) {
                "" => (*name).to_string(),
                d => format!("{name}: {d}"),
            })
            .collect();
        if rows.is_empty() {
            continue;
        }
        ui.label(RichText::new(cat.name()).strong());
        bullets(ui, &rows);
    }
}

fn hardware(ui: &mut Ui) {
    p(
        ui,
        "These tabs show detailed, mostly fixed information about each part, read once when Omni-HW starts.",
    );
    h(ui, "CPU");
    bullets(
        ui,
        &[
            "Name, code name, microarchitecture, manufacturing process and socket.",
            "Family / model / stepping and the instruction sets the CPU supports (AVX2, AVX-512 and so on).",
            "Clocks: live average core speed, base clock, bus speed, multiplier and package power.",
            "Caches and topology: cores (performance and efficiency cores on hybrid CPUs), threads, NUMA nodes.",
        ],
    );
    details(
        ui,
        "cpu",
        &[
            "Read with the CPUID instruction and Windows' processor topology. Live clocks are Windows performance counters (% Processor Performance x base clock).",
            "VT-x may be missing from the instruction list when Hyper-V or Memory Integrity is on; Windows hides it from programs.",
        ],
    );
    h(ui, "Caches");
    p(
        ui,
        "Every cache level with its size, how many copies exist (e.g. one per core), associativity and line size, plus the raw CPUID cache descriptors.",
    );
    h(ui, "Mainboard");
    p(
        ui,
        "Motherboard maker and model, system maker and product, BIOS vendor, version and date, SMBIOS version and CPU socket.",
    );
    h(ui, "Memory");
    p(
        ui,
        "Memory type, installed and usable size, slots used, configured speed and live load, plus a table of every module with its size, rated and configured speed, ranks, maker and part number.",
    );
    h(ui, "GPU");
    bullets(
        ui,
        &[
            "Name, chip, architecture, process, card maker, PCI ids and revision.",
            "PCIe link (current vs maximum), BIOS version, driver version and date (NVIDIA shows more detail).",
            "Resizable BAR: lets the CPU access all of the graphics memory at once, which helps many modern games. Shown as On/Off on NVIDIA cards; Unknown on AMD and Intel for now.",
            "Displays: which monitors are connected to this card.",
            "Memory and limits (VRAM, bus width, CUDA cores, max clocks, power limit) and the card's live sensors.",
            "If you have more than one graphics adapter, pick it from the Adapter list.",
        ],
    );
    details(
        ui,
        "gpu",
        &[
            "Basic adapter information comes from DXGI and works for every vendor. Temperatures, clocks, power, fan, PCIe and ReBAR come from NVIDIA's NVML library, which ships with the NVIDIA driver.",
            "GPU load and VRAM use for any vendor come from the same Windows counters Task Manager uses.",
            "Virtual display adapters (used by streaming and virtual monitor tools) are hidden so your real card isn't listed twice.",
        ],
    );
    h(ui, "Storage");
    bullets(
        ui,
        &[
            "Each physical drive: model, type (NVMe SSD, SSD, HDD), interface, capacity, firmware and serial number.",
            "Volumes: each drive letter with a free-space bar.",
            "NVMe health: health %, spare capacity, temperature, data written and read, power-on hours, power cycles, unsafe shutdowns, media errors and warnings.",
            "SATA health (SMART): reallocated, pending and uncorrectable sectors, SSD life left, temperature and the full attribute table. Needs administrator rights.",
            "Live sensors: temperature, activity and read/write speed.",
        ],
    );
}

fn sensors(ui: &mut Ui) {
    h(ui, "What it shows");
    p(
        ui,
        "Every live reading Omni-HW collects, grouped by device (CPU clocks, CPU usage, memory, power, each GPU and drive). \
         For each sensor you see the current value and the minimum, maximum and average since Omni-HW started (or since \
         you last reset them).",
    );
    h(ui, "Using it");
    bullets(
        ui,
        &[
            "Reset min/max clears the statistics, e.g. right before starting a game or a stress test.",
            "Interval sets how often sensors are read: 250, 500, 1000 (default) or 2000 ms. Faster catches short spikes; slower uses less CPU.",
            "Click a group's header to collapse or expand it.",
        ],
    );
    details(
        ui,
        "sensors",
        &[
            "CPU clock per core is the fastest of its threads. GPU 'Load (busiest engine)' is how Task Manager measures GPU usage; 'Core Load' (NVIDIA) is what GPU-Z shows.",
            "Some sensors only exist on some systems: package power needs a CPU that reports it to Windows; battery sensors only appear on laptops.",
        ],
    );
}

fn recording(ui: &mut Ui) {
    h(ui, "Recording a session");
    bullets(
        ui,
        &[
            "Click Record in the tab bar (or Start recording on the Recording tab). Every sensor is recorded at the sampling interval until you click Stop.",
            r"Each recording is saved as a CSV file in your Documents folder named OmniHW-Recording-<date-time>.csv (opens in Excel). It is written as it records, so a crash loses at most one row.",
            "Sensors that only appear after recording starts aren't added to it.",
        ],
    );
    h(ui, "Session mode");
    p(
        ui,
        "Shows the current or last recording. Tick sensors on the left to chart them; sensors with the same unit share a chart. \
         Hover a chart to read exact values. Under each chart, a table lists each sensor's first, last, change, minimum, average and maximum. \
         Show CSV opens the folder with the file selected.",
    );
    h(ui, "Compare mode");
    bullets(
        ui,
        &[
            "Pick recording A and B: the one in memory or any saved recording in Documents (Refresh list rescans the folder).",
            "Charts overlay both sessions on elapsed time: A solid, B dashed. Only sensors present in both are offered.",
            "The table shows average and maximum for A and B and the difference; for temperatures and power, lower is shown in green.",
            "Typical uses: before vs after a repaste, a driver update, enabling XMP, or a new fan curve. Record the same game or test for the same length both times.",
        ],
    );
    h(ui, "Bottleneck mode");
    p(
        ui,
        "Tells you what limited performance during a recording: the graphics card, the CPU, or neither. Record a few minutes \
         of gameplay, stop the recording, then pick it here.",
    );
    bullets(
        ui,
        &[
            format!("GPU-bound: GPU load at {:.0} % or more. Normal when playing at high settings or resolution.", bn::GPU_BOUND),
            format!(
                "CPU-bound: GPU load below {:.0} % while the busiest CPU thread is at {:.0} % or more (or total CPU at {:.0} % or more). Games often wait on a single thread even when total CPU usage looks low.",
                bn::GPU_HEADROOM,
                bn::THREAD_BUSY,
                bn::CPU_TOTAL_BUSY
            ),
            "Not limited: neither is maxed out, usually because of a frame cap or V-Sync. That's fine.".into(),
            format!("Idle moments (GPU and CPU both below {:.0} %) are left out.", bn::IDLE),
            "The verdict is the situation that covers at least half of the active time; otherwise it's Mixed.".into(),
            format!(
                "With fewer than {} active samples the result is marked as low confidence.",
                bn::MIN_ACTIVE
            ),
        ],
    );
    p(
        ui,
        "Below the verdict you'll find a chart of GPU load and the busiest CPU thread with a colored strip showing the verdict \
         at each moment, contributing factors (VRAM or RAM nearly full, a single overloaded thread) and tips.",
    );
    details(
        ui,
        "rec",
        &[
            "The analysis uses load levels, not frame times, so frame caps are inferred.",
            "Recordings keep all samples in memory while they run (about 1 MB per hour at 1 s); CSVs are about 2 MB per hour.",
            "Saved recordings are matched in Compare by sensor group and name.",
        ],
    );
}

fn alerts(ui: &mut Ui) {
    h(ui, "What it does");
    p(
        ui,
        "Alerts watch a sensor and notify you when it stays above or below a limit for a while, e.g. \"GPU temperature over 85 °C for 30 seconds\". \
         They're checked on every sample while Omni-HW is running, whichever tab is open.",
    );
    h(ui, "Setting alerts up");
    bullets(
        ui,
        &[
            "Quick add offers common alerts for the sensors your PC has: GPU over 85 °C, CPU usage over 95 % for 2 minutes, memory over 90 % for 1 minute, each drive over 70 °C, and battery below 20 % on laptops.",
            "Add custom alert creates a rule you can point at any sensor: choose above or below, the limit, and how many seconds it must hold (0 = immediately).",
            "Untick On to pause a rule without deleting it; Remove deletes it.",
            "Test notification shows a sample notification so you can check Windows displays them.",
        ],
    );
    h(ui, "When an alert fires");
    bullets(
        ui,
        &[
            "You get a Windows notification and the taskbar button flashes if Omni-HW isn't in front.",
            "A red ALERT button appears in the tab bar; click it to open the Alerts tab.",
            "Each alert fires once, then re-arms when the reading goes back past the limit.",
            "History lists the alerts that fired since Omni-HW started.",
        ],
    );
    details(
        ui,
        "alerts",
        &[
            r"Rules are saved automatically to %APPDATA%\OmniHW\alerts.tsv.",
            "Windows hides notifications while Do Not Disturb / Focus is on or a full-screen game is running; the ALERT button and History still record them.",
        ],
    );
}

fn reports(ui: &mut Ui) {
    h(ui, "Save report");
    p(
        ui,
        "Writes a complete report as a web page (HTML) to your Documents folder and opens it in your browser. It starts with \
         the Health Check results, then the system, processor, mainboard, memory, every graphics card and drive, and a table \
         of all sensors with current, minimum, maximum and average. It works in light and dark mode and is easy to attach \
         to a forum post or support ticket.",
    );
    h(ui, "Copy");
    p(
        ui,
        "Copies the same report as plain text to the clipboard, ready to paste into a chat or forum.",
    );
    details(
        ui,
        "reports",
        &[
            "Reports are named OmniHW-Report-<date-time>.html (UTC time).",
            "Gaming findings are tagged (Gaming) and security-related ones carry a caution note, as in the app.",
            "Reports include serial numbers of drives; remove them before posting publicly if you prefer.",
        ],
    );
}

fn settings(ui: &mut Ui) {
    p(
        ui,
        "Settings change immediately and are saved automatically, so they survive restarts.",
    );
    h(ui, "Appearance");
    bullets(
        ui,
        &[
            "Theme: Match Windows (follows your light/dark mode), Dark or Light.",
            "UI scale: makes everything larger or smaller (75 % to 200 %). Reset returns to 100 %.",
            "Navigation bar: Grouped sorts pages into categories with dropdown menus so the bar fits on one line; Classic shows every page as its own tab (it may wrap onto two lines in small windows).",
        ],
    );
    h(ui, "Startup");
    bullets(
        ui,
        &[
            "Start page: the page Omni-HW opens on.",
            "Launch Omni-HW when I sign in to Windows: adds Omni-HW to your startup programs. Turn it off to remove it again.",
            "Start minimized: when launched at sign-in, start minimized instead of opening the window (in the system tray if Minimize to the system tray is on, otherwise in the taskbar).",
            "Minimize to the system tray (on by default): minimizing hides Omni-HW to the notification area next to the clock instead of the taskbar. Click the icon to bring the window back, or right-click it for Open / Exit. Sensors, recordings and alerts keep running while it's hidden.",
            "Ask for administrator rights at start: only matters if you have SATA drives, whose health (SMART) can only be read as administrator. Turn it off to never see the prompt (SATA health will then be unavailable). Launches at sign-in never ask.",
        ],
    );
    h(ui, "Monitoring");
    bullets(
        ui,
        &[
            "Sampling interval: how often sensors are read (250 ms to 2 s). Shorter gives more detail in graphs and recordings but uses a little more CPU.",
            "Show Windows notifications for alerts: turn off to keep alerts inside the app only (the ALERT button and history still show them).",
        ],
    );
    h(ui, "Files");
    p(
        ui,
        "Choose where recordings and reports are saved by pasting a folder path and pressing Apply. \
         Reset to Documents goes back to the default. Open folder shows the current folder in Explorer.",
    );
    details(
        ui,
        "settings",
        &[
            r"Settings are stored in %APPDATA%\OmniHW\settings.ini (plain text).",
            r"Launch at sign-in uses your user's startup entry: HKCU\Software\Microsoft\Windows\CurrentVersion\Run, value Omni-HW, with --autostart (and --minimized).",
            "Command-line options: --no-elevate skips the administrator prompt; --autostart and --minimized are used by the startup entry.",
            "Compare and Bottleneck look for saved recordings in the same folder.",
        ],
    );
}

fn privacy(ui: &mut Ui) {
    h(ui, "Everything stays on your PC");
    bullets(
        ui,
        &[
            "Omni-HW makes no network connections: no telemetry, no update checks, no uploads.",
            "It only reads hardware information and settings; it never changes your system settings. The one exception is the optional Launch at sign-in setting, which adds or removes Omni-HW's own startup entry.",
            "It installs no driver or service. The only files it writes are reports and recordings (Documents, or the folder chosen in Settings) and its own settings and alert rules in AppData.",
        ],
    );
    h(ui, "Administrator rights");
    p(
        ui,
        "Administrator rights are only used to read SATA drive health (SMART) and, on some systems, the NVMe health log. \
         Omni-HW asks for them at start only when a SATA-type drive is present, and works without them if you decline.",
    );
    h(ui, "Security-related advice");
    p(
        ui,
        "Some Health Check suggestions involve security features (marked with an orange Security setting badge). Omni-HW \
         explains the trade-off but never recommends turning protection off blindly.",
    );
}

fn troubleshooting(ui: &mut Ui) {
    h(ui, "A sensor or value is missing");
    bullets(
        ui,
        &[
            "GPU temperature, clocks, power, fan, PCIe link and Resizable BAR are read through NVIDIA's NVML, so they currently appear for NVIDIA cards only. AMD and Intel cards show load and VRAM use.",
            "CPU package power appears only when the CPU reports it to Windows (common on Intel; many AMD systems don't).",
            "The About tab lists any part Omni-HW couldn't read.",
        ],
    );
    h(ui, "Resizable BAR shows Unknown");
    p(
        ui,
        "Detection currently works on NVIDIA cards only. On AMD and Intel, check the BIOS (Above 4G Decoding and Re-Size BAR Support) or the vendor's control panel.",
    );
    h(ui, "Drive health is unavailable");
    bullets(
        ui,
        &[
            "SATA health needs administrator rights: accept the UAC prompt at start, or right-click Omni-HW > Run as administrator.",
            "Drives behind USB adapters, RAID arrays or Storage Spaces often don't pass health data through.",
        ],
    );
    h(ui, "Notifications don't appear");
    bullets(
        ui,
        &[
            "Turn off Do Not Disturb / Focus, and check Settings > System > Notifications.",
            "Use Alerts > Test notification to check.",
            "Notifications only fire while Omni-HW is running (it can be minimized).",
        ],
    );
    h(ui, "Numbers differ from other tools");
    p(
        ui,
        "Tools sample at different moments and some report different sensors (e.g. GPU 'Core Load' vs Task Manager's busiest \
         engine, or effective vs reported clocks). Small differences are normal.",
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_check_is_described() {
        let missing: Vec<_> = all_checks()
            .iter()
            .filter(|(name, _)| check_description(name).is_empty())
            .map(|(name, _)| *name)
            .collect();
        assert!(missing.is_empty(), "add help text for: {missing:?}");
    }
}
