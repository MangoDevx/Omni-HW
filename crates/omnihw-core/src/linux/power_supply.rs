//! Battery charge and wear from `/sys/class/power_supply`.

use super::Root;
use crate::battery::BatteryHealth;
use crate::platform::Battery;

const PS: &str = "/sys/class/power_supply";

/// The first system battery (peripheral batteries, `scope=Device`, are skipped).
fn system_battery(root: &Root) -> Option<String> {
    root.list(PS).into_iter().find(|n| {
        let p = format!("{PS}/{n}");
        root.read(&format!("{p}/type")).as_deref() == Some("Battery")
            && root.read(&format!("{p}/scope")).as_deref() != Some("Device")
            && root.read(&format!("{p}/present")).as_deref() != Some("0")
    })
}

/// Whether any mains/USB supply is online; `None` if there is no such supply.
fn mains_online(root: &Root) -> Option<bool> {
    let mut seen = false;
    for n in root.list(PS) {
        let p = format!("{PS}/{n}");
        if !matches!(
            root.read(&format!("{p}/type")).as_deref(),
            Some("Mains" | "USB")
        ) {
            continue;
        }
        seen = true;
        if root.read(&format!("{p}/online")).as_deref() == Some("1") {
            return Some(true);
        }
    }
    seen.then_some(false)
}

pub fn battery(root: &Root) -> Option<Battery> {
    let bat = system_battery(root)?;
    let p = format!("{PS}/{bat}");
    let get = |f: &str| root.read_u64(&format!("{p}/{f}"));
    let status = root.read(&format!("{p}/status")).unwrap_or_default();
    let percent = get("capacity")
        .or_else(|| {
            let (now, full) = get("energy_now")
                .zip(get("energy_full"))
                .or_else(|| get("charge_now").zip(get("charge_full")))?;
            (full > 0).then(|| now * 100 / full)
        })?
        .min(100) as u8;
    let discharging = status == "Discharging";
    // Remaining time while discharging: energy / power, or charge / current.
    let seconds_left = if discharging {
        get("energy_now")
            .zip(get("power_now"))
            .or_else(|| get("charge_now").zip(get("current_now")))
            .filter(|&(_, rate)| rate > 0)
            .map(|(left, rate)| (left as f64 / rate as f64 * 3600.0) as u32)
    } else {
        None
    };
    Some(Battery {
        percent,
        on_ac: mains_online(root).unwrap_or(!discharging),
        charging: status == "Charging",
        seconds_left,
    })
}

pub fn battery_health(root: &Root) -> Option<BatteryHealth> {
    let bat = system_battery(root)?;
    let p = format!("{PS}/{bat}");
    let get = |f: &str| root.read_u64(&format!("{p}/{f}")).filter(|&v| v > 0);
    let to_u32 = |v: u64| u32::try_from(v).unwrap_or(u32::MAX);
    let (design, full, relative) =
        if let (Some(d), Some(f)) = (get("energy_full_design"), get("energy_full")) {
            // µWh -> mWh
            (to_u32(d / 1000), to_u32(f / 1000), false)
        } else {
            let (d, f) = get("charge_full_design").zip(get("charge_full"))?;
            match get("voltage_min_design") {
                // µAh x µV -> mWh
                Some(uv) => (
                    to_u32((d as u128 * uv as u128 / 1_000_000_000) as u64),
                    to_u32((f as u128 * uv as u128 / 1_000_000_000) as u64),
                    false,
                ),
                // No voltage: only the ratio is meaningful (mAh).
                None => (to_u32(d / 1000), to_u32(f / 1000), true),
            }
        };
    if design == 0 {
        return None;
    }
    Some(BatteryHealth {
        design_mwh: design,
        full_charge_mwh: full,
        cycle_count: get("cycle_count").map(to_u32),
        chemistry: root
            .read(&format!("{p}/technology"))
            .filter(|t| t != "Unknown")
            .unwrap_or_default(),
        relative,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::linux::Fixture;

    fn laptop() -> Fixture {
        let f = Fixture::new();
        f.file("sys/class/power_supply/AC/type", "Mains")
            .file("sys/class/power_supply/AC/online", "0")
            .file("sys/class/power_supply/hidpp_battery_0/type", "Battery")
            .file("sys/class/power_supply/hidpp_battery_0/scope", "Device")
            .file("sys/class/power_supply/hidpp_battery_0/capacity", "5")
            .file("sys/class/power_supply/BAT0/type", "Battery")
            .file("sys/class/power_supply/BAT0/status", "Discharging")
            .file("sys/class/power_supply/BAT0/capacity", "80")
            .file("sys/class/power_supply/BAT0/energy_now", "40000000")
            .file("sys/class/power_supply/BAT0/power_now", "10000000")
            .file("sys/class/power_supply/BAT0/energy_full", "50000000")
            .file("sys/class/power_supply/BAT0/energy_full_design", "60000000")
            .file("sys/class/power_supply/BAT0/cycle_count", "123")
            .file("sys/class/power_supply/BAT0/technology", "Li-ion");
        f
    }

    #[test]
    fn discharging_laptop() {
        let f = laptop();
        let b = battery(&f.root()).unwrap();
        assert_eq!(b.percent, 80);
        assert!(!b.on_ac && !b.charging);
        assert_eq!(b.seconds_left, Some(4 * 3600));
        let h = battery_health(&f.root()).unwrap();
        assert_eq!((h.design_mwh, h.full_charge_mwh), (60_000, 50_000));
        assert_eq!(h.cycle_count, Some(123));
        assert_eq!(h.chemistry, "Li-ion");
        assert!(!h.relative);
    }

    #[test]
    fn charge_based_battery() {
        let f = Fixture::new();
        f.file("sys/class/power_supply/BAT1/type", "Battery")
            .file("sys/class/power_supply/BAT1/status", "Charging")
            .file("sys/class/power_supply/BAT1/charge_now", "2000000")
            .file("sys/class/power_supply/BAT1/charge_full", "4000000")
            .file("sys/class/power_supply/BAT1/charge_full_design", "5000000")
            .file("sys/class/power_supply/BAT1/voltage_min_design", "11400000");
        let b = battery(&f.root()).unwrap();
        assert_eq!(b.percent, 50);
        assert!(b.on_ac && b.charging);
        assert_eq!(b.seconds_left, None);
        let h = battery_health(&f.root()).unwrap();
        assert_eq!((h.design_mwh, h.full_charge_mwh), (57_000, 45_600));
        assert_eq!(h.cycle_count, None);
    }

    #[test]
    fn desktop_has_none() {
        let f = Fixture::new();
        f.file("sys/class/power_supply/AC/type", "Mains");
        assert_eq!(battery(&f.root()), None);
        assert_eq!(battery_health(&f.root()), None);
    }
}
