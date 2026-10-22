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
            network: None,
            events: None,
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
        "Kernel crashes and hangs" => {
            "Kernel panics, oopses, CPU lockups and tasks hung for minutes in the system journal over the last 30 days."
        }
        "Hardware errors (CPU, memory, PCIe)" => {
            "Machine checks, EDAC memory errors and PCIe errors the kernel logged in the last 30 days, usually from an unstable XMP/EXPO profile or overclock. The memory and disk tests also report any logged while they ran."
        }
        "GPU hangs and resets" => {
            "Times the amdgpu, i915/xe, nouveau or NVIDIA driver reset a GPU that stopped responding (including NVIDIA Xid errors) in the last 30 days."
        }
        "Disk and file system errors" => {
            "I/O errors, bad sectors, NVMe timeouts, SATA link resets and ext4/btrfs/XFS errors in the last 30 days."
        }
        "Out-of-memory kills" => {
            "Apps the kernel's OOM killer or systemd-oomd stopped because memory ran out."
        }
        "Overheating" => {
            "CPU thermal throttling or critical-temperature shutdowns the kernel logged."
        }
        "Pending restart" => {
            "Updates (often a new kernel) installed and waiting for a restart: /run/reboot-required on Debian and Ubuntu, /run/reboot-needed on openSUSE, and a replaced kernel elsewhere (its modules are gone from /usr/lib/modules)."
        }
        "Kernel problems since boot" => {
            "The kernel's taint flags show a machine check, bad memory page, oops, warning or soft lockup since this boot."
        }
        "Power profile" => {
            "power-profiles-daemon set to Power Saver while on mains power (a warning on desktops)."
        }
        "PCIe power saving" => {
            "The deepest PCIe power saving (ASPM powersupersave) on a desktop, which can cause stutter and wake problems."
        }
        "CPU frequency driver" => {
            "Ryzen 5000 and newer running on acpi-cpufreq instead of amd-pstate (a suggestion)."
        }
        "CPU governor" => {
            "The powersave governor on acpi-cpufreq (CPU stuck at its lowest clock), or the userspace governor."
        }
        "Swap space" => {
            "No swap or zram at all, so apps get killed when memory runs out (a warning with 16 GB of RAM or less)."
        }
        "NVIDIA driver" => {
            "An NVIDIA card running on nouveau or no driver instead of NVIDIA's driver."
        }
        "CPU microcode (13th/14th Gen Intel stability)" => {
            "13th/14th Gen Intel desktop CPUs need microcode 0x12B or newer (a BIOS or intel-microcode update) to avoid degrading over time."
        }
        "CPU clocks under load" => {
            "While the CPU is busy, its average clock should stay near base clock; lower means thermal or power throttling."
        }
        "CPU temperature" => {
            "CPU temperature (coretemp or k10temp) averaged over the last couple of minutes: warning from 88 °C, critical from 95 °C."
        }
        "Memory speed (XMP / EXPO)" => {
            "Compares the speed your RAM runs at with the speed its part number is rated for (XMP/EXPO off is common). Needs the memory details from Read as root."
        }
        "Memory channel layout" => {
            "Flags a single module or modules sharing one channel (half the bandwidth), and mixed modules."
        }
        "Memory pressure" => {
            "RAM nearly full over the last couple of minutes, which causes stutters or swapping."
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
        "GPU power and thermal slowdown" => {
            "NVIDIA cards that hit their hardware power brake or thermal slowdown under load (often a loose or daisy-chained power cable)."
        }
        "Resizable BAR" => {
            "Resizable BAR off on a card that supports it (NVIDIA through NVML; AMD and Intel from the card's PCI memory window)."
        }
        "Drive free space" => "Mounted file systems with less than 10 % free (critical below 5 %).",
        "Drive health (NVMe SMART)" => {
            "NVMe wear, critical warnings and media errors from the drive's own health log."
        }
        "Drive health (SATA SMART)" => {
            "Reallocated, pending or uncorrectable sectors, SSD life left and cable errors (needs Read as root)."
        }
        "Drive temperature" => "Drives averaging 70 °C or more (critical from 80 °C).",
        "Drives set up" => {
            "Drives with no partitions (not formatted or mounted yet), or large unallocated space."
        }
        "Drive link speed" => {
            "NVMe drives on fewer PCIe lanes or a lower generation than they support, and SATA drives below 6 Gb/s."
        }
        "Battery wear" => {
            "Laptop battery capacity lost compared with new (warning from 30 %, critical from 50 %)."
        }
        "BIOS age" => {
            "BIOS three or more years old; newer versions often fix stability and security issues."
        }
        "Ethernet link speed" => {
            "Wired network connected at 100 Mbps or less (a bad cable or port), or below the adapter's maximum (e.g. 1 Gbps on a 2.5 Gbps adapter, a suggestion)."
        }
        "Network address (DHCP and DNS)" => {
            "A connected adapter with only a self-assigned 169.254.x.x address (no DHCP), an IP address conflict, or no DNS servers."
        }
        "Wi-Fi signal" => "Wi-Fi signal below 40 %, with tips for 2.4 GHz connections.",
        "Secure Boot, TPM and UEFI boot" => {
            "Legacy (CSM) boot, Secure Boot off or no TPM 2.0, as notes: Linux runs fine without them, but they protect the boot chain."
        }
        _ => "",
    }
}

pub(super) fn health_check(ui: &mut Ui) {
    h(ui, "What it does");
    p(
        ui,
        "The Health Check runs a set of rules over your hardware information and recent sensor readings, and reports \
         anything that looks wrong or could be better, in plain language with steps to fix it. Nothing is changed: \
         you decide what to do.",
    );
    h(ui, "Reading the results");
    bullets(
        ui,
        &[
            "Critical (red): fix soon, e.g. a failing drive or outdated CPU microcode with a known stability problem.",
            "Warning (orange): something is holding your PC back, e.g. memory not running at its rated speed.",
            "Suggestion (blue, shown as Info): optional improvements.",
            "The tiles at the top show each category (Crashes, System, CPU, Memory, GPU, Storage, Network and Gaming): OK, not checked, or how many findings it has in the colour of the most serious one. Click a tile to list only that category, and click it again (or All) to show everything.",
            "The list on the left groups findings into Fix these (critical issues and warnings), Suggestions and For gaming, most serious first. Click one to see its details, steps and buttons on the right, or use the Up and Down arrow keys to move through the list.",
            "Passed checks lists each category's checks that found nothing: OK rows found nothing wrong (with what was measured) and dash rows don't apply to this PC (with the reason, e.g. no battery).",
        ],
    );
    h(ui, "Gaming section");
    p(
        ui,
        "Settings that mainly matter for games (such as Resizable BAR) are grouped in the Gaming category and listed under For gaming, separate from the main summary. Turn off Show \
         gaming suggestions in Settings > Health Check to hide the Gaming category and its count everywhere.",
    );
    h(ui, "Dismissing issues");
    bullets(
        ui,
        &[
            "Dismiss at the bottom of a finding's details hides it from the list, the summary and the Overview banner. It moves to Dismissed (n) at the bottom of the list, where you can read it again; Restore brings it back, and Restore all brings them all back.",
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
            "Search Google opens a search in your browser built from your hardware, e.g. your motherboard model plus \"enable EXPO\". Open website goes to the official page. Nothing is opened unless you click.",
            "BIOS steps name the key that opens the setup and the menu path your board maker uses (ASUS, MSI, Gigabyte, ASRock, Dell, HP, Lenovo), read from the motherboard's SMBIOS data.",
            "On UEFI PCs, BIOS-related findings have a Restart into UEFI / BIOS button: after you confirm, your desktop may ask for your password and the PC restarts straight into the setup, so there's no key to catch. Save your work first.",
        ],
    );
    h(ui, "Using it");
    bullets(
        ui,
        &[
            "The most serious finding is selected when the page opens. The list and the details scroll separately; in a narrow window the details move below the list.",
            "Hover the totals line under the tiles to see how the live checks work.",
            "Live checks (memory pressure, temperatures, throttling) use roughly the last two minutes of readings and need at least a few seconds of data first.",
            "The throttling check needs some time with the CPU under heavy load (a game, a render) before it gives a verdict.",
            "Results also appear at the top of saved and copied reports.",
            "The crash checks read the system journal once when Omni-HW starts; Refresh on the Event Log page reads it again. If you aren't in the systemd-journal (or adm) group they only see your own messages.",
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
