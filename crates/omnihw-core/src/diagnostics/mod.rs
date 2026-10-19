//! Health Check: rules over the collected static info and live sensors that flag common
//! problems and suggest fixes. Every check reports a finding, a pass, or why it was skipped,
//! so the user can see what was examined.
//!
//! The checks live in one submodule per area; [`CHECKS`] (plus the power checks) fixes the
//! order they run and are listed in.

mod cpu;
mod crashes;
mod gpu;
mod helpers;
mod memory;
mod network;
mod power;
mod settings;
mod storage;
mod system;
#[cfg(test)]
mod testutil;
mod types;

pub use memory::{channel_letter, rated_speed_from_part};
pub use system::parse_bios_date;
pub use types::{
    Category, CheckResult, Context, Finding, Level, Link, Outcome, RECENT, Step, query_encode,
};

/// A single check: examines the context and reports what it found.
type Check = fn(&Context) -> Outcome;

const CHECKS: &[(&str, Category, Check)] = &[
    (
        "CPU microcode (13th/14th Gen Intel stability)",
        Category::Cpu,
        cpu::intel_raptor_microcode,
    ),
    ("CPU clocks under load", Category::Cpu, cpu::cpu_throttling),
    ("CPU temperature", Category::Cpu, cpu::cpu_temperature),
    (
        "Memory speed (XMP / EXPO)",
        Category::Memory,
        memory::memory_speed,
    ),
    (
        "Memory channel layout",
        Category::Memory,
        memory::memory_channels,
    ),
    ("Memory pressure", Category::Memory, memory::memory_pressure),
    (
        "Installed memory",
        Category::Memory,
        memory::memory_capacity,
    ),
    (
        "Memory diagnostic results",
        Category::Memory,
        memory::memory_diagnostic,
    ),
    (
        "Monitor connected to the graphics card",
        Category::Gpu,
        gpu::monitor_connection,
    ),
    ("GPU PCIe link", Category::Gpu, gpu::gpu_pcie_link),
    ("GPU temperature", Category::Gpu, gpu::gpu_temperature),
    ("GPU memory (VRAM) pressure", Category::Gpu, gpu::gpu_vram),
    ("Monitor refresh rate", Category::Gpu, gpu::monitor_refresh),
    (
        "Graphics driver installed",
        Category::Gpu,
        gpu::gpu_driver_missing,
    ),
    (
        "GPU power and thermal slowdown",
        Category::Gpu,
        gpu::gpu_hw_slowdown,
    ),
    (
        "Leftover graphics drivers",
        Category::Gpu,
        gpu::leftover_gpu_drivers,
    ),
    // ReBAR and Windows settings.
    ("Resizable BAR", Category::Gaming, gpu::resizable_bar),
    ("GPU driver age", Category::Gpu, gpu::gpu_driver_age),
    (
        "Drive free space",
        Category::Storage,
        storage::drive_free_space,
    ),
    (
        "Drive health (NVMe SMART)",
        Category::Storage,
        storage::drive_health,
    ),
    (
        "Drive health (SATA SMART)",
        Category::Storage,
        storage::sata_smart,
    ),
    (
        "Drive temperature",
        Category::Storage,
        storage::drive_temperature,
    ),
    ("Drives set up", Category::Storage, storage::drive_setup),
    (
        "Windows drive type",
        Category::Storage,
        storage::system_drive_type,
    ),
    ("Drive link speed", Category::Storage, storage::storage_link),
    ("Battery wear", Category::System, system::battery_wear),
    ("Power plan", Category::Power, system::power_plan),
    ("BIOS age", Category::System, system::bios_age),
    ("Uptime", Category::System, system::uptime),
    (
        "Pending restart",
        Category::System,
        settings::pending_restart,
    ),
    (
        "Memory Integrity (VBS)",
        Category::Gaming,
        settings::memory_integrity,
    ),
    (
        "Hardware-accelerated GPU scheduling",
        Category::Gaming,
        settings::gpu_scheduling,
    ),
    ("Game Mode", Category::Gaming, settings::game_mode),
    ("Pagefile", Category::System, settings::pagefile),
    ("Device drivers", Category::System, system::device_problems),
    (
        "Hardware errors (WHEA)",
        Category::Crashes,
        crashes::whea_errors,
    ),
    (
        "Blue screens and unexpected restarts",
        Category::Crashes,
        crashes::crashes,
    ),
    (
        "Ethernet link speed",
        Category::Network,
        system::ethernet_speed,
    ),
    (
        "Network address (DHCP and DNS)",
        Category::Network,
        network::addresses,
    ),
    ("Wi-Fi signal", Category::Network, network::wifi_signal),
    (
        "Secure Boot, TPM and UEFI boot",
        Category::System,
        system::firmware_security,
    ),
    ("Windows activation", Category::System, system::activation),
    ("Startup apps", Category::System, system::startup_apps),
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

#[cfg(test)]
mod tests {
    use super::testutil::ctx;
    use super::*;
    use crate::SystemInfo;
    use crate::sensors::SensorTable;

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
