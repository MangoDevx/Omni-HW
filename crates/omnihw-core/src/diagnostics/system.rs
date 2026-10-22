//! System checks: battery, power plan, BIOS age, uptime, Device Manager problems, Ethernet
//! speed, firmware security, activation and startup apps.

use super::helpers::{
    MBR2GPT_URL, bios_update_step, board_drivers_step, finding, guided, months_since, plural,
};
use super::{Category, Context, Level, Outcome, Step};
use crate::bios_help::{self, BiosSetting};
use crate::checkup::{Activation, Tpm, format_link_speed, problem_is_missing_driver, problem_text};
use crate::network::Media;
use crate::platform::PlanKind;
use crate::power::{Action, BALANCED};

/// Device Manager problems listed by name before "and N more".
const DEVICES_LISTED: usize = 8;

pub(super) fn battery_wear(ctx: &Context) -> Outcome {
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

pub(super) fn power_plan(ctx: &Context) -> Outcome {
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
    .with_fix(Action::SetScheme(BALANCED.into()))])
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

pub(super) fn bios_age(ctx: &Context) -> Outcome {
    let Some(b) = ctx.info.smbios.as_ref().and_then(|s| s.bios.as_ref()) else {
        return Outcome::Skipped("BIOS information not available".into());
    };
    let Some((y, m, _)) = parse_bios_date(&b.release_date) else {
        return Outcome::Skipped(format!("Unrecognized BIOS date \"{}\"", b.release_date));
    };
    let months = months_since(ctx, y, m);
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

pub(super) fn uptime(ctx: &Context) -> Outcome {
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

pub(super) fn device_problems(ctx: &Context) -> Outcome {
    let c = &ctx.info.checkup;
    if c.devices.is_empty() {
        return Outcome::Skipped("Device list not available".into());
    }
    // Display adapters without a driver are reported by the GPU driver check.
    let bad: Vec<_> = c
        .devices
        .iter()
        .filter(|d| !(d.is_display() && d.pci_vendor().is_some()))
        .filter_map(|d| Some((d, d.problem?)))
        .collect();
    if bad.is_empty() {
        return Outcome::Pass("No devices with problems in Device Manager".into());
    }
    let list: Vec<String> = bad
        .iter()
        .take(DEVICES_LISTED)
        .map(|(d, p)| format!("{}: {} (code {p})", d.name, problem_text(*p)))
        .collect();
    let more = if bad.len() > DEVICES_LISTED {
        format!(" and {} more", bad.len() - DEVICES_LISTED)
    } else {
        String::new()
    };
    let mut steps = Vec::new();
    if bad.iter().any(|&(_, p)| problem_is_missing_driver(p)) {
        steps.push(board_drivers_step(
            ctx,
            "Install the drivers from your motherboard or PC maker's support page: the chipset driver first, then LAN, Wi-Fi, Bluetooth and audio.",
        ));
        steps.push(Step::settings(
            "Also check Windows Update > Advanced options > Optional updates > Driver updates.",
            "ms-settings:windowsupdate-optionalupdates",
        ));
    }
    if bad.iter().any(|&(_, p)| matches!(p, 10 | 43)) {
        steps.push(Step::new(
            "For \"cannot start\" (code 10) or \"stopped\" (code 43) on USB devices: unplug and replug, try another port, and update the chipset driver.",
        ));
    }
    steps.push(Step::new(
        "Open Device Manager (right-click Start > Device Manager): devices with a yellow \"!\" are the ones listed here. Right-click > Update driver, or Uninstall device and restart to let Windows retry.",
    ));
    Outcome::Findings(vec![guided(
        Level::Warning,
        Category::System,
        format!(
            "{} not working properly",
            plural(bad.len(), "device is", "devices are")
        ),
        format!(
            "Device Manager reports: {}{more}. These are usually missing chipset, network or audio drivers on a fresh Windows install.",
            list.join("; ")
        ),
        steps,
    )])
}

pub(super) fn ethernet_speed(ctx: &Context) -> Outcome {
    // (name, link speed) of each wired adapter: from the live adapter list when it has been read,
    // otherwise from the start-up check-up.
    let wired: Vec<(&str, u64)> = match ctx.network {
        Some(n) => n
            .connected()
            .filter(|a| a.media == Media::Ethernet && a.physical)
            .filter_map(|a| Some((a.name.as_str(), a.speed_bps?)))
            .collect(),
        None => ctx
            .info
            .checkup
            .nics
            .iter()
            .map(|n| (n.name.as_str(), n.speed_bps))
            .collect(),
    };
    let has_wired = match ctx.network {
        Some(n) => n
            .connected()
            .any(|a| a.media == Media::Ethernet && a.physical),
        None => !wired.is_empty(),
    };
    if !has_wired {
        return Outcome::Skipped("No wired network connection".into());
    }
    let mut out = Vec::new();
    // Gigabit or faster, but below what the adapter supports (e.g. 1 Gbps on a 2.5 Gbps port).
    for a in ctx
        .network
        .iter()
        .flat_map(|n| n.connected())
        .filter(|a| a.media == Media::Ethernet && a.physical)
    {
        let Some((cur, max)) = a.below_max_speed().filter(|(c, _)| *c > 100_000_000) else {
            continue;
        };
        let (cur, max) = (format_link_speed(cur), format_link_speed(max));
        out.push(guided(
            Level::Info,
            Category::Network,
            format!("Ethernet linked at {cur}, below its {max} maximum"),
            format!(
                "{} supports {max} but connected at {cur}. That's expected if the router or switch port only does {cur}; if it supports {max}, the cable or a port is limiting it.",
                a.name
            ),
            vec![
                Step::new(format!("Check whether your router or switch has a {max} port, and plug into that one.")),
                Step::new("Use a Cat5e or Cat6 cable (Cat6 for longer runs) and make sure both plugs click in."),
                Step::new("On the Network page, check the adapter's Speed & Duplex setting is Auto Negotiation."),
            ],
        ));
    }
    for &(name, speed_bps) in &wired {
        if speed_bps > 100_000_000 {
            continue;
        }
        let speed = format_link_speed(speed_bps);
        out.push(guided(
            Level::Warning,
            Category::Network,
            format!("Ethernet connected at only {speed}"),
            format!(
                "{name} linked at {speed}. Gigabit (1 Gbps) is standard on PCs and routers; 100 Mbps almost always means a damaged cable, a cable with only four wires, or a bad port, and caps downloads at about 12 MB/s."
            ),
            vec![
                Step::new("Replace the network cable with a Cat5e or Cat6 one, and make sure both plugs click in."),
                Step::new("Try a different port on the router or switch, and check that it supports gigabit."),
                Step::search(
                    "Update the network adapter's driver.",
                    format!("{name} driver download"),
                ),
            ],
        ));
    }
    if out.is_empty() {
        return Outcome::Pass(
            wired
                .iter()
                .map(|(name, bps)| format!("{name}: {}", format_link_speed(*bps)))
                .collect::<Vec<_>>()
                .join("; "),
        );
    }
    Outcome::Findings(out)
}

pub(super) fn firmware_security(ctx: &Context) -> Outcome {
    let fw = &ctx.info.checkup.firmware;
    if fw.uefi.is_none() && fw.secure_boot.is_none() && fw.tpm == Tpm::Unknown {
        return Outcome::Skipped("Firmware information not available".into());
    }
    let why = "Windows 11 and the anti-cheat in games such as Valorant, Battlefield 6 and FACEIT need it.";
    let mut out = Vec::new();
    if fw.uefi == Some(false) {
        let mut steps = vec![Step::url(
            "Convert the Windows disk from MBR to GPT without losing data: in an administrator terminal run \"mbr2gpt /validate /allowFullOS\", then \"mbr2gpt /convert /allowFullOS\". Back up first.",
            MBR2GPT_URL,
        )];
        steps.extend(bios_help::bios_steps(
            ctx.info,
            BiosSetting::Csm,
            "Turn off CSM / legacy boot so the PC boots in UEFI mode.",
            "",
        ));
        out.push(
            guided(
                Level::Warning,
                Category::System,
                "Windows boots in legacy BIOS (CSM) mode",
                format!(
                    "The PC starts Windows through the old BIOS compatibility mode, which rules out Secure Boot and Resizable BAR. {why}"
                ),
                steps,
            )
            .security(),
        );
    } else if fw.secure_boot == Some(false) {
        out.push(
            guided(
                Level::Warning,
                Category::System,
                "Secure Boot is off",
                format!("Secure Boot checks that only trusted code runs while the PC starts. {why}"),
                bios_help::bios_steps(
                    ctx.info,
                    BiosSetting::SecureBoot,
                    "Turn on Secure Boot (some boards first need the default keys installed, or CSM turned off).",
                    "",
                ),
            )
            .security()
            .bios(),
        );
    }
    match fw.tpm {
        Tpm::Missing => out.push(
            guided(
                Level::Warning,
                Category::System,
                "No TPM found",
                format!(
                    "The PC has no active TPM (Trusted Platform Module), used for BitLocker, Windows Hello and secure sign-in. {why} Most CPUs since 2017 have a firmware TPM that's simply turned off."
                ),
                bios_help::bios_steps(
                    ctx.info,
                    BiosSetting::Tpm,
                    "Turn on the firmware TPM (AMD fTPM or Intel PTT).",
                    "",
                ),
            )
            .security()
            .bios(),
        ),
        Tpm::V12 => out.push(guided(
            Level::Info,
            Category::System,
            "Only TPM 1.2 is available",
            "Windows 11 needs TPM 2.0. Many boards can switch the firmware TPM to 2.0, or a discrete TPM 2.0 module can be added.",
            vec![Step::new(
                "Check the BIOS security settings for a TPM 2.0 / firmware TPM option.",
            )],
        )
        .security()
        .bios()),
        _ => {}
    }
    if out.is_empty() {
        let mut parts = Vec::new();
        if fw.uefi == Some(true) {
            parts.push("UEFI boot");
        }
        if fw.secure_boot == Some(true) {
            parts.push("Secure Boot on");
        }
        if fw.tpm == Tpm::V20 {
            parts.push("TPM 2.0");
        }
        return Outcome::Pass(parts.join(", "));
    }
    Outcome::Findings(out)
}

pub(super) fn activation(ctx: &Context) -> Outcome {
    match ctx.info.checkup.activation {
        None => Outcome::Skipped("Activation status not available".into()),
        Some(Activation::Activated) => Outcome::Pass("Windows is activated".into()),
        Some(state) => Outcome::Findings(vec![guided(
            Level::Warning,
            Category::System,
            if state == Activation::Tampered {
                "Windows license isn't genuine"
            } else {
                "Windows isn't activated"
            },
            "An unactivated Windows shows a watermark, locks personalization settings and keeps reminding you.",
            vec![
                Step::settings(
                    "Open Settings > System > Activation and enter your product key, or buy a license there.",
                    "ms-settings:activation",
                ),
                Step::new(
                    "If Windows was activated before you changed the motherboard, run the Troubleshoot option there and choose \"I changed hardware on this device recently\" (sign in with the Microsoft account the license is linked to).",
                ),
            ],
        )]),
    }
}

/// More enabled startup apps than this is flagged.
const STARTUP_LIMIT: usize = 15;

pub(super) fn startup_apps(ctx: &Context) -> Outcome {
    let Some(apps) = &ctx.info.checkup.startup_apps else {
        return Outcome::Skipped("Startup apps not readable".into());
    };
    let n = apps.len();
    if n <= STARTUP_LIMIT {
        return Outcome::Pass(plural(n, "startup app", "startup apps"));
    }
    let shown: Vec<&str> = apps.iter().take(12).map(String::as_str).collect();
    Outcome::Findings(vec![guided(
        Level::Info,
        Category::System,
        format!("{n} apps start with Windows"),
        format!(
            "{n} apps launch at sign-in ({}{}). Each adds to startup time and keeps running in the background.",
            shown.join(", "),
            if n > shown.len() { ", ..." } else { "" }
        ),
        vec![
            Step::settings(
                "Open Settings > Apps > Startup and turn off the apps you don't need right away (launchers, updaters, chat apps).",
                "ms-settings:startupapps",
            ),
            Step::new(
                "Task Manager's Startup apps page shows the same list with each app's startup impact.",
            ),
        ],
    )])
}

#[cfg(test)]
mod tests {
    use super::super::gpu::{gpu_driver_missing, leftover_gpu_drivers};
    use super::super::storage::storage_link;
    use super::super::testutil::*;
    use super::*;
    use crate::SystemInfo;
    use crate::checkup::{Device, NicLink, PcieLink};
    use crate::gpu::VENDOR_NVIDIA;
    use crate::platform::PowerPlan;
    use crate::sensors::SensorTable;
    use crate::storage::{Drive, SataLink};

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
    fn drivers_and_devices() {
        let s = SensorTable::default();
        let dev = |name: &str, class: &str, service: &str, hw: &str, problem| Device {
            name: name.into(),
            class: class.into(),
            service: service.into(),
            hardware_id: hw.into(),
            problem,
        };
        let mut info = SystemInfo::default();
        info.checkup.devices = vec![
            dev(
                "Microsoft Basic Display Adapter",
                "Display",
                "BasicDisplay",
                r"PCI\VEN_10DE&DEV_2204",
                None,
            ),
            dev(
                "Microsoft Hyper-V Video",
                "Display",
                "BasicDisplay",
                r"VMBUS\{x}",
                None,
            ),
        ];
        let o = gpu_driver_missing(&ctx(&info, &s));
        assert_eq!(levels(&o), vec![Level::Critical]);
        let Outcome::Findings(f) = o else {
            unreachable!()
        };
        assert!(f[0].title.contains("NVIDIA"));
        assert!(matches!(device_problems(&ctx(&info, &s)), Outcome::Pass(_)));

        info.checkup.devices.push(dev(
            "Ethernet Controller",
            "Other",
            "",
            r"PCI\VEN_8086&DEV_15F3",
            Some(28),
        ));
        let o = device_problems(&ctx(&info, &s));
        assert_eq!(levels(&o), vec![Level::Warning]);
        let Outcome::Findings(f) = o else {
            unreachable!()
        };
        assert!(
            f[0].detail
                .contains("Ethernet Controller: no driver installed (code 28)")
        );
        assert!(matches!(
            f[0].steps[0].link,
            Some(super::super::Link::Search(_))
        ));
    }

    #[test]
    fn links_and_network() {
        let s = SensorTable::default();
        let mut info = SystemInfo::default();
        assert!(matches!(
            ethernet_speed(&ctx(&info, &s)),
            Outcome::Skipped(_)
        ));
        assert!(matches!(storage_link(&ctx(&info, &s)), Outcome::Skipped(_)));
        info.checkup.nics = vec![NicLink {
            name: "Realtek PCIe 2.5GbE".into(),
            speed_bps: 100_000_000,
        }];
        assert_eq!(
            levels(&ethernet_speed(&ctx(&info, &s))),
            vec![Level::Warning]
        );
        info.checkup.nics[0].speed_bps = 2_500_000_000;
        assert!(matches!(ethernet_speed(&ctx(&info, &s)), Outcome::Pass(_)));
        // 1 Gbps on a 2.5 Gbps adapter is only a suggestion.
        let net = crate::network::NetworkInfo {
            adapters: vec![crate::network::Adapter {
                media: crate::network::Media::Ethernet,
                physical: true,
                status: crate::network::Status::Up,
                speed_bps: Some(1_000_000_000),
                max_speed_bps: Some(2_500_000_000),
                ..Default::default()
            }],
            detailed: true,
        };
        let mut c = ctx(&info, &s);
        c.network = Some(&net);
        assert_eq!(levels(&ethernet_speed(&c)), vec![Level::Info]);

        // 100 Mbps on a physical adapter, from the live adapter list: a Network warning.
        let eth = |physical, speed| crate::network::NetworkInfo {
            adapters: vec![crate::network::Adapter {
                name: "Ethernet".into(),
                media: crate::network::Media::Ethernet,
                physical,
                status: crate::network::Status::Up,
                speed_bps: Some(speed),
                ..Default::default()
            }],
            detailed: true,
        };
        let slow = eth(true, 100_000_000);
        let mut c = ctx(&info, &s);
        c.network = Some(&slow);
        let Outcome::Findings(f) = ethernet_speed(&c) else {
            panic!("expected a finding")
        };
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].category, Category::Network);
        // A virtual adapter is not a wired connection at all.
        let virt = eth(false, 100_000_000);
        c.network = Some(&virt);
        assert!(matches!(ethernet_speed(&c), Outcome::Skipped(_)));
        // A healthy physical adapter passes, ignoring the start-up list.
        let fast = eth(true, 1_000_000_000);
        c.network = Some(&fast);
        assert!(matches!(ethernet_speed(&c), Outcome::Pass(_)));

        info.checkup.nvme_links = vec![
            PcieLink {
                device: "Fast SSD".into(),
                current: (4, 2),
                max: (4, 4),
            },
            PcieLink {
                device: "Other SSD".into(),
                current: (3, 4),
                max: (4, 4),
            },
        ];
        info.drives.push(Drive {
            model: "SATA SSD".into(),
            sata_link: Some(SataLink {
                current_gen: 2,
                max_gen: 3,
            }),
            ..Default::default()
        });
        assert_eq!(
            levels(&storage_link(&ctx(&info, &s))),
            vec![Level::Warning, Level::Info, Level::Warning]
        );
    }

    #[test]
    fn firmware_and_software() {
        use crate::checkup::Firmware;
        let s = SensorTable::default();
        let mut info = SystemInfo::default();
        assert!(matches!(
            firmware_security(&ctx(&info, &s)),
            Outcome::Skipped(_)
        ));
        info.checkup.firmware = Firmware {
            uefi: Some(true),
            secure_boot: Some(false),
            tpm: Tpm::Missing,
        };
        let o = firmware_security(&ctx(&info, &s));
        assert_eq!(levels(&o), vec![Level::Warning, Level::Warning]);
        let Outcome::Findings(f) = o else {
            unreachable!()
        };
        assert!(f.iter().all(|f| f.security && f.bios));
        info.checkup.firmware = Firmware {
            uefi: Some(false),
            secure_boot: Some(false),
            tpm: Tpm::V20,
        };
        let o = firmware_security(&ctx(&info, &s));
        assert_eq!(levels(&o), vec![Level::Warning]);
        info.checkup.firmware.uefi = Some(true);
        info.checkup.firmware.secure_boot = Some(true);
        assert!(matches!(
            firmware_security(&ctx(&info, &s)),
            Outcome::Pass(_)
        ));

        assert!(matches!(activation(&ctx(&info, &s)), Outcome::Skipped(_)));
        info.checkup.activation = Some(Activation::NotActivated);
        assert_eq!(levels(&activation(&ctx(&info, &s))), vec![Level::Warning]);

        info.checkup.gpu_driver_packages = vec![
            "AMD Software".into(),
            "NVIDIA Graphics Driver 581.57".into(),
        ];
        info.gpus = vec![crate::gpu::GpuAdapter {
            vendor_id: VENDOR_NVIDIA,
            ..Default::default()
        }];
        let o = leftover_gpu_drivers(&ctx(&info, &s));
        assert_eq!(levels(&o), vec![Level::Info]);
        let Outcome::Findings(f) = o else {
            unreachable!()
        };
        assert!(f[0].title.contains("AMD") && !f[0].detail.contains("NVIDIA"));

        info.checkup.startup_apps = Some((0..20).map(|i| format!("App{i}")).collect());
        assert_eq!(levels(&startup_apps(&ctx(&info, &s))), vec![Level::Info]);
        info.checkup.startup_apps = Some(vec!["OneDrive".into()]);
        assert!(matches!(startup_apps(&ctx(&info, &s)), Outcome::Pass(_)));
    }
}
