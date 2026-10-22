//! Linux system checks: pending restart, swap, kernel taint, the CPU frequency driver and
//! governor, the power profile, PCIe power saving and the NVIDIA driver.

use super::helpers::{distro, finding, plural};
use super::{Category, Context, Level, Outcome};
use crate::checkup::TAINT_PROBLEMS;
use crate::cpuid::Vendor;
use crate::format_bytes;
use crate::platform::Distro;

/// A laptop running on battery (desktops and plugged-in laptops count as on AC).
fn on_battery(ctx: &Context) -> bool {
    ctx.info.battery.is_some_and(|b| !b.on_ac)
}

fn is_desktop(ctx: &Context) -> bool {
    ctx.info.battery.is_none()
}

pub(super) fn pending_restart(ctx: &Context) -> Outcome {
    match &ctx.info.checkup.reboot_required {
        None => Outcome::Pass("No restart pending".into()),
        Some(pkgs) => {
            let detail = if pkgs.is_empty() {
                "Installed updates are waiting for a restart.".to_string()
            } else {
                format!(
                    "Updated packages waiting for a restart: {}",
                    pkgs.join(", ")
                )
            };
            Outcome::Findings(vec![finding(
                Level::Info,
                Category::System,
                "Restart to finish installing updates",
                detail,
                &["Save your work and restart, so the new kernel and libraries are in use."],
            )])
        }
    }
}

pub(super) fn swap_space(ctx: &Context) -> Outcome {
    let Some(swap) = ctx.info.checkup.swap else {
        return Outcome::Skipped("Swap information not available".into());
    };
    let Some(mem) = ctx.info.memory else {
        return Outcome::Skipped("Memory size not available".into());
    };
    if swap.total_bytes > 0 {
        let zram = if swap.zram_bytes > 0 {
            " (with zram)"
        } else {
            ""
        };
        return Outcome::Pass(format!("{} of swap{zram}", format_bytes(swap.total_bytes)));
    }
    // No swap: apps get killed instead of slowed down when memory runs out.
    let level = if mem.total_bytes <= 16 << 30 {
        Level::Warning
    } else {
        Level::Info
    };
    Outcome::Findings(vec![finding(
        level,
        Category::Memory,
        "No swap space",
        format!(
            "{} of RAM and no swap: when memory runs out, the kernel kills apps instead of slowing down.",
            format_bytes(mem.total_bytes)
        ),
        &[
            &match distro(ctx) {
                Distro::Ubuntu | Distro::Debian => {
                    "Turn on zram (compressed swap in RAM): sudo apt install zram-tools.".into()
                }
                Distro::Other => "Turn on zram (compressed swap in RAM) with your distribution's zram-generator or zram-tools package.".into(),
                d => format!(
                    "Turn on zram (compressed swap in RAM): {}, then put a [zram0] section in /etc/systemd/zram-generator.conf and restart.",
                    d.install("zram-generator")
                ),
            },
            "Or create a swap file: sudo fallocate -l 4G /swapfile && sudo chmod 600 /swapfile && sudo mkswap /swapfile && sudo swapon /swapfile, then add it to /etc/fstab.",
        ],
    )])
}

pub(super) fn kernel_taint(ctx: &Context) -> Outcome {
    let Some(taint) = ctx.info.checkup.kernel_taint else {
        return Outcome::Skipped("Kernel taint flags not available".into());
    };
    let hit: Vec<&str> = TAINT_PROBLEMS
        .iter()
        .filter(|(bit, _)| taint & (1 << bit) != 0)
        .map(|(_, what)| *what)
        .collect();
    if hit.is_empty() {
        return Outcome::Pass("No kernel problems since boot".into());
    }
    let serious = taint & ((1 << 4) | (1 << 5) | (1 << 7)) != 0;
    Outcome::Findings(vec![finding(
        if serious { Level::Warning } else { Level::Info },
        Category::System,
        "The kernel reported a problem since boot",
        format!(
            "Since this boot the kernel logged {} (taint flags {taint}).",
            hit.join(", ")
        ),
        &[
            "Open the Event Log page to see the logged message.",
            "Install all updates and restart; if it keeps happening, check the Crashes checks above for its cause.",
        ],
    )])
}

/// AMD Zen 3 and newer (family 19h and up) run better with `amd-pstate` than `acpi-cpufreq`.
pub(super) fn cpu_frequency_driver(ctx: &Context) -> Outcome {
    let Some(p) = ctx.power else {
        return Outcome::Skipped("Power settings not read yet".into());
    };
    if p.cpufreq_driver.is_empty() {
        return Outcome::Skipped("No CPU frequency driver loaded".into());
    }
    let cpu = &ctx.info.cpu;
    if cpu.vendor == Vendor::Amd && cpu.family >= 0x19 && p.cpufreq_driver == "acpi-cpufreq" {
        return Outcome::Findings(vec![finding(
            Level::Info,
            Category::Cpu,
            "AMD P-State driver not in use",
            "This Ryzen runs on the older acpi-cpufreq driver; amd-pstate lets the CPU choose its own clocks, for better speed and power use.",
            &[
                "Check that CPPC is enabled in the BIOS (often under AMD CBS > NBIO or CPU Common Options).",
                "Add amd_pstate=active to the kernel command line: edit GRUB_CMDLINE_LINUX_DEFAULT in /etc/default/grub, run sudo update-grub and restart.",
            ],
        )]);
    }
    Outcome::Pass(p.cpufreq_driver.clone())
}

pub(super) fn cpu_governor(ctx: &Context) -> Outcome {
    let Some(g) = ctx.power.and_then(|p| p.governor.as_ref()) else {
        return Outcome::Skipped("No CPU frequency governor".into());
    };
    let driver = ctx.power.map_or("", |p| p.cpufreq_driver.as_str());
    // With intel_pstate / amd-pstate in active mode, "powersave" is the normal setting.
    let self_managed = driver.starts_with("intel_pstate") || driver.starts_with("amd-pstate");
    if g.current == "powersave" && !self_managed {
        return Outcome::Findings(vec![finding(
            if on_battery(ctx) {
                Level::Info
            } else {
                Level::Warning
            },
            Category::Cpu,
            "CPU held at its lowest clock",
            format!(
                "The {driver} driver's powersave governor keeps every core at its minimum clock."
            ),
            &[
                "Pick the schedutil (or ondemand) governor on the Power page.",
                "Check for a tool that sets it at startup (TLP, auto-cpufreq, cpupower) and change it there to make it stick.",
            ],
        )]);
    }
    if g.current == "userspace" {
        return Outcome::Findings(vec![finding(
            Level::Info,
            Category::Cpu,
            "CPU clocks set by a program",
            "The userspace governor lets a program fix the CPU clock; usually something left over from tuning.",
            &["Pick schedutil (or powersave with intel_pstate / amd-pstate) on the Power page."],
        )]);
    }
    if g.current.is_empty() {
        return Outcome::Pass("Different governors per core".into());
    }
    Outcome::Pass(g.current.clone())
}

pub(super) fn power_profile(ctx: &Context) -> Outcome {
    let Some(c) = ctx.power.and_then(|p| p.power_profile.as_ref()) else {
        return Outcome::Skipped("power-profiles-daemon isn't running".into());
    };
    if c.current == "power-saver" && !on_battery(ctx) {
        return Outcome::Findings(vec![finding(
            if is_desktop(ctx) {
                Level::Warning
            } else {
                Level::Info
            },
            Category::System,
            "Power Saver profile on mains power",
            "Power Saver lowers CPU and GPU speed to save energy, which isn't needed while plugged in.",
            &[
                "Pick Balanced or Performance on the Power page (or in the system menu's power mode).",
            ],
        )]);
    }
    Outcome::Pass(c.current.clone())
}

pub(super) fn pcie_power_saving(ctx: &Context) -> Outcome {
    let Some(a) = ctx.power.and_then(|p| p.aspm.as_ref()) else {
        return Outcome::Skipped("PCIe ASPM policy not available".into());
    };
    if is_desktop(ctx) && a.current == "powersupersave" {
        return Outcome::Findings(vec![finding(
            Level::Info,
            Category::System,
            "Deepest PCIe power saving on a desktop",
            "The powersupersave ASPM policy saves little power on a desktop and can cause GPU or NVMe stutter and wake problems.",
            &[
                "Remove pcie_aspm.policy=powersupersave from the kernel command line (or the TLP setting PCIE_ASPM_ON_AC) and restart.",
            ],
        )]);
    }
    Outcome::Pass(a.current.clone())
}

pub(super) fn nvidia_driver(ctx: &Context) -> Outcome {
    let nvidia: Vec<_> = ctx
        .info
        .gpus
        .iter()
        .filter(|g| g.vendor_id == 0x10DE)
        .collect();
    if nvidia.is_empty() {
        return Outcome::Skipped("No NVIDIA graphics card".into());
    }
    let open_driver: Vec<_> = nvidia
        .iter()
        .filter(|g| !g.driver_version.starts_with("nvidia"))
        .collect();
    if open_driver.is_empty() {
        return Outcome::Pass(nvidia[0].driver_version.clone());
    }
    let names: Vec<&str> = open_driver.iter().map(|g| g.name.as_str()).collect();
    Outcome::Findings(vec![finding(
        Level::Warning,
        Category::Gpu,
        "NVIDIA driver not installed",
        format!(
            "{} {} on {} instead of NVIDIA's driver: much slower in games, no CUDA and no NVIDIA sensors.",
            plural(names.len(), "card runs", "cards run"),
            names.join(", "),
            open_driver
                .first()
                .map(|g| if g.driver_version.is_empty() {
                    "no driver"
                } else {
                    g.driver_version.as_str()
                })
                .unwrap_or("no driver")
        ),
        &[
            match distro(ctx) {
                Distro::Ubuntu => {
                    "Open Software & Updates > Additional Drivers (Driver Manager on Linux Mint) and pick the recommended NVIDIA driver, or run sudo ubuntu-drivers install."
                }
                Distro::Debian => {
                    "Enable the non-free and non-free-firmware components in /etc/apt/sources.list, then run sudo apt install nvidia-driver."
                }
                Distro::Arch => {
                    "Run sudo pacman -S nvidia-open-dkms (GeForce GTX 16 / RTX 20 series and newer); CachyOS: sudo chwd -a."
                }
                Distro::Fedora => {
                    "Enable the RPM Fusion repositories, then run sudo dnf install akmod-nvidia."
                }
                Distro::Suse => {
                    "Run sudo zypper install-new-recommends (new openSUSE installs include NVIDIA's repository; otherwise add it in YaST > Software Repositories)."
                }
                Distro::Other => {
                    "Install NVIDIA's driver from your distribution's repositories (often called nvidia or nvidia-driver)."
                }
            },
            "Restart after installing.",
        ],
    )])
}

#[cfg(test)]
mod tests {
    use super::super::testutil::*;
    use super::*;
    use crate::SystemInfo;
    use crate::checkup::Swap;
    use crate::linux::power::{Choice, PowerInfo};
    use crate::platform::{Battery, MemoryStatus};
    use crate::sensors::SensorTable;

    fn choice(current: &str) -> Option<Choice> {
        Some(Choice {
            current: current.into(),
            options: Vec::new(),
        })
    }

    #[test]
    fn restart_swap_taint() {
        let mut info = SystemInfo {
            memory: Some(MemoryStatus {
                total_bytes: 16 << 30,
                ..Default::default()
            }),
            ..Default::default()
        };
        let t = SensorTable::default();
        assert!(matches!(pending_restart(&ctx(&info, &t)), Outcome::Pass(_)));
        info.checkup.reboot_required = Some(vec!["linux-base".into()]);
        info.checkup.swap = Some(Swap::default());
        info.checkup.kernel_taint = Some(1 | (1 << 9)); // proprietary module + a warning
        let c = ctx(&info, &t);
        assert_eq!(levels(&pending_restart(&c)), [Level::Info]);
        assert_eq!(levels(&swap_space(&c)), [Level::Warning]);
        assert_eq!(levels(&kernel_taint(&c)), [Level::Info]);
        info.checkup.kernel_taint = Some(1 | (1 << 7));
        assert_eq!(levels(&kernel_taint(&ctx(&info, &t))), [Level::Warning]);
        info.checkup.kernel_taint = Some(1 << 12); // out-of-tree module only
        assert!(matches!(kernel_taint(&ctx(&info, &t)), Outcome::Pass(_)));
        info.checkup.swap = Some(Swap {
            total_bytes: 8 << 30,
            zram_bytes: 8 << 30,
        });
        assert!(matches!(swap_space(&ctx(&info, &t)), Outcome::Pass(_)));
    }

    #[test]
    fn power_settings() {
        let mut info = SystemInfo::default();
        info.cpu.vendor = Vendor::Amd;
        info.cpu.family = 0x19;
        let t = SensorTable::default();
        let mut p = PowerInfo {
            cpufreq_driver: "acpi-cpufreq".into(),
            governor: choice("powersave"),
            power_profile: choice("power-saver"),
            aspm: choice("powersupersave"),
            ..Default::default()
        };
        let mut c = ctx(&info, &t);
        assert!(matches!(cpu_governor(&c), Outcome::Skipped(_)));
        c.power = Some(&p);
        assert_eq!(levels(&cpu_frequency_driver(&c)), [Level::Info]);
        assert_eq!(levels(&cpu_governor(&c)), [Level::Warning]);
        assert_eq!(levels(&power_profile(&c)), [Level::Warning], "desktop");
        assert_eq!(levels(&pcie_power_saving(&c)), [Level::Info]);

        // amd-pstate with powersave is the normal setup.
        p.cpufreq_driver = "amd-pstate-epp".into();
        p.power_profile = choice("balanced");
        let c = Context {
            power: Some(&p),
            ..ctx(&info, &t)
        };
        assert!(matches!(cpu_frequency_driver(&c), Outcome::Pass(_)));
        assert!(matches!(cpu_governor(&c), Outcome::Pass(_)));
        assert!(matches!(power_profile(&c), Outcome::Pass(_)));

        // A laptop on battery may use Power Saver and deep ASPM.
        info.battery = Some(Battery {
            percent: 50,
            on_ac: false,
            charging: false,
            seconds_left: None,
        });
        p.power_profile = choice("power-saver");
        let c = Context {
            power: Some(&p),
            ..ctx(&info, &t)
        };
        assert!(matches!(power_profile(&c), Outcome::Pass(_)));
        assert!(matches!(pcie_power_saving(&c), Outcome::Pass(_)));
    }

    #[test]
    fn nvidia() {
        let mut info = crate::testkit::desktop();
        let t = SensorTable::default();
        info.gpus[0].driver_version = "nvidia 581.57".into();
        assert!(matches!(nvidia_driver(&ctx(&info, &t)), Outcome::Pass(_)));
        info.gpus[0].driver_version = "nouveau".into();
        match nvidia_driver(&ctx(&info, &t)) {
            Outcome::Findings(f) => {
                assert_eq!(f[0].level, Level::Warning);
                assert!(f[0].detail.contains("on nouveau"), "{}", f[0].detail);
            }
            o => panic!("{o:?}"),
        }
        assert!(matches!(
            nvidia_driver(&ctx(&crate::testkit::laptop(), &t)),
            Outcome::Skipped(_)
        ));
    }
}
