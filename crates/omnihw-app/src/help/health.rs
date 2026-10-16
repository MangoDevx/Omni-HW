//! Health Check help, including a description of every rule.

use super::{bullets, h, p};
use egui::{RichText, Ui};
use omnihw_core::diagnostics::{self, Category};
use omnihw_core::sensors::SensorTable;
use std::sync::OnceLock;

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
            power: None,
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
        "USB selective suspend" => {
            "On while USB audio interfaces, capture cards, VR headsets or game controllers are connected (drop-outs, crackling)."
        }
        "Network adapter power saving" => {
            "Windows allowed to turn off a network adapter, Energy-Efficient Ethernet on, or high Wi-Fi power saving while plugged in."
        }
        "Bluetooth power saving" => {
            "Windows allowed to turn off the Bluetooth adapter, a common cause of controller and headset disconnects."
        }
        "PCI Express power saving" => {
            "Link State Power Management at maximum savings on a desktop (GPU/NVMe stutter, wake problems)."
        }
        "Sleep timer" => {
            "Sleep after less than 10 minutes when plugged in, or sleep set sooner than the display timeout."
        }
        "Waking from sleep" => {
            "The PC woke by itself (wake timers or a network adapter) in the last 2 weeks; shows the wake source."
        }
        "Fast Startup" => {
            "Fast Startup on with Windows running for a week or more: shutdowns don't start Windows fresh."
        }
        "Hard disk turn-off timer" => {
            "Hard disks set to turn off after less than 10 minutes while plugged in (spin-up freezes)."
        }
        "Processor state limits" => {
            "Maximum processor state below 100 % when plugged in, or minimum at 100 % on battery."
        }
        "Apps keeping the PC awake" => {
            "Apps or drivers with active power requests that stop sleep (needs administrator; use Check now on the Power page)."
        }
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
        "Memory diagnostic results" => {
            "Errors reported by a Windows Memory Diagnostic run in the last 90 days."
        }
        "Monitor refresh rate" => {
            "A monitor set below the best refresh rate Windows offers for it (e.g. a 144 Hz monitor left at 60 Hz)."
        }
        "Graphics driver installed" => {
            "A graphics card running on Microsoft's Basic Display driver instead of the NVIDIA/AMD/Intel driver."
        }
        "GPU power and thermal slowdown" => {
            "NVIDIA cards that hit their hardware power brake or thermal slowdown under load (often a loose or daisy-chained power cable)."
        }
        "Leftover graphics drivers" => {
            "Driver packages for a graphics card brand that is no longer installed."
        }
        "Drives set up" => {
            "New drives that were never initialized (they don't show in File Explorer), or large unallocated space."
        }
        "Windows drive type" => {
            "Windows installed on a hard drive, especially when an SSD is present."
        }
        "Drive link speed" => {
            "NVMe drives on fewer PCIe lanes or a lower generation than they support, and SATA drives below 6 Gb/s (needs administrator)."
        }
        "Device drivers" => {
            "Devices with a problem in Device Manager, usually missing chipset, network or audio drivers."
        }
        "Hardware errors (WHEA)" => {
            "CPU, memory and PCIe hardware errors logged in the last 30 days, usually from an unstable XMP/EXPO profile or overclock."
        }
        "Blue screens and unexpected restarts" => {
            "Blue screens (with their stop codes) and restarts without a clean shutdown in the last 30 days."
        }
        "Ethernet link speed" => {
            "Wired network connected at 100 Mbps or less, which points to a bad cable or port."
        }
        "Secure Boot, TPM and UEFI boot" => {
            "Legacy (CSM) boot, Secure Boot off or no TPM 2.0: needed for Windows 11 and some games' anti-cheat."
        }
        "Windows activation" => "Windows is not activated.",
        "Startup apps" => "More than 15 apps starting with Windows.",
        _ => "",
    }
}

pub(super) fn health_check(ui: &mut Ui) {
    h(ui, "What it does");
    p(
        ui,
        "The Health Check runs a set of rules over your hardware information and recent sensor readings, and reports \
         anything that looks wrong or could be better, in plain language with steps to fix it. Nothing is changed \
         automatically: you decide what to do, and only a Fix button you click changes a setting.",
    );
    h(ui, "Reading the results");
    bullets(
        ui,
        &[
            "Critical (red): fix soon, e.g. a failing drive or outdated CPU microcode with a known stability problem.",
            "Warning (orange): something is holding your PC back, e.g. memory not running at its rated speed.",
            "Suggestion (blue, shown as INFO on cards): optional improvements.",
            "Results are grouped by category: Crashes (BSOD), System, CPU, Memory, GPU, Storage, Power Management and Gaming. A category opens by default only when it has something to fix; its pill shows the count in the colour of the most serious one.",
            "Inside each category, OK rows are checks that found nothing wrong (with what was measured) and dash rows are checks that don't apply to this PC (with the reason, e.g. no battery).",
        ],
    );
    h(ui, "Gaming section");
    p(
        ui,
        "Settings that mainly matter for games (Resizable BAR, Memory Integrity, hardware GPU scheduling, Game Mode) are \
         grouped in the Gaming category. Its findings are counted separately at the top (\"gaming suggestions below\") \
         rather than in the main summary. Turn off Show gaming suggestions in Settings > Health Check to hide the \n         Gaming category and its count everywhere.",
    );
    h(ui, "Dismissing issues");
    bullets(
        ui,
        &[
            "Dismiss on a card hides it from its category, the summary and the Overview banner. It moves to Dismissed at the bottom of the page; the Dismissed (n) button at the top jumps there. Open a dismissed card to read it again, and Restore brings it back.",
            "Expand all opens every category, Expand issues only opens just the categories with something to fix, and Collapse all closes them.",
            "If the same check later finds something more serious (e.g. a suggestion becomes a warning), it shows up again.",
            "By default dismissed issues stay dismissed after restarting Omni-HW. Turn that off in Settings > Health Check to have them come back on each start; Restore dismissed issues there brings them all back.",
        ],
    );
    h(ui, "Security settings");
    p(
        ui,
        "Findings about a security setting carry an orange \"Security setting\" badge and a caution note. They explain \
         the trade-off; only change such a setting if you know what you are doing and understand the protection you \
         would give up.",
    );
    h(ui, "Fixing what it finds");
    bullets(
        ui,
        &[
            "Each finding has numbered steps. Tick a step's box as you finish it; the header shows how many are done (ticks reset when Omni-HW closes).",
            "Search Google opens a search in your browser built from your hardware, e.g. your motherboard model plus \"enable EXPO\". Open Settings opens the right Windows Settings page, and Open website goes to the official download page. Nothing is opened unless you click.",
            "BIOS steps name the key that opens the setup and the menu path your board maker uses (ASUS, MSI, Gigabyte, ASRock, Dell, HP, Lenovo), read from the motherboard's SMBIOS data.",
            "On UEFI PCs, BIOS-related findings have a Restart into UEFI / BIOS button: after you confirm, Windows asks for administrator rights and restarts straight into the setup, so there's no key to catch. Save your work first.",
        ],
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
            "Some power findings have a Fix button that changes the setting for you. Changes that affect networking or shutdown ask first, and ones that need administrator rights show the Windows permission prompt.",
        ],
    );
    h(ui, "All checks");
    for cat in Category::ALL {
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
