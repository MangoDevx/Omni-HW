//! Power page data: CPU frequency scaling (cpufreq governor, energy/performance preference,
//! boost), the firmware platform profile, power-profiles-daemon, PCIe ASPM, USB autosuspend,
//! the sleep mode and ACPI wake devices.
//!
//! Reading needs no privileges. Changing a kernel setting needs root: the app runs
//! `pkexec omnihw --root-set <setting> <value>`, which calls [`apply`]. [`apply`] only writes the
//! files of the [`Setting`]s below, and only values the kernel lists as available, so the
//! helper can't be used to write anything else. Kernel settings last until the next restart.

use super::Root;
use std::process::Command;

const CPUFREQ: &str = "/sys/devices/system/cpu/cpufreq";
const PLATFORM_PROFILE: &str = "/sys/firmware/acpi/platform_profile";
const PLATFORM_PROFILE_CHOICES: &str = "/sys/firmware/acpi/platform_profile_choices";
const BOOST: &str = "/sys/devices/system/cpu/cpufreq/boost";
const INTEL_NO_TURBO: &str = "/sys/devices/system/cpu/intel_pstate/no_turbo";
const ASPM_POLICY: &str = "/sys/module/pcie_aspm/parameters/policy";
const USB_AUTOSUSPEND: &str = "/sys/module/usbcore/parameters/autosuspend";
const MEM_SLEEP: &str = "/sys/power/mem_sleep";
const ACPI_WAKEUP: &str = "/proc/acpi/wakeup";

/// A setting with one current value out of a list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Choice {
    /// Empty when the policies disagree (see [`PowerInfo::governor`]).
    pub current: String,
    pub options: Vec<String>,
}

impl Choice {
    /// A kernel list with the current value in brackets: "default [performance] powersave".
    pub fn parse_bracketed(s: &str) -> Option<Choice> {
        let mut c = Choice::default();
        for w in s.split_whitespace() {
            match w.strip_prefix('[').and_then(|w| w.strip_suffix(']')) {
                Some(cur) => {
                    c.current = cur.to_string();
                    c.options.push(cur.to_string());
                }
                None => c.options.push(w.to_string()),
            }
        }
        (!c.options.is_empty()).then_some(c)
    }
}

/// A device that may wake the PC from sleep (`/proc/acpi/wakeup`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WakeDevice {
    /// ACPI name, e.g. "XHC0".
    pub name: String,
    pub enabled: bool,
    /// The device behind it, e.g. "pci:0000:0c:00.3"; may be empty.
    pub sysfs: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PowerInfo {
    /// cpufreq scaling driver, e.g. "amd-pstate-epp", "intel_pstate", "acpi-cpufreq".
    pub cpufreq_driver: String,
    /// `intel_pstate` / `amd_pstate` operation mode, e.g. "active", "passive", "guided".
    pub pstate_mode: Option<String>,
    /// The governor of every CPU (empty `current` if they differ).
    pub governor: Option<Choice>,
    /// Energy/performance preference (EPP), when the driver supports it.
    pub epp: Option<Choice>,
    /// CPU boost (Turbo Boost / Precision Boost) enabled.
    pub boost: Option<bool>,
    /// The firmware's platform profile (ACPI), e.g. "balanced", "performance".
    pub platform_profile: Option<Choice>,
    /// power-profiles-daemon (or tuned-ppd): the active profile and the available ones.
    pub power_profile: Option<Choice>,
    /// PCIe Active State Power Management policy.
    pub aspm: Option<Choice>,
    /// USB autosuspend delay in seconds (negative: autosuspend off).
    pub usb_autosuspend_secs: Option<i64>,
    /// Suspend mode: "s2idle" (suspend-to-idle) or "deep" (S3).
    pub mem_sleep: Option<Choice>,
    pub wake_devices: Vec<WakeDevice>,
}

/// The cpufreq policy directories ("policy0", ...).
fn policies(root: &Root) -> Vec<String> {
    root.list(CPUFREQ)
        .into_iter()
        .filter(|p| p.starts_with("policy"))
        .collect()
}

/// A per-policy setting as one choice: the common value (empty if they differ) and the
/// options of the first policy.
fn policy_choice(root: &Root, policies: &[String], file: &str, options: &str) -> Option<Choice> {
    let values: Vec<String> = policies
        .iter()
        .filter_map(|p| root.read(&format!("{CPUFREQ}/{p}/{file}")))
        .collect();
    let first = values.first()?;
    let options = policies
        .first()
        .and_then(|p| root.read(&format!("{CPUFREQ}/{p}/{options}")))
        .map(|o| o.split_whitespace().map(String::from).collect())
        .unwrap_or_default();
    Some(Choice {
        current: if values.iter().all(|v| v == first) {
            first.clone()
        } else {
            String::new()
        },
        options,
    })
}

/// `/proc/acpi/wakeup`: "Device  S-state  Status  Sysfs node" rows, status "*enabled" or
/// "*disabled".
pub fn parse_wakeup(s: &str) -> Vec<WakeDevice> {
    s.lines()
        .skip(1)
        .filter_map(|l| {
            let mut f = l.split_whitespace();
            let name = f.next()?.to_string();
            let _state = f.next()?;
            let status = f.next()?;
            Some(WakeDevice {
                name,
                enabled: status.trim_start_matches('*') == "enabled",
                sysfs: f.next().unwrap_or_default().to_string(),
            })
        })
        .collect()
}

/// `powerprofilesctl list`: one "name:" line per profile, the active one marked with "*".
pub fn parse_power_profiles(s: &str) -> Option<Choice> {
    let mut c = Choice::default();
    for l in s.lines() {
        let Some(name) = l.trim_end().strip_suffix(':') else {
            continue;
        };
        // Profile lines start in column 0 or 2; details are indented further.
        let (active, name) = match name.strip_prefix("* ") {
            Some(n) => (true, n),
            None => (false, name.strip_prefix("  ").unwrap_or(name)),
        };
        if name.is_empty() || name.starts_with(' ') || name.contains(char::is_whitespace) {
            continue;
        }
        if active {
            c.current = name.to_string();
        }
        c.options.push(name.to_string());
    }
    (!c.options.is_empty()).then_some(c)
}

fn power_profiles() -> Option<Choice> {
    let out = Command::new("powerprofilesctl").arg("list").output().ok()?;
    out.status
        .success()
        .then(|| parse_power_profiles(&String::from_utf8_lossy(&out.stdout)))
        .flatten()
}

/// Switch the power-profiles-daemon profile (no root needed for the active session).
pub fn set_power_profile(name: &str) -> Result<(), String> {
    let out = Command::new("powerprofilesctl")
        .args(["set", name])
        .output()
        .map_err(|e| format!("Couldn't run powerprofilesctl: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// Everything but power-profiles-daemon, from files under `root`.
pub fn read_files(root: &Root) -> PowerInfo {
    let pol = policies(root);
    let pstate_mode = ["intel_pstate", "amd_pstate"]
        .iter()
        .find_map(|d| root.read(&format!("/sys/devices/system/cpu/{d}/status")));
    let boost = match root.read_u64(BOOST) {
        Some(b) => Some(b == 1),
        None => root.read_u64(INTEL_NO_TURBO).map(|n| n == 0),
    };
    let platform_profile = root.read(PLATFORM_PROFILE).map(|current| Choice {
        current,
        options: root
            .read(PLATFORM_PROFILE_CHOICES)
            .map(|o| o.split_whitespace().map(String::from).collect())
            .unwrap_or_default(),
    });
    PowerInfo {
        cpufreq_driver: pol
            .first()
            .and_then(|p| root.read(&format!("{CPUFREQ}/{p}/scaling_driver")))
            .unwrap_or_default(),
        pstate_mode,
        governor: policy_choice(
            root,
            &pol,
            "scaling_governor",
            "scaling_available_governors",
        ),
        epp: policy_choice(
            root,
            &pol,
            "energy_performance_preference",
            "energy_performance_available_preferences",
        ),
        boost,
        platform_profile,
        power_profile: None,
        aspm: root
            .read(ASPM_POLICY)
            .and_then(|s| Choice::parse_bracketed(&s)),
        usb_autosuspend_secs: root.read_i64(USB_AUTOSUSPEND),
        mem_sleep: root
            .read(MEM_SLEEP)
            .and_then(|s| Choice::parse_bracketed(&s)),
        wake_devices: root
            .read(ACPI_WAKEUP)
            .map(|s| parse_wakeup(&s))
            .unwrap_or_default(),
    }
}

/// Everything, including power-profiles-daemon. Blocking (runs `powerprofilesctl`): call it on
/// a worker thread.
pub fn read() -> PowerInfo {
    PowerInfo {
        power_profile: power_profiles(),
        ..read_files(&Root::system())
    }
}

/// A kernel setting the Power page can change (through the root helper).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Setting {
    Governor,
    Epp,
    Boost,
    PlatformProfile,
}

impl Setting {
    pub const ALL: [Setting; 4] = [
        Setting::Governor,
        Setting::Epp,
        Setting::Boost,
        Setting::PlatformProfile,
    ];

    /// The helper's command-line name.
    pub fn key(self) -> &'static str {
        match self {
            Setting::Governor => "governor",
            Setting::Epp => "epp",
            Setting::Boost => "boost",
            Setting::PlatformProfile => "platform-profile",
        }
    }

    pub fn from_key(key: &str) -> Option<Setting> {
        Setting::ALL.into_iter().find(|s| s.key() == key)
    }

    pub fn label(self) -> &'static str {
        match self {
            Setting::Governor => "CPU governor",
            Setting::Epp => "Energy/performance preference",
            Setting::Boost => "CPU boost",
            Setting::PlatformProfile => "Platform profile",
        }
    }
}

fn write(root: &Root, rel: &str, value: &str) -> Result<(), String> {
    std::fs::write(root.path(rel), value).map_err(|e| format!("{rel}: {e}"))
}

/// Set `setting` to `value`, after checking `value` is one the kernel offers. For the root
/// helper; also works directly when already root.
pub fn apply(root: &Root, setting: Setting, value: &str) -> Result<(), String> {
    let info = read_files(root);
    let allowed = |c: &Option<Choice>| {
        c.as_ref()
            .is_some_and(|c| c.options.iter().any(|o| o == value))
    };
    let per_policy = |file: &str| {
        let pol = policies(root);
        if pol.is_empty() {
            return Err("no cpufreq policies".to_string());
        }
        pol.iter()
            .try_for_each(|p| write(root, &format!("{CPUFREQ}/{p}/{file}"), value))
    };
    match setting {
        Setting::Governor if allowed(&info.governor) => per_policy("scaling_governor"),
        Setting::Epp if allowed(&info.epp) => per_policy("energy_performance_preference"),
        Setting::PlatformProfile if allowed(&info.platform_profile) => {
            write(root, PLATFORM_PROFILE, value)
        }
        Setting::Boost if matches!(value, "on" | "off") => {
            let on = value == "on";
            if root.exists(BOOST) {
                write(root, BOOST, if on { "1" } else { "0" })
            } else if root.exists(INTEL_NO_TURBO) {
                write(root, INTEL_NO_TURBO, if on { "0" } else { "1" })
            } else {
                Err("this CPU's driver has no boost switch".into())
            }
        }
        _ => Err(format!("\"{value}\" isn't a valid {}", setting.label())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::linux::Fixture;

    fn amd() -> Fixture {
        let f = Fixture::new();
        for p in ["policy0", "policy1"] {
            let d = format!("sys/devices/system/cpu/cpufreq/{p}");
            f.file(&format!("{d}/scaling_driver"), "amd-pstate-epp\n")
                .file(&format!("{d}/scaling_governor"), "powersave\n")
                .file(
                    &format!("{d}/scaling_available_governors"),
                    "performance powersave\n",
                )
                .file(
                    &format!("{d}/energy_performance_preference"),
                    "balance_performance\n",
                )
                .file(
                    &format!("{d}/energy_performance_available_preferences"),
                    "default performance balance_performance balance_power power \n",
                );
        }
        f.file("sys/devices/system/cpu/amd_pstate/status", "active\n")
            .file("sys/devices/system/cpu/cpufreq/boost", "1\n")
            .file("sys/firmware/acpi/platform_profile", "balanced\n")
            .file(
                "sys/firmware/acpi/platform_profile_choices",
                "low-power balanced performance\n",
            )
            .file(
                "sys/module/pcie_aspm/parameters/policy",
                "[default] performance powersave powersupersave \n",
            )
            .file("sys/module/usbcore/parameters/autosuspend", "2\n")
            .file("sys/power/mem_sleep", "s2idle [deep]\n")
            .file(
                "proc/acpi/wakeup",
                "Device\tS-state\t  Status   Sysfs node\nGPP0\t  S4\t*disabled  pci:0000:00:01.1\nXHC0\t  S4\t*enabled   pci:0000:0c:00.3\nPWRB\t  S3\t*enabled\n",
            );
        f
    }

    #[test]
    fn reads_everything() {
        let f = amd();
        let p = read_files(&f.root());
        assert_eq!(p.cpufreq_driver, "amd-pstate-epp");
        assert_eq!(p.pstate_mode.as_deref(), Some("active"));
        let g = p.governor.unwrap();
        assert_eq!(g.current, "powersave");
        assert_eq!(g.options, ["performance", "powersave"]);
        assert_eq!(p.epp.unwrap().options.len(), 5);
        assert_eq!(p.boost, Some(true));
        assert_eq!(p.platform_profile.unwrap().current, "balanced");
        assert_eq!(p.aspm.unwrap().current, "default");
        assert_eq!(p.usb_autosuspend_secs, Some(2));
        assert_eq!(p.mem_sleep.unwrap().current, "deep");
        assert_eq!(p.wake_devices.len(), 3);
        assert!(!p.wake_devices[0].enabled);
        assert_eq!(p.wake_devices[1].sysfs, "pci:0000:0c:00.3");
        assert_eq!(p.wake_devices[2].sysfs, "");
    }

    #[test]
    fn mixed_governors() {
        let f = amd();
        f.file(
            "sys/devices/system/cpu/cpufreq/policy1/scaling_governor",
            "performance\n",
        );
        assert_eq!(read_files(&f.root()).governor.unwrap().current, "");
    }

    #[test]
    fn power_profiles() {
        let text = "  performance:\n    CpuDriver:\tamd_pstate\n    PlatformDriver:\tplatform_profile\n    Degraded:   no\n\n* balanced:\n    CpuDriver:\tamd_pstate\n    PlatformDriver:\tplatform_profile\n\n  power-saver:\n    CpuDriver:\tamd_pstate\n";
        let c = parse_power_profiles(text).unwrap();
        assert_eq!(c.current, "balanced");
        assert_eq!(c.options, ["performance", "balanced", "power-saver"]);
        assert_eq!(parse_power_profiles(""), None);
    }

    #[test]
    fn apply_checks_values() {
        let f = amd();
        let root = f.root();
        apply(&root, Setting::Governor, "performance").unwrap();
        let g = read_files(&root).governor.unwrap();
        assert_eq!(g.current, "performance");
        apply(&root, Setting::Epp, "power").unwrap();
        assert_eq!(read_files(&root).epp.unwrap().current, "power");
        apply(&root, Setting::Boost, "off").unwrap();
        assert_eq!(read_files(&root).boost, Some(false));
        apply(&root, Setting::PlatformProfile, "performance").unwrap();
        // Only values the kernel lists are written.
        assert!(apply(&root, Setting::Governor, "../../x").is_err());
        assert!(apply(&root, Setting::PlatformProfile, "turbo").is_err());
        assert!(apply(&root, Setting::Boost, "1").is_err());
        assert_eq!(read_files(&root).governor.unwrap().current, "performance");
        assert_eq!(Setting::from_key("epp"), Some(Setting::Epp));
        assert_eq!(Setting::from_key("x"), None);
    }
}
