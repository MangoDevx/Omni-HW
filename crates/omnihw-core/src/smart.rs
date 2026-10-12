//! ATA SMART attribute parsing (SATA drives), from the 512-byte SMART READ DATA sector.

/// One SMART attribute entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SmartAttribute {
    pub id: u8,
    pub name: &'static str,
    /// Normalized value (vendor scale, usually 1-100 or 1-253; higher is better).
    pub current: u8,
    pub worst: u8,
    /// 48-bit raw value (vendor-specific meaning).
    pub raw: u64,
}

impl SmartAttribute {
    /// Low 32 bits of the raw value; most counters live here, with vendor data above.
    pub fn raw_low(&self) -> u64 {
        self.raw & 0xFFFF_FFFF
    }
}

/// SMART data for an ATA drive.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AtaSmart {
    pub attributes: Vec<SmartAttribute>,
}

/// Common attribute names. Several ids are vendor-specific; these are the usual meanings.
pub fn attribute_name(id: u8) -> &'static str {
    match id {
        1 => "Raw Read Error Rate",
        3 => "Spin-Up Time",
        4 => "Start/Stop Count",
        5 => "Reallocated Sectors Count",
        7 => "Seek Error Rate",
        9 => "Power-On Hours",
        10 => "Spin Retry Count",
        12 => "Power Cycle Count",
        170 => "Available Reserved Space",
        171 => "Program Fail Count",
        172 => "Erase Fail Count",
        173 => "Wear Leveling Count",
        174 => "Unexpected Power Loss Count",
        177 => "Wear Leveling Count",
        179 => "Used Reserved Block Count",
        181 => "Program Fail Count",
        182 => "Erase Fail Count",
        183 => "Runtime Bad Block",
        184 => "End-to-End Error",
        187 => "Reported Uncorrectable Errors",
        188 => "Command Timeout",
        190 => "Airflow Temperature",
        192 => "Power-off Retract Count",
        193 => "Load Cycle Count",
        194 => "Temperature",
        195 => "Hardware ECC Recovered",
        196 => "Reallocation Event Count",
        197 => "Current Pending Sector Count",
        198 => "Uncorrectable Sector Count",
        199 => "UltraDMA CRC Error Count",
        202 => "Percent Lifetime Remaining",
        231 => "SSD Life Left",
        232 => "Available Reserved Space",
        233 => "Media Wearout Indicator",
        235 => "Power Fail Backup Health",
        241 => "Total LBAs Written",
        242 => "Total LBAs Read",
        _ => "Vendor Specific",
    }
}

/// Parse SMART READ DATA: 30 entries of 12 bytes from offset 2 (id, flags u16, current,
/// worst, raw[6], reserved). Entries with id 0 are unused.
pub fn parse_ata_smart(b: &[u8]) -> Option<AtaSmart> {
    if b.len() < 2 + 30 * 12 {
        return None;
    }
    let attributes: Vec<SmartAttribute> = (0..30)
        .map(|i| &b[2 + i * 12..2 + (i + 1) * 12])
        .filter(|e| e[0] != 0)
        .map(|e| {
            let mut raw = [0u8; 8];
            raw[..6].copy_from_slice(&e[5..11]);
            SmartAttribute {
                id: e[0],
                name: attribute_name(e[0]),
                current: e[3],
                worst: e[4],
                raw: u64::from_le_bytes(raw),
            }
        })
        .collect();
    (!attributes.is_empty()).then_some(AtaSmart { attributes })
}

impl AtaSmart {
    pub fn attr(&self, id: u8) -> Option<&SmartAttribute> {
        self.attributes.iter().find(|a| a.id == id)
    }

    fn raw_low(&self, id: u8) -> Option<u64> {
        self.attr(id).map(SmartAttribute::raw_low)
    }

    /// Drive temperature in °C (194, else 190); the low byte of raw holds the current value.
    pub fn temperature_c(&self) -> Option<u32> {
        [194, 190]
            .iter()
            .find_map(|&id| self.attr(id))
            .map(|a| (a.raw & 0xFF) as u32)
            .filter(|&t| t > 0 && t < 120)
    }

    pub fn power_on_hours(&self) -> Option<u64> {
        self.raw_low(9)
    }

    pub fn power_cycles(&self) -> Option<u64> {
        self.raw_low(12)
    }

    pub fn reallocated_sectors(&self) -> Option<u64> {
        self.raw_low(5)
    }

    pub fn pending_sectors(&self) -> Option<u64> {
        self.raw_low(197)
    }

    /// Uncorrectable errors: the larger of offline-uncorrectable (198) and reported (187).
    pub fn uncorrectable(&self) -> Option<u64> {
        match (self.raw_low(198), self.raw_low(187)) {
            (None, None) => None,
            (a, b) => Some(a.unwrap_or(0).max(b.unwrap_or(0))),
        }
    }

    pub fn crc_errors(&self) -> Option<u64> {
        self.raw_low(199)
    }

    /// Remaining SSD life in percent from the normalized value of a recognized wear
    /// attribute: 231 SSD Life Left, 233 Media Wearout (Intel), 202 Percent Lifetime
    /// Remaining (Crucial/Micron), 177 Wear Leveling Count (Samsung), 173 Wear Leveling.
    pub fn life_remaining_pct(&self) -> Option<u8> {
        [231, 233, 202, 177, 173]
            .iter()
            .find_map(|&id| self.attr(id))
            .map(|a| a.current.min(100))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Build a SMART data sector from (id, current, worst, raw) entries.
    pub fn sector(entries: &[(u8, u8, u8, u64)]) -> Vec<u8> {
        let mut b = vec![0u8; 512];
        b[0] = 0x10;
        for (i, &(id, cur, worst, raw)) in entries.iter().enumerate() {
            let o = 2 + i * 12;
            b[o] = id;
            b[o + 1] = 0x33;
            b[o + 3] = cur;
            b[o + 4] = worst;
            b[o + 5..o + 11].copy_from_slice(&raw.to_le_bytes()[..6]);
        }
        b
    }

    #[test]
    fn parses_attributes() {
        let s = parse_ata_smart(&sector(&[
            (5, 100, 100, 3),
            (9, 95, 95, 12_345),
            (12, 99, 99, 850),
            (177, 92, 92, 120),
            (187, 100, 100, 0),
            (194, 64, 50, 0x0032_0012_0024), // current 36 °C, min/max packed above
            (197, 100, 100, 1),
            (198, 100, 100, 2),
            (199, 100, 100, 7),
            (241, 99, 99, 0x0000_1234_5678),
        ]))
        .unwrap();
        assert_eq!(s.attributes.len(), 10);
        assert_eq!(s.attr(5).unwrap().name, "Reallocated Sectors Count");
        assert_eq!(s.reallocated_sectors(), Some(3));
        assert_eq!(s.power_on_hours(), Some(12_345));
        assert_eq!(s.power_cycles(), Some(850));
        assert_eq!(s.temperature_c(), Some(36));
        assert_eq!(s.pending_sectors(), Some(1));
        assert_eq!(s.uncorrectable(), Some(2));
        assert_eq!(s.crc_errors(), Some(7));
        assert_eq!(s.life_remaining_pct(), Some(92));
        assert_eq!(s.attr(241).unwrap().raw, 0x1234_5678);
    }

    #[test]
    fn life_attribute_priority_and_absence() {
        let s = parse_ata_smart(&sector(&[(177, 80, 80, 0), (231, 15, 15, 0)])).unwrap();
        assert_eq!(s.life_remaining_pct(), Some(15));
        let hdd = parse_ata_smart(&sector(&[(5, 100, 100, 0), (9, 90, 90, 100)])).unwrap();
        assert_eq!(hdd.life_remaining_pct(), None);
        assert_eq!(hdd.uncorrectable(), None);
        assert_eq!(hdd.temperature_c(), None);
    }

    #[test]
    fn rejects_short_or_empty() {
        assert!(parse_ata_smart(&[0u8; 100]).is_none());
        assert!(parse_ata_smart(&[0u8; 512]).is_none());
    }
}
