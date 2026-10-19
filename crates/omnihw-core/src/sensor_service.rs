//! The Omni-HW sensor service: a small Windows service (the same exe, run by the service
//! manager) that alone talks to the PawnIO driver and hands the decoded readings to Omni-HW.
//!
//! PawnIO exposes dangerous interfaces (raw MSR, PCI and port access), so it only admits
//! administrators. Omni-HW itself stays unprivileged: the service runs as SYSTEM, accepts no
//! input at all, and to anyone who connects to its pipe it writes the current readings and
//! hangs up. This module is the platform-neutral part: names and the text format.

use crate::sensors::{Reading, Unit};

/// Bumped when the response format changes; the app offers to update an older service.
pub const PROTOCOL: u32 = 1;
pub const SERVICE_NAME: &str = "OmniHWSensors";
pub const SERVICE_DISPLAY_NAME: &str = "Omni-HW Sensor Service";
pub const PIPE_NAME: &str = r"\\.\pipe\OmniHW-Sensors";
/// Responses larger than this are rejected (a normal one is a few KB).
pub const MAX_RESPONSE: usize = 1 << 20;
const MAGIC: &str = "OMNIHW-SENSORS";

/// One snapshot from the service.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Response {
    pub protocol: u32,
    /// Omni-HW version the service runs.
    pub version: String,
    /// What it reads, e.g. "CPU temperatures and voltage", "Nuvoton NCT6687D".
    pub sources: Vec<String>,
    /// Problems setting up or polling the PawnIO backends.
    pub errors: Vec<String>,
    pub readings: Vec<Reading>,
}

/// What the app knows about the service.
#[derive(Debug, Clone, PartialEq)]
pub enum ServiceState {
    /// The service isn't installed.
    NotSetUp,
    /// Installed but its pipe doesn't answer (stopped, or still starting).
    NotRunning(String),
    /// Running an incompatible protocol (installed by another Omni-HW version).
    Outdated { version: String },
    Ready {
        version: String,
        sources: Vec<String>,
        errors: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ServiceStatus {
    pub state: ServiceState,
    /// Installed PawnIO driver version, e.g. "2.2.0" (`None` = not installed).
    pub pawnio_version: Option<String>,
}

/// Whether a sensor id comes from the sensor service (CPU registers and Super I/O); used to
/// clear its readings when the service is turned off or goes away.
pub fn is_service_reading(id: &str) -> bool {
    ["cpu/temp/", "cpu/volt/", "cpu/power/", "sio"]
        .iter()
        .any(|p| id.starts_with(p))
}

/// Tabs and line breaks would break the line format; labels never need them.
fn clean(s: &str) -> String {
    s.replace(['\t', '\r', '\n'], " ")
}

/// Serialize a response: a header line, then `source`, `error` and `r` (reading) lines.
pub fn encode(r: &Response) -> String {
    let mut out = format!("{MAGIC}\t{}\t{}\n", r.protocol, clean(&r.version));
    for s in &r.sources {
        out += &format!("source\t{}\n", clean(s));
    }
    for e in &r.errors {
        out += &format!("error\t{}\n", clean(e));
    }
    for x in &r.readings {
        out += &format!(
            "r\t{}\t{}\t{}\t{}\t{}\n",
            clean(&x.id),
            clean(&x.label),
            clean(&x.group),
            x.unit.code(),
            x.value
        );
    }
    out
}

/// Parse a response. A different protocol returns just the header (no readings), so the
/// caller can report the version; malformed lines are skipped.
pub fn decode(text: &str) -> Result<Response, String> {
    let mut lines = text.lines();
    let header: Vec<&str> = lines.next().unwrap_or_default().split('\t').collect();
    let [MAGIC, protocol, version] = header.as_slice() else {
        return Err("not an Omni-HW sensor service".into());
    };
    let mut r = Response {
        protocol: protocol.parse().map_err(|_| "bad protocol version")?,
        version: version.to_string(),
        ..Default::default()
    };
    if r.protocol != PROTOCOL {
        return Ok(r);
    }
    for line in lines {
        let f: Vec<&str> = line.split('\t').collect();
        match f.as_slice() {
            ["source", s] => r.sources.push(s.to_string()),
            ["error", e] => r.errors.push(e.to_string()),
            ["r", id, label, group, unit, value] => {
                if let (Some(unit), Ok(value)) = (Unit::from_code(unit), value.parse::<f64>()) {
                    r.readings.push(Reading {
                        id: id.to_string(),
                        label: label.to_string(),
                        group: group.to_string(),
                        unit,
                        value,
                    });
                }
            }
            _ => {}
        }
    }
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let r = Response {
            protocol: PROTOCOL,
            version: "0.4.0".into(),
            sources: vec!["Nuvoton NCT6687D".into()],
            errors: vec!["PawnIO CPU sensors:\tbad\nthing".into()],
            readings: vec![Reading {
                id: "cpu/temp/core0".into(),
                label: "P-core 0".into(),
                group: "CPU Temperatures".into(),
                unit: Unit::Celsius,
                value: 41.5,
            }],
        };
        let back = decode(&encode(&r)).unwrap();
        assert_eq!(back.readings, r.readings);
        assert_eq!(back.sources, r.sources);
        assert_eq!(back.errors, ["PawnIO CPU sensors: bad thing"]);
        assert_eq!(back.version, "0.4.0");
    }

    #[test]
    fn service_reading_ids() {
        assert!(is_service_reading("cpu/temp/core3"));
        assert!(is_service_reading("sio1/fan0"));
        assert!(!is_service_reading("cpu/clock/core3"));
        assert!(!is_service_reading("power/RAPL_Package0_PKG"));
    }

    #[test]
    fn other_protocols_and_garbage() {
        let newer = decode("OMNIHW-SENSORS\t99\t9.0.0\nr\tx\tx\tx\tc\t1\n").unwrap();
        assert_eq!((newer.protocol, newer.readings.len()), (99, 0));
        assert!(decode("hello").is_err());
        assert!(decode("").is_err());
        let skipped = decode("OMNIHW-SENSORS\t1\tv\nr\ta\tb\tc\tfurlongs\t1\nr\ta\n").unwrap();
        assert!(skipped.readings.is_empty());
    }
}
