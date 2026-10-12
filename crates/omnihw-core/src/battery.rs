//! Battery wear from `IOCTL_BATTERY_QUERY_INFORMATION` (`BATTERY_INFORMATION`).

const BATTERY_CAPACITY_RELATIVE: u32 = 0x4000_0000;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BatteryHealth {
    /// Capacity when new, in mWh (or relative units when `relative`).
    pub design_mwh: u32,
    /// Capacity at the last full charge, same units as `design_mwh`.
    pub full_charge_mwh: u32,
    /// Charge cycles, if the battery reports them.
    pub cycle_count: Option<u32>,
    /// Chemistry code, e.g. "LION", "LiP".
    pub chemistry: String,
    /// Capacities are relative (not mWh); only the ratio is meaningful.
    pub relative: bool,
}

impl BatteryHealth {
    /// Capacity lost versus design, in percent (0 when the battery holds at least its design capacity).
    pub fn wear_pct(&self) -> f64 {
        if self.design_mwh == 0 {
            return 0.0;
        }
        (100.0 * (1.0 - self.full_charge_mwh as f64 / self.design_mwh as f64)).max(0.0)
    }

    /// "50.0 Wh of 60.0 Wh design (17 % wear)"
    pub fn summary(&self) -> String {
        let cap = if self.relative {
            format!("{} of {} (design)", self.full_charge_mwh, self.design_mwh)
        } else {
            format!(
                "{:.1} Wh of {:.1} Wh design",
                self.full_charge_mwh as f64 / 1000.0,
                self.design_mwh as f64 / 1000.0
            )
        };
        let cycles = self
            .cycle_count
            .map(|c| format!(", {c} cycles"))
            .unwrap_or_default();
        format!("{cap} ({:.0} % wear{cycles})", self.wear_pct())
    }
}

/// Parse a 36-byte `BATTERY_INFORMATION`: Capabilities u32, Technology u8, Reserved[3],
/// Chemistry[4], DesignedCapacity u32 @12, FullChargedCapacity u32 @16, DefaultAlert1,
/// DefaultAlert2, CriticalBias, CycleCount u32 @32.
pub fn parse_battery_information(b: &[u8]) -> Option<BatteryHealth> {
    let u32_at = |o: usize| {
        b.get(o..o + 4)
            .map(|x| u32::from_le_bytes(x.try_into().unwrap()))
    };
    let caps = u32_at(0)?;
    let design = u32_at(12)?;
    let full = u32_at(16)?;
    let cycles = u32_at(32)?;
    if design == 0 || design == u32::MAX || full == u32::MAX {
        return None;
    }
    let chemistry = String::from_utf8_lossy(&b[8..12])
        .trim_matches(|c: char| c == '\0' || c.is_whitespace())
        .to_string();
    Some(BatteryHealth {
        design_mwh: design,
        full_charge_mwh: full,
        cycle_count: (cycles > 0).then_some(cycles),
        chemistry,
        relative: caps & BATTERY_CAPACITY_RELATIVE != 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(caps: u32, design: u32, full: u32, cycles: u32) -> Vec<u8> {
        let mut b = vec![0u8; 36];
        b[0..4].copy_from_slice(&caps.to_le_bytes());
        b[4] = 1;
        b[8..12].copy_from_slice(b"LION");
        b[12..16].copy_from_slice(&design.to_le_bytes());
        b[16..20].copy_from_slice(&full.to_le_bytes());
        b[32..36].copy_from_slice(&cycles.to_le_bytes());
        b
    }

    #[test]
    fn wear_and_summary() {
        let h = parse_battery_information(&info(0x8000_0000, 60_000, 45_000, 312)).unwrap();
        assert_eq!(h.wear_pct(), 25.0);
        assert_eq!(h.cycle_count, Some(312));
        assert_eq!(h.chemistry, "LION");
        assert!(!h.relative);
        assert_eq!(
            h.summary(),
            "45.0 Wh of 60.0 Wh design (25 % wear, 312 cycles)"
        );
    }

    #[test]
    fn edge_cases() {
        let fresh = parse_battery_information(&info(0, 50_000, 52_000, 0)).unwrap();
        assert_eq!(fresh.wear_pct(), 0.0);
        assert_eq!(fresh.cycle_count, None);
        let rel = parse_battery_information(&info(BATTERY_CAPACITY_RELATIVE, 100, 70, 0)).unwrap();
        assert!(rel.relative);
        assert_eq!(rel.wear_pct().round(), 30.0);
        assert!(parse_battery_information(&info(0, 0, 0, 0)).is_none());
        assert!(parse_battery_information(&[0u8; 20]).is_none());
    }
}
