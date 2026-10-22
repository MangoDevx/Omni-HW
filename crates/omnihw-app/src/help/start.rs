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
            "Monitoring: Sensors (every live reading with min/max/average, like HWiNFO), Recording (record, compare, bottleneck analysis), Alerts (Windows notifications when a sensor crosses a limit) and Event Log (crashes, hardware errors and driver timeouts Windows logged, in plain English).",
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

pub(super) fn settings(ui: &mut Ui) {
    p(
        ui,
        "Settings change immediately and are saved automatically, so they survive restarts.",
    );
    h(ui, "Appearance");
    bullets(
        ui,
        &[
            "Theme: Match Windows (follows your light/dark mode), Dark, Light or Retro (2000s), a classic beveled desktop look with square buttons and sunken fields.",
            "UI scale: makes everything larger or smaller (75 % to 200 %). Reset returns to 100 %.",
            "Navigation bar: Sidebar (the default) lists every page down the left side, sorted into categories; Tabs shows category tabs with the open category's pages on a second row; Classic shows every page as its own tab (it may wrap onto two lines in small windows).",
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

pub(super) fn privacy(ui: &mut Ui) {
    h(ui, "Everything stays on your PC");
    bullets(
        ui,
        &[
            "Omni-HW makes no network connections on its own and works fully offline: no telemetry, no uploads.",
            "The update check only runs when you click Check for updates in Settings or turn on Check for updates when Omni-HW starts (off by default). It asks GitHub for the latest release's version number and sends nothing about your PC.",
            "The internet tests on the Network page only run when you click Run internet tests. They look up dns.msftncsi.com, request www.msftconnecttest.com/connecttest.txt (Windows' own connectivity probes) and ping the public DNS resolvers 1.1.1.1 and 8.8.8.8. The local tests only contact your own router and the DNS servers on your local network.",
            "It only reads hardware information and settings and never changes system settings on its own. It changes a setting only when you ask: power settings on the Power page or a Health Check Fix button, and the optional Launch at sign-in setting, which adds or removes Omni-HW's own startup entry.",
            "Health Check links (Google searches, Settings pages, driver downloads) only open in your browser or Windows when you click them; Omni-HW itself requests nothing.",
            "It reads the System event log (hardware errors, blue screens) and the Device Manager state locally to explain crashes and missing drivers.",
            "It installs no driver or service. The only files it writes are reports and recordings (Documents, or the folder chosen in Settings) and its own settings and alert rules in AppData.",
        ],
    );
    h(ui, "Administrator rights");
    p(
        ui,
        "Administrator rights are used to read SATA drive health (SMART) and link speed and, on some systems, the NVMe health log. Omni-HW asks for them at start only when a SATA-type drive is present, and works without them if you decline. The Restart into UEFI / BIOS button and power changes that need them (Fast Startup, adapter power saving, wake devices) ask separately, only when you click them.",
    );
    p(
        ui,
        "Some power changes (Fast Startup, adapter power saving, wake devices), network changes on the Network page and reading power requests also need them. \
         Windows asks each time, and a short-lived copy of Omni-HW makes just that one change and exits.",
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
