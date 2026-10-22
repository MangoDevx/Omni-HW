//! System checks: battery, BIOS age, Ethernet speed and firmware security.

use super::helpers::{bios_update_step, finding, guided, months_since};
use super::{Category, Context, Level, Outcome, Step};
use crate::bios_help::{self, BiosSetting};
use crate::checkup::{Tpm, format_link_speed};
use crate::network::Media;

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
                Step::new("Check that the link isn't forced to a fixed speed: \"ethtool <interface>\" should show auto-negotiation on."),
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
                Step::new("Install your distribution's updates: network drivers come with the kernel."),
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
    let mut out = Vec::new();
    if fw.uefi == Some(false) {
        let mut steps = vec![Step::new(
            "Switching an installed system to UEFI means a GPT partition table with an EFI system partition and reinstalling the bootloader in UEFI mode; for most people a fresh install in UEFI mode is simpler. Back up first.",
        )];
        steps.extend(bios_help::bios_steps(
            ctx.info,
            BiosSetting::Csm,
            "Turn off CSM / legacy boot so the PC can boot in UEFI mode.",
            "",
        ));
        out.push(
            guided(
                Level::Info,
                Category::System,
                "The system boots in legacy BIOS (CSM) mode",
                "The PC starts through the old BIOS compatibility mode, which rules out Secure Boot and Resizable BAR.",
                steps,
            )
            .security(),
        );
    } else if fw.secure_boot == Some(false) {
        out.push(
            guided(
                Level::Info,
                Category::System,
                "Secure Boot is off",
                "Secure Boot checks that only signed code runs while the PC starts. Most distributions boot with it on; out-of-tree kernel modules (such as some NVIDIA driver packages) then need to be signed with a Machine Owner Key, which their installers usually offer to set up.",
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
    if fw.tpm == Tpm::Missing {
        out.push(
            guided(
                Level::Info,
                Category::System,
                "No TPM found",
                "The PC has no active TPM (Trusted Platform Module), which can unlock encrypted disks at boot (systemd-cryptenroll) and measure the boot chain. Most CPUs since 2017 have a firmware TPM that's simply turned off.",
                bios_help::bios_steps(
                    ctx.info,
                    BiosSetting::Tpm,
                    "Turn on the firmware TPM (AMD fTPM or Intel PTT).",
                    "",
                ),
            )
            .security()
            .bios(),
        );
    }
    if out.is_empty() {
        let mut parts = Vec::new();
        if fw.uefi == Some(true) {
            parts.push("UEFI boot");
        }
        if fw.secure_boot == Some(true) {
            parts.push("Secure Boot on");
        }
        match fw.tpm {
            Tpm::V20 => parts.push("TPM 2.0"),
            Tpm::V12 => parts.push("TPM 1.2"),
            _ => {}
        }
        return Outcome::Pass(parts.join(", "));
    }
    Outcome::Findings(out)
}

#[cfg(test)]
mod tests {
    use super::super::storage::storage_link;
    use super::super::testutil::*;
    use super::*;
    use crate::SystemInfo;
    use crate::checkup::{NicLink, PcieLink};
    use crate::sensors::SensorTable;
    use crate::storage::{Drive, SataLink};

    #[test]
    fn system_checks() {
        let s = SensorTable::default();
        let mut info = system(vec![]);
        assert!(matches!(bios_age(&ctx(&info, &s)), Outcome::Pass(_)));
        info.smbios
            .as_mut()
            .unwrap()
            .bios
            .as_mut()
            .unwrap()
            .release_date = "03/15/2021".into();
        assert_eq!(levels(&bios_age(&ctx(&info, &s))), vec![Level::Info]);
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
    fn firmware() {
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
        assert_eq!(levels(&o), vec![Level::Info, Level::Info]);
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
        assert_eq!(levels(&o), vec![Level::Info]);
        info.checkup.firmware.uefi = Some(true);
        info.checkup.firmware.secure_boot = Some(true);
        assert!(matches!(
            firmware_security(&ctx(&info, &s)),
            Outcome::Pass(_)
        ));
    }
}
