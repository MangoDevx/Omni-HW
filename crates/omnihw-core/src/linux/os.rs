//! Distribution, kernel, uptime, memory and CPU details from procfs and sysfs.

use super::{Root, key_value, parse_int};
use crate::checkup::Swap;
use crate::platform::{MemoryStatus, OsInfo};

/// Distribution name from os-release, kernel release and host name.
pub fn os_info(root: &Root) -> Option<OsInfo> {
    let release = root
        .read("/etc/os-release")
        .or_else(|| root.read("/usr/lib/os-release"));
    let product = release
        .as_deref()
        .and_then(|r| key_value(r, "PRETTY_NAME").or_else(|| key_value(r, "NAME")))
        .unwrap_or_else(|| "Linux".into());
    let kernel = root.read("/proc/sys/kernel/osrelease").unwrap_or_default();
    let computer_name = root
        .read_nonempty("/proc/sys/kernel/hostname")
        .or_else(|| root.read_nonempty("/etc/hostname"))
        .unwrap_or_default();
    if release.is_none() && kernel.is_empty() {
        return None;
    }
    Some(OsInfo {
        product,
        kernel,
        computer_name,
    })
}

/// Seconds since boot, from `/proc/uptime`.
pub fn uptime_secs(root: &Root) -> Option<u64> {
    let s = root.read("/proc/uptime")?;
    let secs: f64 = s.split_whitespace().next()?.parse().ok()?;
    Some(secs as u64)
}

/// `/proc/meminfo` value of `key`, in bytes.
fn meminfo_bytes(text: &str, key: &str) -> Option<u64> {
    text.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        if k.trim() != key {
            return None;
        }
        let kb: u64 = v.split_whitespace().next()?.parse().ok()?;
        Some(kb * 1024)
    })
}

pub fn parse_meminfo(text: &str) -> Option<MemoryStatus> {
    let total = meminfo_bytes(text, "MemTotal")?;
    // MemAvailable exists since Linux 3.14; older kernels only have MemFree.
    let available = meminfo_bytes(text, "MemAvailable")
        .or_else(|| meminfo_bytes(text, "MemFree"))?
        .min(total);
    if total == 0 {
        return None;
    }
    let used = total - available;
    Some(MemoryStatus {
        load_percent: ((used as f64 / total as f64) * 100.0).round() as u32,
        total_bytes: total,
        available_bytes: available,
    })
}

pub fn memory_status(root: &Root) -> Option<MemoryStatus> {
    parse_meminfo(&root.read("/proc/meminfo")?)
}

/// Swap (total, free) in bytes.
pub fn swap(root: &Root) -> Option<(u64, u64)> {
    let text = root.read("/proc/meminfo")?;
    Some((
        meminfo_bytes(&text, "SwapTotal")?,
        meminfo_bytes(&text, "SwapFree")?,
    ))
}

/// Effective uid 0, from the `Uid:` line of `/proc/self/status` (real, effective, ...).
pub fn is_root(root: &Root) -> bool {
    root.read("/proc/self/status")
        .and_then(|s| {
            s.lines()
                .find_map(|l| l.strip_prefix("Uid:"))
                .and_then(|ids| ids.split_whitespace().nth(1)?.parse::<u32>().ok())
        })
        .is_some_and(|uid| uid == 0)
}

/// First value of `key` in `/proc/cpuinfo` text.
pub fn cpuinfo_value<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    text.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        (k.trim() == key).then(|| v.trim())
    })
}

/// Loaded microcode revision: sysfs (per CPU) or the `microcode` line of `/proc/cpuinfo`.
pub fn microcode(root: &Root) -> Option<u32> {
    let v = root
        .read("/sys/devices/system/cpu/cpu0/microcode/version")
        .and_then(|s| parse_int(&s))
        .or_else(|| {
            let text = root.read("/proc/cpuinfo")?;
            parse_int(cpuinfo_value(&text, "microcode")?)
        })?;
    u32::try_from(v).ok().filter(|&v| v != 0)
}

/// "@ 3.60GHz" at the end of Intel brand strings.
pub fn brand_ghz_mhz(brand: &str) -> Option<u32> {
    let (_, tail) = brand.rsplit_once('@')?;
    let ghz: f64 = tail.trim().strip_suffix("GHz")?.trim().parse().ok()?;
    Some((ghz * 1000.0).round() as u32)
}

/// Base clock: intel_pstate's `base_frequency`, else the brand string's "@ x.xxGHz".
pub fn nominal_mhz(root: &Root) -> Option<u32> {
    if let Some(khz) = root.read_u64("/sys/devices/system/cpu/cpu0/cpufreq/base_frequency") {
        return u32::try_from(khz / 1000).ok().filter(|&m| m > 0);
    }
    let text = root.read("/proc/cpuinfo")?;
    brand_ghz_mhz(cpuinfo_value(&text, "model name")?)
}

/// The packages waiting for a restart when the system asks for one (Debian/Ubuntu's
/// `/run/reboot-required`, written by package scripts); `None` when no restart is pending.
pub fn reboot_required(root: &Root) -> Option<Vec<String>> {
    if !root.exists("/run/reboot-required") {
        return None;
    }
    let mut pkgs: Vec<String> = root
        .read("/run/reboot-required.pkgs")
        .unwrap_or_default()
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    pkgs.dedup();
    Some(pkgs)
}

/// Active swap from `/proc/swaps` ("Filename Type Size Used Priority", sizes in KiB).
pub fn parse_swaps(text: &str) -> Swap {
    let mut s = Swap::default();
    for l in text.lines().skip(1) {
        let f: Vec<&str> = l.split_whitespace().collect();
        let Some(kib) = f.get(2).and_then(|v| v.parse::<u64>().ok()) else {
            continue;
        };
        s.total_bytes += kib * 1024;
        if f[0].starts_with("/dev/zram") {
            s.zram_bytes += kib * 1024;
        }
    }
    s
}

pub fn swap_space(root: &Root) -> Option<Swap> {
    root.read("/proc/swaps").map(|t| parse_swaps(&t))
}

/// The kernel's taint flags (`/proc/sys/kernel/tainted`).
pub fn kernel_taint(root: &Root) -> Option<u64> {
    root.read_u64("/proc/sys/kernel/tainted")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::linux::Fixture;

    #[test]
    fn pending_restart_swap_and_taint() {
        let f = Fixture::new();
        assert_eq!(reboot_required(&f.root()), None);
        assert_eq!(swap_space(&f.root()), None);
        f.file("run/reboot-required", "*** System restart required ***\n")
            .file(
                "run/reboot-required.pkgs",
                "linux-base\nlinux-base\nlibc6\n",
            )
            .file(
                "proc/swaps",
                "Filename\t\t\t\tType\t\tSize\t\tUsed\t\tPriority\n\
                 /swap.img                               file\t\t4194300\t\t0\t\t-2\n\
                 /dev/zram0                              partition\t8388604\t\t1024\t\t100\n",
            )
            .file("proc/sys/kernel/tainted", "4609\n");
        assert_eq!(
            reboot_required(&f.root()),
            Some(vec!["linux-base".to_string(), "libc6".to_string()])
        );
        let s = swap_space(&f.root()).unwrap();
        assert_eq!(s.total_bytes, (4194300 + 8388604) * 1024);
        assert_eq!(s.zram_bytes, 8388604 * 1024);
        assert_eq!(kernel_taint(&f.root()), Some(4609));
        assert_eq!(
            parse_swaps("Filename Type Size Used Priority\n"),
            Swap::default()
        );
    }

    #[test]
    fn os_release() {
        let f = Fixture::new();
        f.file(
            "etc/os-release",
            "NAME=\"Ubuntu\"\nPRETTY_NAME=\"Ubuntu 24.04.1 LTS\"\n",
        )
        .file("proc/sys/kernel/osrelease", "6.8.0-45-generic\n")
        .file("proc/sys/kernel/hostname", "box\n");
        let o = os_info(&f.root()).unwrap();
        assert_eq!(o.product, "Ubuntu 24.04.1 LTS");
        assert_eq!(o.kernel, "6.8.0-45-generic");
        assert_eq!(o.computer_name, "box");
        assert_eq!(o.summary(), "Ubuntu 24.04.1 LTS (kernel 6.8.0-45-generic)");
    }

    #[test]
    fn meminfo() {
        let m = parse_meminfo(
            "MemTotal:       16000000 kB\nMemFree:  1000 kB\nMemAvailable:    4000000 kB\n",
        )
        .unwrap();
        assert_eq!(m.total_bytes, 16_000_000 * 1024);
        assert_eq!(m.available_bytes, 4_000_000 * 1024);
        assert_eq!(m.load_percent, 75);
    }

    #[test]
    fn uptime_and_uid() {
        let f = Fixture::new();
        f.file("proc/uptime", "12345.67 54321.00\n").file(
            "proc/self/status",
            "Name:\tx\nUid:\t1000\t0\t1000\t1000\nGid:\t1000\n",
        );
        assert_eq!(uptime_secs(&f.root()), Some(12345));
        assert!(is_root(&f.root()));
        f.file("proc/self/status", "Uid:\t0\t1000\t0\t0\n");
        assert!(!is_root(&f.root()));
    }

    #[test]
    fn cpu_details() {
        let f = Fixture::new();
        f.file(
            "proc/cpuinfo",
            "processor\t: 0\nmodel name\t: Intel(R) Core(TM) i7-8700K CPU @ 3.70GHz\nmicrocode\t: 0xf4\n",
        );
        assert_eq!(microcode(&f.root()), Some(0xf4));
        assert_eq!(nominal_mhz(&f.root()), Some(3700));
        f.file(
            "sys/devices/system/cpu/cpu0/cpufreq/base_frequency",
            "3600000\n",
        );
        assert_eq!(nominal_mhz(&f.root()), Some(3600));
        assert_eq!(brand_ghz_mhz("AMD Ryzen 7 7800X3D 8-Core Processor"), None);
    }
}
