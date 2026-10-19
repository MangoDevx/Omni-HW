//! WHEA-Logger hardware error events (System log, IDs 17, 18, 19 and 47): parsing the rendered
//! event XML into what the error was and where it came from.
//!
//! Field names are those of the provider's own event templates, read on Windows 11 with
//! `(Get-WinEvent -ListProvider Microsoft-Windows-WHEA-Logger).Events` (`.Template`):
//! - 17 (corrected PCIe error): `ErrorSource`, `PortType`, `Bus`, `Device`, `Function`,
//!   `VendorID`, `DeviceID`, and from template version 1 on `PrimaryDeviceName`.
//! - 18 (fatal) and 19 (corrected) processor errors: `ErrorSource`, `ApicId`, `MCABank`,
//!   `MciStat`, `ErrorType`, ...
//! - 47 (corrected memory error): `ErrorSource`, `Node`, `Card`, `Module`, `Bank`, ...
//!
//! Value meanings: `ErrorSource` is `WHEA_ERROR_SOURCE_TYPE` (Microsoft Learn, ntddk.h);
//! `PortType` is the PCIe Device/Port Type (`WHEA_PCIEXPRESS_DEVICE_TYPE`, as in the PCIe
//! capability register); the error class of a machine check comes from the MCA error code in
//! bits 15:0 of `MciStat` (Intel SDM Vol. 3B "Interpreting the MCA Error Codes", AMD APM
//! Vol. 2 "Machine-Check Error Codes"). The numeric `ErrorType` field has no public value
//! map, so it is not used. Hex-typed fields render as `0x..`, others as decimal; both are
//! accepted. Missing fields simply leave their value `None`.

use crate::checkup::{element, event_data};
use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// What kind of hardware reported the error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WheaKind {
    /// CPU cache (machine check, cache hierarchy error code).
    Cache,
    /// CPU translation lookaside buffer.
    Tlb,
    /// The memory controller.
    MemoryController,
    /// Bus or interconnect: memory or Infinity Fabric / ring bus, typical of unstable XMP/EXPO.
    Interconnect,
    /// CPU internal (timer, parity, microcode) errors.
    CpuInternal,
    /// A processor machine check whose error code isn't known.
    Cpu,
    /// A corrected memory error (event 47).
    Memory,
    /// A PCI Express error (event 17).
    Pcie,
}

impl WheaKind {
    /// Short lower-case name for a sentence.
    pub fn label(self) -> &'static str {
        match self {
            WheaKind::Cache => "CPU cache",
            WheaKind::Tlb => "CPU TLB",
            WheaKind::MemoryController => "memory controller",
            WheaKind::Interconnect => "bus/interconnect",
            WheaKind::CpuInternal => "CPU internal",
            WheaKind::Cpu => "processor",
            WheaKind::Memory => "memory",
            WheaKind::Pcie => "PCIe",
        }
    }
}

/// PCI address of the device that reported a PCIe error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PciAddr {
    pub bus: u32,
    pub device: u32,
    pub function: u32,
}

impl std::fmt::Display for PciAddr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:02x}:{:02x}.{:x}",
            self.bus, self.device, self.function
        )
    }
}

/// One WHEA-Logger event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WheaEvent {
    /// Event log record number (unique within the System log).
    pub record_id: u64,
    /// Event ID: 17, 18, 19 or 47.
    pub id: u32,
    /// When it was logged.
    pub time: Option<SystemTime>,
    /// Fatal (uncorrected, event 18) rather than corrected.
    pub fatal: bool,
    pub kind: WheaKind,
    /// Error source, e.g. "Corrected Machine Check".
    pub source: String,
    /// APIC ID of the processor that reported a machine check.
    pub apic_id: Option<u32>,
    /// Machine check bank.
    pub mca_bank: Option<u32>,
    /// Reporting PCIe device (event 17).
    pub pcie: Option<PciAddr>,
    /// PCIe component, e.g. "PCI Express Root Port", or the device's hardware ID when logged.
    pub device: Option<String>,
    /// One-line description, e.g. "Corrected CPU cache error (CPU APIC ID 5, bank 1)".
    pub summary: String,
}

impl WheaEvent {
    /// Where the error came from, when the event says: "CPU APIC ID 5" or "PCIe device 01:00.0".
    pub fn location(&self) -> Option<String> {
        if let Some(a) = self.apic_id {
            Some(format!("CPU APIC ID {a}"))
        } else {
            self.pcie.map(|p| format!("PCIe device {p}"))
        }
    }
}

/// `WHEA_ERROR_SOURCE_TYPE` names, in enum order.
const SOURCES: [&str; 19] = [
    "Machine Check Exception",
    "Corrected Machine Check",
    "Corrected Platform Error",
    "Non-Maskable Interrupt",
    "PCI Express",
    "Generic",
    "INIT",
    "Boot",
    "SCI Generic",
    "IPF MCA",
    "IPF CMC",
    "IPF CPE",
    "Generic V2",
    "SCI Generic V2",
    "BMC",
    "PMEM",
    "Device Driver",
    "SEA",
    "SEI",
];

/// PCIe Device/Port Type names (`WHEA_PCIEXPRESS_DEVICE_TYPE`).
fn port_type(t: u32) -> Option<&'static str> {
    Some(match t {
        0 => "PCI Express Endpoint",
        1 => "PCI Express Legacy Endpoint",
        4 => "PCI Express Root Port",
        5 => "PCI Express Upstream Switch Port",
        6 => "PCI Express Downstream Switch Port",
        7 => "PCI Express to PCI/PCI-X Bridge",
        8 => "PCI/PCI-X to PCI Express Bridge",
        9 => "Root Complex Integrated Endpoint",
        10 => "Root Complex Event Collector",
        _ => return None,
    })
}

/// Class of a machine check from the MCA error code (bits 15:0 of MCi_STATUS). Bit 12 is the
/// "filtered" flag and is ignored.
fn mca_kind(status: u64) -> WheaKind {
    let code = (status & 0xEFFF) as u16;
    if code & 0xE800 == 0x0800 {
        WheaKind::Interconnect
    } else if code & 0xEF00 == 0x0100 {
        WheaKind::Cache
    } else if code & 0xEF80 == 0x0080 {
        WheaKind::MemoryController
    } else if code & 0xEFF0 == 0x0010 {
        WheaKind::Tlb
    } else if code & 0xFC00 == 0x0400 || matches!(code, 2 | 5) {
        WheaKind::CpuInternal
    } else {
        WheaKind::Cpu
    }
}

/// "0x1b" or "27".
fn number(s: &str) -> Option<u64> {
    let s = s.trim();
    match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        Some(hex) => u64::from_str_radix(hex, 16).ok(),
        None => s.parse().ok(),
    }
}

fn field(xml: &str, name: &str) -> Option<u64> {
    event_data(xml, name).as_deref().and_then(number)
}

fn field32(xml: &str, name: &str) -> Option<u32> {
    field(xml, name).and_then(|v| u32::try_from(v).ok())
}

/// Days since 1970-01-01 for a Gregorian date (Howard Hinnant's days_from_civil).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = i64::from((m + 9) % 12);
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// `SystemTime='2026-10-19T12:34:56.1234567Z'` (UTC), to the second.
fn time_created(xml: &str) -> Option<SystemTime> {
    let t = xml.find("SystemTime=")? + "SystemTime=".len() + 1;
    let s = xml.get(t..t + 19)?;
    let n = |r: std::ops::Range<usize>| s.get(r)?.parse::<u32>().ok();
    let days = days_from_civil(i64::from(n(0..4)?), n(5..7)?, n(8..10)?);
    let secs = days * 86_400 + i64::from(n(11..13)? * 3600 + n(14..16)? * 60 + n(17..19)?);
    Some(UNIX_EPOCH + Duration::from_secs(u64::try_from(secs).ok()?))
}

/// Parse a WHEA-Logger event rendered as XML (`EvtRender`). `None` for other event IDs.
pub fn parse_whea_xml(xml: &str) -> Option<WheaEvent> {
    let id: u32 = element(xml, "EventID")?.trim().parse().ok()?;
    let fatal = id == 18;
    let source = field(xml, "ErrorSource")
        .and_then(|s| SOURCES.get(usize::try_from(s).ok()?))
        .map_or_else(String::new, |s| (*s).to_string());
    let (mut apic_id, mut mca_bank, mut pcie, mut device) = (None, None, None, None);
    let kind = match id {
        18 | 19 => {
            apic_id = field32(xml, "ApicId");
            mca_bank = field32(xml, "MCABank");
            field(xml, "MciStat").map_or(WheaKind::Cpu, mca_kind)
        }
        17 => {
            let bdf = (
                field32(xml, "Bus"),
                field32(xml, "Device"),
                field32(xml, "Function"),
            );
            if let (Some(bus), Some(device), Some(function)) = bdf {
                pcie = Some(PciAddr {
                    bus,
                    device,
                    function,
                });
            }
            device = event_data(xml, "PrimaryDeviceName")
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .or_else(|| {
                    field32(xml, "PortType")
                        .and_then(port_type)
                        .map(str::to_string)
                });
            WheaKind::Pcie
        }
        47 => WheaKind::Memory,
        _ => return None,
    };
    let mut at = Vec::new();
    if let Some(a) = apic_id {
        at.push(format!("CPU APIC ID {a}"));
    }
    if let Some(b) = mca_bank {
        at.push(format!("bank {b}"));
    }
    if let Some(p) = pcie {
        at.push(format!("device {p}"));
    }
    if let Some(d) = &device {
        at.push(d.clone());
    }
    let mut summary = format!(
        "{} {} error",
        if fatal { "Fatal" } else { "Corrected" },
        kind.label()
    );
    if !at.is_empty() {
        summary = format!("{summary} ({})", at.join(", "));
    }
    Some(WheaEvent {
        record_id: element(xml, "EventRecordID")
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0),
        id,
        time: time_created(xml),
        fatal,
        kind,
        source,
        apic_id,
        mca_bank,
        pcie,
        device,
        summary,
    })
}

/// The location most events came from, with its count (ties: the first seen).
pub fn most_common_location(events: &[WheaEvent]) -> Option<(String, usize)> {
    let mut counts: HashMap<String, (usize, usize)> = HashMap::new();
    for (i, loc) in events.iter().filter_map(|e| e.location()).enumerate() {
        counts.entry(loc).or_insert((0, i)).0 += 1;
    }
    counts
        .into_iter()
        .max_by_key(|(_, (n, first))| (*n, std::cmp::Reverse(*first)))
        .map(|(loc, (n, _))| (loc, n))
}

/// Up to `max` distinct locations and kinds, most frequent location first:
/// "CPU APIC ID 5, bus/interconnect".
pub fn where_text(events: &[WheaEvent], max: usize) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some((loc, _)) = most_common_location(events) {
        parts.push(loc);
    }
    for e in events {
        for p in [e.location(), Some(e.kind.label().to_string())]
            .into_iter()
            .flatten()
        {
            if !parts.contains(&p) {
                parts.push(p);
            }
        }
    }
    parts.truncate(max);
    parts.join(", ")
}

/// Warning for a test during which WHEA errors were logged, or `None` when there were none.
pub fn during_test_text(events: &[WheaEvent]) -> Option<String> {
    let n = events.len();
    if n == 0 {
        return None;
    }
    Some(format!(
        "{n} hardware error{} (WHEA) {} logged during this test: results may be unreliable and \
         the system may be unstable ({})",
        if n == 1 { "" } else { "s" },
        if n == 1 { "was" } else { "were" },
        where_text(events, 4)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wrap(id: u32, data: &str) -> String {
        format!(
            "<Event xmlns='http://schemas.microsoft.com/win/2004/08/events/event'><System>\
             <Provider Name='Microsoft-Windows-WHEA-Logger' Guid='{{c26c4f3c-3f66-4e99-8f8a-39405cfed220}}'/>\
             <EventID>{id}</EventID><Version>0</Version><Level>3</Level>\
             <TimeCreated SystemTime='2026-10-19T12:34:56.1234567Z'/>\
             <EventRecordID>48213</EventRecordID><Channel>System</Channel></System>\
             <EventData>{data}</EventData></Event>"
        )
    }

    const MCE19: &str = "<Data Name='ErrorSource'>1</Data><Data Name='ApicId'>5</Data>\
        <Data Name='MCABank'>0x1b</Data><Data Name='MciStat'>0x9c20000000010c0f</Data>\
        <Data Name='MciAddr'>0x0</Data><Data Name='MciMisc'>0xd01a0fff00000000</Data>\
        <Data Name='ErrorType'>12</Data><Data Name='TransactionType'>256</Data>\
        <Data Name='Participation'>0</Data><Data Name='RequestType'>0</Data>\
        <Data Name='MemorIO'>0</Data><Data Name='MemHierarchyLvl'>3</Data>\
        <Data Name='Timeout'>0</Data><Data Name='OperationType'>0</Data>\
        <Data Name='Channel'>0</Data><Data Name='Length'>936</Data><Data Name='RawData'>435045</Data>";

    #[test]
    fn corrected_machine_check() {
        let e = parse_whea_xml(&wrap(19, MCE19)).unwrap();
        assert_eq!(e.id, 19);
        assert_eq!(e.record_id, 48213);
        assert!(!e.fatal);
        assert_eq!(e.kind, WheaKind::Interconnect);
        assert_eq!(e.source, "Corrected Machine Check");
        assert_eq!(e.apic_id, Some(5));
        assert_eq!(e.mca_bank, Some(27));
        assert_eq!(e.pcie, None);
        assert_eq!(e.location().as_deref(), Some("CPU APIC ID 5"));
        assert_eq!(
            e.summary,
            "Corrected bus/interconnect error (CPU APIC ID 5, bank 27)"
        );
        let t = e
            .time
            .unwrap()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        // 2026-10-19 12:34:56 UTC.
        assert_eq!(t, 20_745 * 86_400 + 12 * 3600 + 34 * 60 + 56);
        assert_eq!(crate::civil_from_days(20_745), (2026, 10, 19));
    }

    #[test]
    fn fatal_cache_error() {
        let data = "<Data Name='ErrorSource'>0</Data><Data Name='ApicId'>14</Data>\
            <Data Name='MCABank'>1</Data><Data Name='MciStat'>0xbea0000000000108</Data>";
        let e = parse_whea_xml(&wrap(18, data)).unwrap();
        assert!(e.fatal);
        assert_eq!(e.kind, WheaKind::Cache);
        assert_eq!(e.source, "Machine Check Exception");
        assert_eq!((e.apic_id, e.mca_bank), (Some(14), Some(1)));
        assert!(e.summary.starts_with("Fatal CPU cache error"));
    }

    #[test]
    fn mca_codes() {
        assert_eq!(mca_kind(0x0E0B), WheaKind::Interconnect);
        assert_eq!(mca_kind(0x1E0B), WheaKind::Interconnect);
        assert_eq!(mca_kind(0x0150), WheaKind::Cache);
        assert_eq!(mca_kind(0x009F), WheaKind::MemoryController);
        assert_eq!(mca_kind(0x0011), WheaKind::Tlb);
        assert_eq!(mca_kind(0x0400), WheaKind::CpuInternal);
        assert_eq!(mca_kind(0x0005), WheaKind::CpuInternal);
        assert_eq!(mca_kind(0), WheaKind::Cpu);
    }

    #[test]
    fn pcie_error() {
        let data = "<Data Name='ErrorSource'>4</Data>\
            <Data Name='FRUId'>{00000000-0000-0000-0000-000000000000}</Data><Data Name='FRUText'></Data>\
            <Data Name='ValidBits'>0x3</Data><Data Name='PortType'>4</Data>\
            <Data Name='Version'>0x101</Data><Data Name='Command'>0x407</Data>\
            <Data Name='Status'>0x10</Data><Data Name='Bus'>0x0</Data><Data Name='Device'>0x1</Data>\
            <Data Name='Function'>0x1</Data><Data Name='Segment'>0x0</Data>\
            <Data Name='SecondaryBus'>0x1</Data><Data Name='SecondaryDevice'>0x0</Data>\
            <Data Name='SecondaryFunction'>0x0</Data><Data Name='VendorID'>0x1022</Data>\
            <Data Name='DeviceID'>0x1483</Data><Data Name='ClassCode'>0x60400</Data>\
            <Data Name='CorrectableErrorStatus'>0x1000</Data>\
            <Data Name='PrimaryDeviceName'>PCI\\VEN_1022&amp;DEV_1483&amp;SUBSYS_14531022&amp;REV_00</Data>\
            <Data Name='SecondaryDeviceName'></Data>";
        let e = parse_whea_xml(&wrap(17, data)).unwrap();
        assert_eq!(e.kind, WheaKind::Pcie);
        assert_eq!(e.source, "PCI Express");
        assert_eq!(e.apic_id, None);
        assert_eq!(e.pcie.unwrap().to_string(), "00:01.1");
        assert_eq!(
            e.device.as_deref(),
            Some("PCI\\VEN_1022&DEV_1483&SUBSYS_14531022&REV_00")
        );
        assert_eq!(e.location().as_deref(), Some("PCIe device 00:01.1"));

        // Version 0 template: no device name, so the port type names it.
        let old = data.replace("PrimaryDeviceName", "Other");
        let e = parse_whea_xml(&wrap(17, &old)).unwrap();
        assert_eq!(e.device.as_deref(), Some("PCI Express Root Port"));
    }

    #[test]
    fn missing_fields() {
        let e = parse_whea_xml(&wrap(19, "")).unwrap();
        assert_eq!(e.kind, WheaKind::Cpu);
        assert_eq!((e.apic_id, e.mca_bank, e.pcie), (None, None, None));
        assert_eq!(e.source, "");
        assert_eq!(e.location(), None);
        assert_eq!(e.summary, "Corrected processor error");

        let e = parse_whea_xml(&wrap(17, "<Data Name='Bus'>0x1</Data>")).unwrap();
        assert_eq!((e.pcie, e.device), (None, None));

        let e = parse_whea_xml(&wrap(47, "<Data Name='ErrorSource'>2</Data>")).unwrap();
        assert_eq!(e.kind, WheaKind::Memory);
        assert_eq!(e.source, "Corrected Platform Error");

        assert!(parse_whea_xml(&wrap(41, "")).is_none());
        assert!(parse_whea_xml("<Event/>").is_none());
        let no_time = wrap(19, MCE19).replace("SystemTime=", "Other=");
        assert_eq!(parse_whea_xml(&no_time).unwrap().time, None);
    }

    #[test]
    fn test_warning() {
        assert_eq!(during_test_text(&[]), None);
        let cpu = parse_whea_xml(&wrap(19, MCE19)).unwrap();
        let mut other = cpu.clone();
        other.apic_id = Some(2);
        let events = [cpu.clone(), other, cpu];
        assert_eq!(
            most_common_location(&events),
            Some(("CPU APIC ID 5".to_string(), 2))
        );
        assert_eq!(
            during_test_text(&events).unwrap(),
            "3 hardware errors (WHEA) were logged during this test: results may be unreliable \
             and the system may be unstable (CPU APIC ID 5, bus/interconnect, CPU APIC ID 2)"
        );
        assert!(
            during_test_text(&events[..1])
                .unwrap()
                .starts_with("1 hardware error (WHEA) was")
        );
    }
}
