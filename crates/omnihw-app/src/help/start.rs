//! Help pages: getting started, the Overview, settings, privacy and troubleshooting.

use super::{bullets, details, h, p, pct};
use crate::heat;
use egui::Ui;

pub(super) fn getting_started(ui: &mut Ui) {
    h(ui, "What is Omni-HW?");
    p(
        ui,
        "Omni-HW combines what CPU-Z, GPU-Z and HWiNFO do in one app: it identifies your hardware, shows live sensors, \
         and goes further by explaining what it finds (Health Check), alerting you, recording sessions and analyzing them.",
    );
    h(ui, "Finding your way around");
    bullets(
        ui,
        &[
            "Overview: a one-page dashboard of CPU, GPU, memory, storage and system, with live numbers and graphs.",
            "Health Check: automatic checks that flag common problems and explain how to fix them.",
            "Hardware: CPU, Caches, Mainboard, Memory, GPU, Storage, Power - detailed hardware information, like CPU-Z and GPU-Z.",
            "Network: adapters, connection details and tests, plus the LAN Security Report.",
            "Monitoring: Sensors (every live reading with min/max/average, like HWiNFO), Recording (record, compare, bottleneck analysis), Alerts (desktop notifications when a sensor crosses a limit) and Event Log (crashes, hardware errors and GPU hangs from the system journal, in plain English).",
            "Record (start/stop a recording from any page) and Report (save an HTML report or copy a text one) are always at the top right, next to the alerts badge while alerts fire.",
        ],
    );
    h(ui, "Navigation styles");
    bullets(
        ui,
        &[
            "Sidebar (the default): every page listed down the left side under its category. Feature guide (this guide), About and Settings are at the bottom of the sidebar; the bar above the page shows where you are.",
            "Tabs: category tabs along the top, with the open category's pages as a row of chips under them. Info (this guide and About) and Settings are on the right.",
            "Classic: every page as its own tab in one row (it may wrap onto two lines in small windows), with Info and Settings on the right.",
            "Switch between them in Settings > Navigation bar.",
        ],
    );
    h(ui, "Keyboard");
    bullets(
        ui,
        &[
            "Sidebar and Tabs: Left and Right move between the top-level entries (Overview, Health Check, Hardware, Network, \
             Monitoring, About, Settings), returning to the page you last used in a category. Up and Down move \
             through the pages inside the current category and stop at the first and last page.",
            "Classic: Left and Right move to the previous or next tab, wrapping around at the ends.",
            "The arrows are left alone while you're typing in a text field, adjusting a slider, moving between controls \
             with Tab, or a menu is open.",
        ],
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
        "Omni-HW reads the kernel's own interfaces (/proc, /sys, the system journal) and the graphics driver's \
         monitoring library (NVIDIA). It installs nothing and runs as a single program.",
    );
    h(ui, "Password prompt (Read as root)");
    p(
        ui,
        "A few things only root can read: the memory module details (SMBIOS), SATA drive health (SMART) and CPU \
         package power. Omni-HW always runs as you; when you ask, a short-lived helper reads them once through \
         pkexec, which shows your desktop's password prompt. If you cancel, everything else still works.",
    );
    details(
        ui,
        "start",
        &[
            "Turn on Settings > Root access to read them at every start, or click Read as root on a page that needs it.",
            "The About tab shows whether Omni-HW is running as root.",
        ],
    );
}

pub(super) fn overview(ui: &mut Ui) {
    h(ui, "What it shows");
    p(
        ui,
        "The first tab puts the most useful information from every other tab on one page: your CPU and graphics card \
         names, the Health Check summary, and a card each for the processor, graphics card(s), memory, storage and system.",
    );
    bullets(
        ui,
        &[
            "Processor: average and fastest core clock, total usage, package power (after Read as root), a usage graph, and one tile per physical core.",
            "Graphics card: load, temperature, power and core clock, graphs of load and temperature, a VRAM bar, Resizable BAR status, fan speed, memory clock and power state.",
            "Memory: total, type and speed, the installed modules, a used/total bar and a load graph.",
            "Storage: each drive with temperature, activity, read/write speed, a health bar (NVMe) and a free-space bar per mounted file system.",
            "System: distribution and kernel version, computer name, uptime, motherboard, BIOS, CPU socket and combined CPU + GPU power.",
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

pub(super) fn settings(ui: &mut Ui) {
    p(
        ui,
        "Settings change immediately and are saved automatically, so they survive restarts.",
    );
    h(ui, "Appearance");
    bullets(
        ui,
        &[
            "Theme: Match system (follows your desktop's light/dark mode), Dark, Light or Retro (2000s), a classic beveled desktop look with square buttons and sunken fields.",
            "UI scale: makes everything larger or smaller (75 % to 200 %). Reset returns to 100 %.",
            "Navigation bar: Sidebar (the default) lists every page down the left side, sorted into categories; Tabs shows category tabs with the open category's pages on a second row; Classic shows every page as its own tab (it may wrap onto two lines in small windows).",
        ],
    );
    h(ui, "Startup");
    bullets(
        ui,
        &[
            "Start page: the page Omni-HW opens on.",
            "Launch Omni-HW when I sign in: adds Omni-HW to your desktop's startup applications. Turn it off to remove it again.",
            "Start minimized: when launched at sign-in, start minimized instead of opening the window. Sensors, recordings and alerts keep running while it's minimized.",
            "Root access: read the memory module details and drive health at every start (your desktop asks for your password). Launches at sign-in never ask.",
        ],
    );
    h(ui, "Monitoring");
    bullets(
        ui,
        &[
            "Sampling interval: how often sensors are read (250 ms to 2 s). Shorter gives more detail in graphs and recordings but uses a little more CPU.",
            "Show desktop notifications for alerts: turn off to keep alerts inside the app only (the ALERT button and history still show them).",
        ],
    );
    h(ui, "Files");
    p(
        ui,
        "Choose where recordings and reports are saved by pasting a folder path and pressing Apply. \
         Reset to Documents goes back to the default. Open folder shows the current folder in your file manager.",
    );
    details(
        ui,
        "settings",
        &[
            "Settings are stored in ~/.config/omnihw/settings.ini (plain text; $XDG_CONFIG_HOME is used when set).",
            "Launch at sign-in writes an autostart entry, ~/.config/autostart/omnihw.desktop, that starts Omni-HW with --autostart (and --minimized).",
            "Command-line options: --autostart and --minimized are used by the autostart entry.",
            "Compare and Bottleneck look for saved recordings in the same folder.",
        ],
    );
}

pub(super) fn privacy(ui: &mut Ui) {
    h(ui, "Everything stays on your PC");
    bullets(
        ui,
        &[
            "Omni-HW makes no network connections on its own and works fully offline: no telemetry, no uploads.",
            "The update check only runs when you click Check for updates in Settings or turn on Check for updates when Omni-HW starts (off by default). It asks GitHub (through curl) for the latest release's version number and sends nothing about your PC.",
            "It only reads hardware information and settings and never changes system settings on its own. It changes a setting only when you ask: power settings on the Power page, and the optional Launch at sign-in setting, which adds or removes Omni-HW's own autostart entry.",
            "Health Check links (Google searches, driver downloads) only open in your browser when you click them; Omni-HW itself requests nothing.",
            "It reads the system journal locally (journalctl) to explain crashes, hardware errors and GPU hangs.",
            "It installs no driver or service. The only files it writes are reports and recordings (Documents, or the folder chosen in Settings) and its own settings and alert rules in ~/.config/omnihw.",
        ],
    );
    h(ui, "Root access");
    p(
        ui,
        "Omni-HW never runs as root. Root is used only to read the memory module details, SATA drive health (SMART) \
         and CPU package power, and to change the CPU settings on the Power page. Each time, pkexec asks for your \
         password and a short-lived copy of Omni-HW does just that one thing and exits. It asks at start only if you \
         turned that on in Settings, and works without root if you cancel.",
    );
    h(ui, "Security-related advice");
    p(
        ui,
        "Some Health Check suggestions involve security features (marked with an orange Security setting badge). Omni-HW \
         explains the trade-off but never recommends turning protection off blindly.",
    );
}

pub(super) fn troubleshooting(ui: &mut Ui) {
    h(ui, "A sensor or value is missing");
    bullets(
        ui,
        &[
            "NVIDIA cards show their sensors only with NVIDIA's driver; on nouveau they show basic information only.",
            "CPU package power, memory module details and SATA health need Read as root (Settings > Root access).",
            "Motherboard fans and voltages need the kernel driver for the board's sensor chip (often nct6775 or it87); run sudo sensors-detect from lm-sensors to find it.",
            "The About tab lists any part Omni-HW couldn't read.",
        ],
    );
    h(ui, "Resizable BAR shows Unknown");
    p(
        ui,
        "Omni-HW couldn't read the card's PCI memory window. Check the BIOS (Above 4G Decoding and Re-Size BAR Support), \
         or run lspci -vv as root and look for the card's Resizable BAR capability.",
    );
    h(ui, "Drive health is unavailable");
    bullets(
        ui,
        &[
            "SATA health needs root: click Read as root on the Storage page and enter your password.",
            "Drives behind USB adapters, RAID controllers or mdadm/LVM layers often don't pass health data through.",
        ],
    );
    h(ui, "Notifications don't appear");
    bullets(
        ui,
        &[
            "Turn off Do Not Disturb in your desktop's notification settings, and check notifications are allowed for Omni-HW.",
            "Use Alerts > Test notification to check.",
            "Notifications only fire while Omni-HW is running (it can be minimized).",
        ],
    );
    h(ui, "Numbers differ from other tools");
    p(
        ui,
        "Tools sample at different moments and some report different sensors (e.g. GPU 'Core Load' vs a tool's busiest engine, \
         or effective vs reported clocks). Small differences are normal.",
    );
}
