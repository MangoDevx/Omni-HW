//! SMBIOS parser for the `RSMB` firmware table returned by `GetSystemFirmwareTable`.
//!
//! Decodes types 0 (BIOS), 1 (System), 2 (Baseboard), 4 (Processor),
//! 16 (Physical Memory Array) and 17 (Memory Device).

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Bios {
    pub vendor: String,
    pub version: String,
    pub release_date: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SystemInfo {
    pub manufacturer: String,
    pub product: String,
    pub version: String,
    pub family: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Board {
    pub manufacturer: String,
    pub product: String,
    pub version: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Processor {
    pub socket: String,
    pub manufacturer: String,
    pub version: String,
    /// External (bus) clock in MHz, 0 if unknown.
    pub external_clock_mhz: u16,
    pub max_speed_mhz: u16,
    pub current_speed_mhz: u16,
    pub core_count: u16,
    pub thread_count: u16,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemoryDevice {
    pub locator: String,
    pub bank_locator: String,
    /// 0 = empty slot.
    pub size_mb: u64,
    pub form_factor: &'static str,
    pub memory_type: &'static str,
    /// Rated speed in MT/s.
    pub speed_mts: u32,
    /// Configured speed in MT/s.
    pub configured_speed_mts: u32,
    pub manufacturer: String,
    pub part_number: String,
    pub serial: String,
    pub rank: u8,
    pub data_width: u16,
    /// Configured voltage in millivolts, 0 if unknown.
    pub configured_voltage_mv: u16,
}

impl MemoryDevice {
    pub fn is_populated(&self) -> bool {
        self.size_mb > 0
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Smbios {
    pub version: (u8, u8),
    pub bios: Option<Bios>,
    pub system: Option<SystemInfo>,
    pub board: Option<Board>,
    pub processors: Vec<Processor>,
    /// Sum of slot counts from all type 16 arrays.
    pub memory_slots: u32,
    pub memory: Vec<MemoryDevice>,
}

impl Smbios {
    pub fn populated_memory(&self) -> impl Iterator<Item = &MemoryDevice> {
        self.memory.iter().filter(|m| m.is_populated())
    }

    pub fn total_memory_mb(&self) -> u64 {
        self.populated_memory().map(|m| m.size_mb).sum()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    TooShort,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SMBIOS buffer too short")
    }
}

impl std::error::Error for ParseError {}

/// One raw structure: formatted area plus its string set.
struct Structure<'a> {
    kind: u8,
    formatted: &'a [u8],
    strings: Vec<&'a [u8]>,
}

impl Structure<'_> {
    fn byte(&self, o: usize) -> Option<u8> {
        self.formatted.get(o).copied()
    }
    fn word(&self, o: usize) -> Option<u16> {
        crate::le::u16_at(self.formatted, o)
    }
    fn dword(&self, o: usize) -> Option<u32> {
        crate::le::u32_at(self.formatted, o)
    }
    /// String referenced by the index byte at offset `o` (1-based; 0 = none).
    fn string(&self, o: usize) -> String {
        let idx = self.byte(o).unwrap_or(0) as usize;
        if idx == 0 {
            return String::new();
        }
        self.strings
            .get(idx - 1)
            .map(|s| String::from_utf8_lossy(s).trim().to_string())
            .unwrap_or_default()
    }
}

/// Split the table data into structures. Stops at type 127 or malformed data.
fn structures(mut data: &[u8]) -> Vec<Structure<'_>> {
    let mut out = Vec::new();
    while data.len() >= 4 {
        let kind = data[0];
        let len = data[1] as usize;
        if len < 4 || len > data.len() {
            break;
        }
        let formatted = &data[..len];
        // String set ends with a double NUL.
        let rest = &data[len..];
        let Some(end) = rest.windows(2).position(|w| w == [0, 0]) else {
            break;
        };
        let strings = if end == 0 {
            Vec::new()
        } else {
            rest[..end].split(|&b| b == 0).collect()
        };
        out.push(Structure {
            kind,
            formatted,
            strings,
        });
        data = &rest[end + 2..];
        if kind == 127 {
            break;
        }
    }
    out
}

fn memory_type_name(t: u8) -> &'static str {
    match t {
        0x0F => "SDRAM",
        0x12 => "DDR",
        0x13 => "DDR2",
        0x14 => "DDR2 FB-DIMM",
        0x18 => "DDR3",
        0x1A => "DDR4",
        0x1B => "LPDDR",
        0x1C => "LPDDR2",
        0x1D => "LPDDR3",
        0x1E => "LPDDR4",
        0x20 => "HBM",
        0x21 => "HBM2",
        0x22 => "DDR5",
        0x23 => "LPDDR5",
        0x24 => "HBM3",
        _ => "Unknown",
    }
}

fn form_factor_name(f: u8) -> &'static str {
    match f {
        0x08 => "SIMM",
        0x09 => "DIMM",
        0x0B => "Row of chips",
        0x0D => "SO-DIMM",
        0x0F => "FB-DIMM",
        0x10 => "Die",
        0x11 => "CAMM",
        _ => "Other",
    }
}

fn parse_memory_device(s: &Structure) -> MemoryDevice {
    let size_mb = match s.word(0x0C).unwrap_or(0) {
        0 | 0xFFFF => 0,
        0x7FFF => (s.dword(0x1C).unwrap_or(0) & 0x7FFF_FFFF) as u64,
        v if v & 0x8000 != 0 => ((v & 0x7FFF) as u64) / 1024,
        v => v as u64,
    };
    let speed = |short: usize, ext: usize| -> u32 {
        match s.word(short).unwrap_or(0) {
            0xFFFF => s.dword(ext).unwrap_or(0),
            v => v as u32,
        }
    };
    let data_width = match s.word(0x0A).unwrap_or(0) {
        0xFFFF => 0,
        v => v,
    };
    MemoryDevice {
        locator: s.string(0x10),
        bank_locator: s.string(0x11),
        size_mb,
        form_factor: form_factor_name(s.byte(0x0E).unwrap_or(0)),
        memory_type: memory_type_name(s.byte(0x12).unwrap_or(0)),
        speed_mts: speed(0x15, 0x54),
        configured_speed_mts: speed(0x20, 0x58),
        manufacturer: s.string(0x17),
        serial: s.string(0x18),
        part_number: s.string(0x1A),
        rank: s.byte(0x1B).unwrap_or(0) & 0x0F,
        data_width,
        configured_voltage_mv: s.word(0x26).unwrap_or(0),
    }
}

fn parse_processor(s: &Structure) -> Processor {
    let byte_or_word = |b: usize, w: usize| -> u16 {
        match s.byte(b).unwrap_or(0) {
            0xFF => s.word(w).unwrap_or(0xFF),
            v => v as u16,
        }
    };
    Processor {
        socket: s.string(0x04),
        manufacturer: s.string(0x07),
        version: s.string(0x10),
        external_clock_mhz: s.word(0x12).unwrap_or(0),
        max_speed_mhz: s.word(0x14).unwrap_or(0),
        current_speed_mhz: s.word(0x16).unwrap_or(0),
        core_count: byte_or_word(0x23, 0x2A),
        thread_count: byte_or_word(0x25, 0x2E),
    }
}

impl Smbios {
    /// Parse a `RawSMBIOSData` buffer (8-byte header followed by table data).
    pub fn parse_raw(buf: &[u8]) -> Result<Smbios, ParseError> {
        let Some(len) = crate::le::u32_at(buf, 4) else {
            return Err(ParseError::TooShort);
        };
        let version = (buf[1], buf[2]);
        let data = &buf[8..buf.len().min(8usize.saturating_add(len as usize))];
        let mut out = Smbios::parse_table(data);
        out.version = version;
        Ok(out)
    }

    /// Parse bare SMBIOS structure-table data.
    pub fn parse_table(data: &[u8]) -> Smbios {
        let mut out = Smbios::default();
        for s in structures(data) {
            match s.kind {
                0 => {
                    out.bios = Some(Bios {
                        vendor: s.string(0x04),
                        version: s.string(0x05),
                        release_date: s.string(0x08),
                    })
                }
                1 => {
                    out.system = Some(SystemInfo {
                        manufacturer: s.string(0x04),
                        product: s.string(0x05),
                        version: s.string(0x06),
                        family: s.string(0x1A),
                    })
                }
                2 if out.board.is_none() => {
                    out.board = Some(Board {
                        manufacturer: s.string(0x04),
                        product: s.string(0x05),
                        version: s.string(0x06),
                    })
                }
                4 => out.processors.push(parse_processor(&s)),
                16 => {
                    // Use = 3 means system memory; skip flash/video arrays.
                    if s.byte(0x05) == Some(3) {
                        out.memory_slots += s.word(0x0D).unwrap_or(0) as u32;
                    }
                }
                17 => out.memory.push(parse_memory_device(&s)),
                _ => {}
            }
        }
        out
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Build one structure: `formatted` must start with type/len/handle; len is patched.
    fn structure(kind: u8, mut formatted: Vec<u8>, strings: &[&str]) -> Vec<u8> {
        formatted[0] = kind;
        formatted[1] = formatted.len() as u8;
        let mut v = formatted;
        if strings.is_empty() {
            v.extend([0, 0]);
        } else {
            for s in strings {
                v.extend(s.as_bytes());
                v.push(0);
            }
            v.push(0);
        }
        v
    }

    fn put16(v: &mut [u8], o: usize, x: u16) {
        v[o..o + 2].copy_from_slice(&x.to_le_bytes());
    }
    fn put32(v: &mut [u8], o: usize, x: u32) {
        v[o..o + 4].copy_from_slice(&x.to_le_bytes());
    }

    fn dimm(locator: &str, size_mb: u16, part: &str) -> Vec<u8> {
        let mut f = vec![0u8; 0x5C];
        put16(&mut f, 0x08, 64);
        put16(&mut f, 0x0A, 64);
        put16(&mut f, 0x0C, size_mb);
        f[0x0E] = 0x09; // DIMM
        f[0x10] = 1; // locator
        f[0x11] = 2; // bank
        f[0x12] = 0x22; // DDR5
        put16(&mut f, 0x15, 4800);
        f[0x17] = 3; // manufacturer
        f[0x18] = 4; // serial
        f[0x1A] = if part.is_empty() { 0 } else { 5 }; // part number
        f[0x1B] = 0x02; // rank 2
        put16(&mut f, 0x20, 6000);
        put16(&mut f, 0x26, 1350);
        // SMBIOS strings are never empty; an absent string uses index 0.
        let strings = [locator, "BANK 0", "G Skill Intl", "00000000", part];
        structure(
            17,
            f,
            if part.is_empty() {
                &strings[..4]
            } else {
                &strings
            },
        )
    }

    pub fn sample_table() -> Vec<u8> {
        let mut t = Vec::new();
        let mut bios = vec![0u8; 0x18];
        bios[0x04] = 1;
        bios[0x05] = 2;
        bios[0x08] = 3;
        t.extend(structure(
            0,
            bios,
            &[
                "American Megatrends International, LLC.",
                "1663",
                "06/15/2024",
            ],
        ));

        let mut sys = vec![0u8; 0x1B];
        sys[0x04] = 1;
        sys[0x05] = 2;
        sys[0x06] = 3;
        sys[0x1A] = 4;
        t.extend(structure(
            1,
            sys,
            &[
                "ASUS",
                "System Product Name",
                "System Version",
                "To be filled by O.E.M.",
            ],
        ));

        let mut board = vec![0u8; 0x0F];
        board[0x04] = 1;
        board[0x05] = 2;
        board[0x06] = 3;
        t.extend(structure(
            2,
            board,
            &[
                "ASUSTeK COMPUTER INC.",
                "ROG STRIX X670E-E GAMING WIFI",
                "Rev 1.xx",
            ],
        ));

        let mut cpu = vec![0u8; 0x30];
        cpu[0x04] = 1;
        cpu[0x07] = 2;
        cpu[0x10] = 3;
        put16(&mut cpu, 0x12, 100);
        put16(&mut cpu, 0x14, 5050);
        put16(&mut cpu, 0x16, 4200);
        cpu[0x23] = 8;
        cpu[0x25] = 16;
        t.extend(structure(
            4,
            cpu,
            &[
                "AM5",
                "Advanced Micro Devices, Inc.",
                "AMD Ryzen 7 7800X3D 8-Core Processor",
            ],
        ));

        let mut array = vec![0u8; 0x17];
        array[0x05] = 3;
        put16(&mut array, 0x0D, 4);
        t.extend(structure(16, array, &[]));

        t.extend(dimm("DIMM_A1", 0, "")); // empty slot
        t.extend(dimm("DIMM_A2", 16384, "F5-6000J3038F16G"));
        t.extend(dimm("DIMM_B1", 0, ""));
        // 64 GB module via the extended-size field.
        let mut big = dimm("DIMM_B2", 0x7FFF, "F5-6000J3038F64G");
        put32(&mut big, 0x1C, 65536);
        t.extend(big);

        t.extend(structure(127, vec![0u8; 4], &[]));
        // Garbage after end-of-table must be ignored.
        t.extend([0xDE, 0xAD]);
        t
    }

    #[test]
    fn parses_identity_structures() {
        let s = Smbios::parse_table(&sample_table());
        let bios = s.bios.unwrap();
        assert_eq!(bios.vendor, "American Megatrends International, LLC.");
        assert_eq!(bios.version, "1663");
        assert_eq!(bios.release_date, "06/15/2024");
        assert_eq!(s.board.unwrap().product, "ROG STRIX X670E-E GAMING WIFI");
        assert_eq!(s.system.unwrap().manufacturer, "ASUS");
        let p = &s.processors[0];
        assert_eq!(p.socket, "AM5");
        assert_eq!(
            (
                p.external_clock_mhz,
                p.max_speed_mhz,
                p.core_count,
                p.thread_count
            ),
            (100, 5050, 8, 16)
        );
    }

    #[test]
    fn parses_memory() {
        let s = Smbios::parse_table(&sample_table());
        assert_eq!(s.memory_slots, 4);
        assert_eq!(s.memory.len(), 4);
        assert_eq!(s.populated_memory().count(), 2);
        assert_eq!(s.total_memory_mb(), 16384 + 65536);
        let d = &s.memory[1];
        assert_eq!(d.locator, "DIMM_A2");
        assert_eq!(d.memory_type, "DDR5");
        assert_eq!(d.form_factor, "DIMM");
        assert_eq!((d.speed_mts, d.configured_speed_mts), (4800, 6000));
        assert_eq!(d.part_number, "F5-6000J3038F16G");
        assert_eq!(d.manufacturer, "G Skill Intl");
        assert_eq!(d.rank, 2);
        assert_eq!(d.configured_voltage_mv, 1350);
        assert_eq!(s.memory[3].size_mb, 65536);
        assert!(s.memory[0].part_number.is_empty());
    }

    #[test]
    fn raw_header() {
        let table = sample_table();
        let mut raw = vec![0, 3, 6, 0];
        raw.extend((table.len() as u32).to_le_bytes());
        raw.extend(&table);
        let s = Smbios::parse_raw(&raw).unwrap();
        assert_eq!(s.version, (3, 6));
        assert!(s.bios.is_some());
        assert!(Smbios::parse_raw(&[0; 4]).is_err());
    }

    #[test]
    fn truncated_tables_do_not_panic() {
        let t = sample_table();
        for cut in 0..t.len() {
            let _ = Smbios::parse_table(&t[..cut]);
        }
    }
}
