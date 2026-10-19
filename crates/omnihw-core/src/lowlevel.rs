//! Decoders for register-level sensors read through the optional PawnIO driver: CPU MSRs and
//! SMN registers, and motherboard Super I/O chips (fans, temperatures, voltages).
//!
//! Platform-agnostic so the bit layouts are unit-tested anywhere; the reads themselves live in
//! `win::pawnio`.

/// Whether the PawnIO driver can be used right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PawnioState {
    NotInstalled,
    /// Installed, but this process isn't elevated (the device only admits administrators).
    NeedsAdmin,
    /// Installed but the device didn't open (driver not started yet, e.g. before a reboot).
    NotRunning(String),
    Ready,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PawnioStatus {
    pub state: PawnioState,
    /// Installed driver version, e.g. "2.2.0".
    pub version: Option<String>,
}

// --- Intel MSRs ---

pub const MSR_IA32_PERF_STATUS: u32 = 0x198;
pub const MSR_IA32_THERM_STATUS: u32 = 0x19C;
pub const MSR_IA32_TEMPERATURE_TARGET: u32 = 0x1A2;
pub const MSR_IA32_PACKAGE_THERM_STATUS: u32 = 0x1B1;

/// TjMax (°C) from `IA32_TEMPERATURE_TARGET` bits 23:16; `None` if implausible.
pub fn intel_tjmax(temperature_target: u64) -> Option<f64> {
    let t = (temperature_target >> 16) & 0xFF;
    (50..=150).contains(&t).then_some(t as f64)
}

/// Core temperature from `IA32_THERM_STATUS`: TjMax minus the digital readout (bits 22:16),
/// `None` unless the reading-valid bit (31) is set.
pub fn intel_core_temp(therm_status: u64, tjmax: f64) -> Option<f64> {
    (therm_status & (1 << 31) != 0).then_some(tjmax - ((therm_status >> 16) & 0x7F) as f64)
}

/// Package temperature from `IA32_PACKAGE_THERM_STATUS` (same readout field, no valid bit).
pub fn intel_package_temp(therm_status: u64, tjmax: f64) -> f64 {
    tjmax - ((therm_status >> 16) & 0x7F) as f64
}

/// Core voltage (VID) from `IA32_PERF_STATUS` bits 47:32, in 1/8192 V. `None` on CPUs that
/// leave the field zero.
pub fn intel_vid(perf_status: u64) -> Option<f64> {
    plausible_volts(((perf_status >> 32) & 0xFFFF) as f64 / 8192.0)
}

// --- AMD Zen (family 17h-1Ah) ---

pub const MSR_AMD_HARDWARE_PSTATE_STATUS: u32 = 0xC001_0293;
pub const MSR_AMD_PWR_UNIT: u32 = 0xC001_0299;
pub const MSR_AMD_CORE_ENERGY_STAT: u32 = 0xC001_029A;
pub const MSR_AMD_PKG_ENERGY_STAT: u32 = 0xC001_029B;
/// SMN address of the "current temperature" (Tctl) register.
pub const SMN_THM_TCON_CUR_TMP: u32 = 0x0005_9800;
/// How many CCD temperature registers to probe.
pub const MAX_CCDS: u32 = 8;

/// Tctl (°C) from `THM_TCON_CUR_TMP`: bits 31:21 in 1/8 °C, minus 49 when the range-select
/// flag (bit 19) is set.
pub fn amd_tctl(raw: u32) -> f64 {
    let t = (raw >> 21) as f64 * 0.125;
    if raw & 0x8_0000 != 0 { t - 49.0 } else { t }
}

/// Offset between Tctl and the real die temperature on early Zen parts (AMD's published
/// values); 0 on everything else.
pub fn amd_tctl_offset(brand: &str) -> f64 {
    const OFFSETS: &[(&str, f64)] = &[
        ("Ryzen 5 1600X", 20.0),
        ("Ryzen 7 1700X", 20.0),
        ("Ryzen 7 1800X", 20.0),
        ("Ryzen 7 2700X", 10.0),
        ("Threadripper 19", 27.0),
        ("Threadripper 29", 27.0),
    ];
    OFFSETS
        .iter()
        .find(|(m, _)| brand.contains(m))
        .map_or(0.0, |&(_, o)| o)
}

/// Zen 4 / Zen 5 parts moved the CCD temperature registers.
fn amd_zen4_or_later(family: u32, model: u32) -> bool {
    family >= 0x1A || (family == 0x19 && matches!(model, 0x10..=0x1F | 0x60..=0x7F | 0xA0..=0xAF))
}

/// SMN address of CCD `i`'s temperature register, or `None` before Zen 2 (no per-CCD sensors).
pub fn amd_ccd_temp_addr(family: u32, model: u32, i: u32) -> Option<u32> {
    let base = if amd_zen4_or_later(family, model) {
        0x0005_9B08
    } else if family == 0x19 || (family == 0x17 && model >= 0x30) {
        0x0005_9954
    } else {
        return None;
    };
    Some(base + i * 4)
}

/// CCD temperature (°C): bits 10:0 in 1/8 °C offset by -49, `None` unless the valid bit (11)
/// is set and the value is plausible (absent CCDs read as zero).
pub fn amd_ccd_temp(raw: u32) -> Option<f64> {
    if raw & 0x800 == 0 {
        return None;
    }
    let t = (raw & 0x7FF) as f64 * 0.125 - 49.0;
    (t > 0.0 && t < 125.0).then_some(t)
}

/// Whether the core VID uses the SVI2 encoding (Zen 1-3); Zen 4+ use SVI3, decoded differently.
pub fn amd_uses_svi2(family: u32, model: u32) -> bool {
    (family == 0x17 || family == 0x19) && !amd_zen4_or_later(family, model)
}

/// SVI2 core VID from `HARDWARE_PSTATE_STATUS` bits 21:14: 1.55 V - 6.25 mV per step.
pub fn amd_svi2_vid(pstate_status: u64) -> Option<f64> {
    plausible_volts(1.55 - ((pstate_status >> 14) & 0xFF) as f64 * 0.00625)
}

/// Joules per energy-counter tick from `PWR_UNIT` bits 12:8 (1 / 2^ESU).
pub fn amd_energy_unit(pwr_unit: u64) -> f64 {
    1.0 / (1u64 << ((pwr_unit >> 8) & 0x1F)) as f64
}

/// Ticks between two readings of a 32-bit energy counter, allowing for one wrap.
pub fn energy_delta(prev: u64, cur: u64) -> u64 {
    (cur.wrapping_sub(prev)) & 0xFFFF_FFFF
}

fn plausible_volts(v: f64) -> Option<f64> {
    (v > 0.2 && v < 2.0).then_some(v)
}

// --- Super I/O ---

/// Register-layout family of a supported Super I/O chip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChipKind {
    /// Nuvoton NCT6779D and later: banked registers, fan RPM at 0x4C0.
    Nct67xx { fans: usize },
    /// Nuvoton NCT6686D / NCT6687D (most MSI boards): EC-space pages.
    Nct6687,
    /// ITE IT86xx / IT87xx (most Gigabyte and many ASRock boards), 16-bit fan counters.
    Ite,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chip {
    pub name: String,
    pub kind: ChipKind,
}

/// Identify a Nuvoton chip from config registers 0x20 (id) and 0x21 (revision).
pub fn identify_nuvoton(id: u8, rev: u8) -> Option<Chip> {
    let (name, kind) = match (id, rev) {
        (0xC5, r) if r & 0xF0 == 0x60 => ("NCT6779D", ChipKind::Nct67xx { fans: 5 }),
        (0xC8, _) => ("NCT6791D", ChipKind::Nct67xx { fans: 6 }),
        (0xC9, _) => ("NCT6792D", ChipKind::Nct67xx { fans: 6 }),
        (0xD1, _) => ("NCT6793D", ChipKind::Nct67xx { fans: 6 }),
        (0xD3, _) => ("NCT6795D", ChipKind::Nct67xx { fans: 6 }),
        (0xD4, 0x23 | 0x2A) => ("NCT6796D", ChipKind::Nct67xx { fans: 6 }),
        (0xD4, 0x51) => ("NCT6797D", ChipKind::Nct67xx { fans: 7 }),
        (0xD4, 0x2B) => ("NCT6798D", ChipKind::Nct67xx { fans: 7 }),
        (0xD4, 0x40 | 0x41) => ("NCT6686D", ChipKind::Nct6687),
        (0xD5, 0x92) => ("NCT6687D", ChipKind::Nct6687),
        (0xD8, _) => ("NCT6799D", ChipKind::Nct67xx { fans: 7 }),
        _ => return None,
    };
    Some(Chip {
        name: format!("Nuvoton {name}"),
        kind,
    })
}

/// Identify an ITE chip from config registers 0x20/0x21 (e.g. 0x86 0x88 = IT8688E).
pub fn identify_ite(id: u8, rev: u8) -> Option<Chip> {
    let code = u16::from(id) << 8 | u16::from(rev);
    let suffix = match code {
        0x8613 | 0x8620 | 0x8625 | 0x8628 | 0x8631 | 0x8655 | 0x8665 | 0x8686 | 0x8688 | 0x8689
        | 0x8695 | 0x8696 | 0x8721 | 0x8728 | 0x8771 | 0x8772 | 0x8790 | 0x8792 => "E",
        0x8716 | 0x8718 | 0x8720 | 0x8726 => "F",
        _ => return None,
    };
    Some(Chip {
        name: format!("ITE IT{code:04X}{suffix}"),
        kind: ChipKind::Ite,
    })
}

/// Fan header labels on MSI boards with an NCT6687D (same order as MSI Center).
pub const NCT6687_FANS: [&str; 8] = [
    "CPU Fan",
    "Pump Fan",
    "System Fan 1",
    "System Fan 2",
    "System Fan 3",
    "System Fan 4",
    "System Fan 5",
    "System Fan 6",
];

/// Temperature inputs of the NCT6687D on MSI boards.
pub const NCT6687_TEMPS: [&str; 7] = [
    "CPU",
    "System",
    "VRM MOS",
    "PCH",
    "CPU Socket",
    "PCIe x1",
    "M.2",
];

/// NCT6687 voltage inputs: (register, label, divider scale, plausible range). Only rails
/// whose meaning is clear from their value; a reading outside its range means the board wires
/// that input differently, so it is left out rather than mislabelled.
pub const NCT6687_VOLTS: &[(u16, &str, f64, std::ops::RangeInclusive<f64>)] = &[
    (0x120, "+12V", 12.0, 10.2..=13.8),
    (0x122, "+5V", 5.0, 4.25..=5.75),
    (0x130, "+3.3V", 1.0, 2.8..=3.8),
    (0x124, "CPU Vcore", 1.0, 0.5..=1.7),
    (0x128, "DRAM", 2.0, 0.9..=1.7),
];

/// NCT6687 voltage register in volts: 1 mV steps, high byte x 16 plus the low byte's top nibble.
pub fn nct6687_volts(hi: u8, lo: u8) -> f64 {
    f64::from(u16::from(hi) * 16 + u16::from(lo >> 4)) / 1000.0
}

/// NCT6687 temperature: whole degrees (signed) in the high byte, half a degree in bit 7 of
/// the low byte. `None` for unconnected inputs.
pub fn nct6687_temp(hi: u8, lo: u8) -> Option<f64> {
    let t = f64::from(hi as i8) + if lo & 0x80 != 0 { 0.5 } else { 0.0 };
    plausible_temp(t)
}

/// ITE fan RPM from its 16-bit tachometer count (`None` when no fan: count 0 or saturated).
pub fn ite_fan_rpm(count: u16) -> Option<f64> {
    (count > 0x3F && count < 0xFFFF).then(|| 1_350_000.0 / (f64::from(count) * 2.0))
}

/// ITE temperature register: signed whole degrees; unconnected inputs read -128 or 127.
pub fn ite_temp(raw: u8) -> Option<f64> {
    plausible_temp(f64::from(raw as i8))
}

fn plausible_temp(t: f64) -> Option<f64> {
    (t > 0.0 && t < 125.0).then_some(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intel_temperatures() {
        // TjMax 100 °C; readout 35 below it, valid.
        let target = 100 << 16;
        let tj = intel_tjmax(target).unwrap();
        assert_eq!(intel_core_temp(0x8823_0000, tj), Some(65.0));
        assert_eq!(intel_core_temp(0x0823_0000, tj), None);
        assert_eq!(intel_package_temp(0x0828_0000, tj), 60.0);
        assert_eq!(intel_tjmax(0), None);
    }

    #[test]
    fn intel_voltage() {
        // 0x2666 / 8192 = 1.2 V.
        let v = intel_vid(0x2666_u64 << 32 | 0x2A00).unwrap();
        assert!((v - 1.2).abs() < 0.001);
        assert_eq!(intel_vid(0x2A00), None);
    }

    #[test]
    fn amd_temperatures() {
        // 0x2D0 eighths = 90 °C; with the range flag it is 41 °C.
        assert_eq!(amd_tctl(0x2D0 << 21), 90.0);
        assert_eq!(amd_tctl(0x2D0 << 21 | 0x8_0000), 41.0);
        assert_eq!(
            amd_tctl_offset("AMD Ryzen 7 1800X Eight-Core Processor"),
            20.0
        );
        assert_eq!(amd_tctl_offset("AMD Ryzen 7 7800X3D 8-Core Processor"), 0.0);
        // (0x2F0 / 8) - 49 = 45 °C.
        assert_eq!(amd_ccd_temp(0x800 | 0x2F0), Some(45.0));
        assert_eq!(amd_ccd_temp(0x2F0), None);
        assert_eq!(amd_ccd_temp(0x800), None);
    }

    #[test]
    fn amd_register_layouts() {
        // Zen 3 (Vermeer), Zen 4 (Raphael), Zen 5 (Granite Ridge), Zen+ (Pinnacle Ridge).
        assert_eq!(amd_ccd_temp_addr(0x19, 0x21, 1), Some(0x5_9958));
        assert_eq!(amd_ccd_temp_addr(0x19, 0x61, 0), Some(0x5_9B08));
        assert_eq!(amd_ccd_temp_addr(0x1A, 0x44, 0), Some(0x5_9B08));
        assert_eq!(amd_ccd_temp_addr(0x17, 0x08, 0), None);
        assert!(amd_uses_svi2(0x19, 0x21));
        assert!(!amd_uses_svi2(0x19, 0x61));
        assert!(!amd_uses_svi2(0x1A, 0x44));
    }

    #[test]
    fn amd_vid_and_energy() {
        // VID 0x30 = 1.55 - 48 * 6.25 mV = 1.25 V.
        let v = amd_svi2_vid(0x30 << 14).unwrap();
        assert!((v - 1.25).abs() < 1e-9);
        // ESU 16 -> 15.3 µJ per tick.
        assert_eq!(amd_energy_unit(0x000A_1003), 1.0 / 65536.0);
        assert_eq!(energy_delta(0xFFFF_FFF0, 0x10), 0x20);
        assert_eq!(energy_delta(100, 150), 50);
    }

    #[test]
    fn super_io_identification() {
        assert_eq!(
            identify_nuvoton(0xD5, 0x92).unwrap().kind,
            ChipKind::Nct6687
        );
        assert_eq!(
            identify_nuvoton(0xD4, 0x2B).unwrap().name,
            "Nuvoton NCT6798D"
        );
        assert_eq!(identify_nuvoton(0xFF, 0xFF), None);
        assert_eq!(identify_ite(0x86, 0x88).unwrap().name, "ITE IT8688E");
        assert_eq!(identify_ite(0x87, 0x20).unwrap().name, "ITE IT8720F");
        assert_eq!(identify_ite(0x12, 0x34), None);
    }

    #[test]
    fn super_io_values() {
        assert_eq!(nct6687_temp(45, 0x80), Some(45.5));
        assert_eq!(nct6687_temp(0x80, 0), None);
        // Readings from an MSI Z790 board: +12V rail and DDR5 at 1.35 V.
        assert!((nct6687_volts(0x3F, 0xA0) * 12.0 - 12.216).abs() < 1e-9);
        assert!((nct6687_volts(0x2A, 0x40) * 2.0 - 1.352).abs() < 1e-9);
        assert_eq!(ite_fan_rpm(675), Some(1000.0));
        assert_eq!(ite_fan_rpm(0xFFFF), None);
        assert_eq!(ite_fan_rpm(0), None);
        assert_eq!(ite_temp(38), Some(38.0));
        assert_eq!(ite_temp(0x80), None);
    }
}
