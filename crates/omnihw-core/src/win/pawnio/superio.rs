//! Motherboard fans, temperatures and voltages from the Super I/O chip, through the LpcIO PawnIO
//! module (which only allows the chip's config ports and the I/O ranges it decodes).
//!
//! Read-only: the only port writes are the ones every read of these chips needs (the config-mode
//! enter/exit keys, the logical-device select, and the index / bank / page registers that pick
//! which register the next read returns). No chip setting is ever written.

use super::{GlobalMutex, ISA_BUS_MUTEX, LPC_IO, Module, locked};
use crate::lowlevel::*;
use crate::sensors::{Reading, SensorBackend, Unit};

/// Config-port pairs the module can select (slot 0 and 1).
const PORTS: [u16; 2] = [0x2E, 0x4E];
const FANS: &str = "Motherboard Fans";
const TEMPS: &str = "Motherboard Temperatures";
const VOLTS: &str = "Motherboard Voltages";

struct Port<'a>(&'a Module);

impl Port<'_> {
    fn outb(&self, port: u16, v: u8) -> Result<(), String> {
        self.0
            .call("ioctl_pio_outb", &[port.into(), v.into()], &mut [])
            .map(drop)
    }
    fn inb(&self, port: u16) -> Result<u8, String> {
        self.0.call1("ioctl_pio_inb", port.into()).map(|v| v as u8)
    }
    /// Read a Super I/O config register (config mode must be entered).
    fn sio_in(&self, reg: u8) -> Result<u8, String> {
        self.0
            .call1("ioctl_superio_inb", reg.into())
            .map(|v| v as u8)
    }
    fn sio_out(&self, reg: u8, v: u8) -> Result<(), String> {
        self.0
            .call("ioctl_superio_outb", &[reg.into(), v.into()], &mut [])
            .map(drop)
    }
}

struct Device {
    module: Module,
    chip: Chip,
    /// Hardware-monitor I/O base.
    base: u16,
    /// Sensor id prefix ("sio0").
    prefix: String,
    /// Fan headers that have reported a speed; empty headers (always 0) stay hidden.
    fans_seen: Vec<bool>,
}

/// Fans, temperatures and voltages of the motherboard's Super I/O chip(s).
pub struct SuperIoBackend {
    devices: Vec<Device>,
    isa: Option<GlobalMutex>,
}

/// Look for a supported chip on config port `slot`; on success returns it with its
/// hardware-monitor base, leaving config mode again either way.
fn detect(m: &Module, slot: usize) -> Result<Option<(Chip, u16)>, String> {
    m.call("ioctl_select_slot", &[slot as u64], &mut [])?;
    let p = Port(m);
    let port = PORTS[slot];

    // Nuvoton (and Winbond/Fintek) enter config mode with 0x87 0x87 and leave with 0xAA.
    p.outb(port, 0x87)?;
    p.outb(port, 0x87)?;
    let (id, rev) = (p.sio_in(0x20)?, p.sio_in(0x21)?);
    if let Some(chip) = identify_nuvoton(id, rev) {
        // Some tools also clear the NCT679x "I/O space lock" bit here; Omni-HW never changes
        // chip settings, so a locked chip just fails verification and is skipped.
        let found = hw_base(m, &p, 0x0B);
        p.outb(port, 0xAA)?;
        return Ok(found?.map(|base| (chip, base)));
    }
    p.outb(port, 0xAA)?;

    // ITE: 0x87 0x01 0x55 0x55 (0x55 0xAA on 0x4E); leave by writing 0x02 to register 0x02.
    for b in [0x87, 0x01, 0x55, if port == 0x2E { 0x55 } else { 0xAA }] {
        p.outb(port, b)?;
    }
    let (id, rev) = (p.sio_in(0x20)?, p.sio_in(0x21)?);
    let found = match identify_ite(id, rev) {
        Some(chip) => hw_base(m, &p, 0x04).map(|b| b.map(|base| (chip, base))),
        None => Ok(None),
    };
    p.sio_out(0x02, 0x02)?;
    found
}

/// Select logical device `ldn`, read its I/O base and let the module allow access to it.
fn hw_base(m: &Module, p: &Port, ldn: u8) -> Result<Option<u16>, String> {
    p.sio_out(0x07, ldn)?;
    let base = (u16::from(p.sio_in(0x60)?) << 8 | u16::from(p.sio_in(0x61)?)) & 0xFFF8;
    m.call("ioctl_find_bars", &[], &mut [])?;
    Ok((0x100..0xFFF8).contains(&base).then_some(base))
}

impl Device {
    /// Read one hardware-monitor register.
    fn read(&self, addr: u16) -> Result<u8, String> {
        let p = Port(&self.module);
        let [hi, lo] = addr.to_be_bytes();
        match self.chip.kind {
            ChipKind::Nct67xx { .. } => {
                // Bank select register 0x4E, then the register within the bank.
                p.outb(self.base + 5, 0x4E)?;
                p.outb(self.base + 6, hi)?;
                p.outb(self.base + 5, lo)?;
                p.inb(self.base + 6)
            }
            ChipKind::Nct6687 => {
                // EC space: reset the page latch, set page and index, read data.
                p.outb(self.base + 4, 0xFF)?;
                p.outb(self.base + 4, hi)?;
                p.outb(self.base + 5, lo)?;
                p.inb(self.base + 6)
            }
            ChipKind::Ite => {
                p.outb(self.base + 5, lo)?;
                p.inb(self.base + 6)
            }
        }
    }

    fn read16(&self, hi: u16, lo: u16) -> Result<u16, String> {
        Ok(u16::from(self.read(hi)?) << 8 | u16::from(self.read(lo)?))
    }

    /// The chip answers like the one we identified (guards against a wrong base address).
    fn verify(&self) -> bool {
        match self.chip.kind {
            // Nuvoton vendor id 0x5CA3: high byte in bank 0x80, low byte in bank 0.
            ChipKind::Nct67xx { .. } => self.read16(0x804F, 0x004F) == Ok(0x5CA3),
            // ITE vendor id register.
            ChipKind::Ite => self.read(0x58) == Ok(0x90),
            ChipKind::Nct6687 => self.read(0x100).is_ok(),
        }
    }

    fn fan(&mut self, out: &mut Vec<Reading>, i: usize, label: &str, rpm: Option<f64>) {
        let Some(rpm) = rpm else { return };
        if rpm > 0.0 {
            self.fans_seen[i] = true;
        }
        if self.fans_seen[i] {
            out.push(Reading {
                id: format!("{}/fan{i}", self.prefix),
                label: label.into(),
                group: FANS.into(),
                unit: Unit::Rpm,
                value: rpm,
            });
        }
    }

    fn push(
        &self,
        out: &mut Vec<Reading>,
        kind: &str,
        group: &str,
        label: &str,
        unit: Unit,
        v: f64,
    ) {
        out.push(Reading {
            id: format!("{}/{kind}", self.prefix),
            label: label.into(),
            group: group.into(),
            unit,
            value: v,
        });
    }

    fn poll(&mut self, out: &mut Vec<Reading>) -> Result<(), String> {
        match self.chip.kind {
            ChipKind::Nct67xx { fans } => {
                for i in 0..fans {
                    let reg = 0x4C0 + 2 * i as u16;
                    let rpm = self.read16(reg, reg + 1)?;
                    self.fan(out, i, &format!("Fan {}", i + 1), Some(f64::from(rpm)));
                }
                // VIN0 is the CPU core voltage, 8 mV per step.
                let vcore = f64::from(self.read(0x480)?) * 0.008;
                if vcore > 0.2 {
                    self.push(out, "volt/vcore", VOLTS, "CPU Vcore", Unit::Volt, vcore);
                }
            }
            ChipKind::Nct6687 => {
                for (i, label) in NCT6687_FANS.iter().enumerate() {
                    let reg = 0x140 + 2 * i as u16;
                    let rpm = self.read16(reg, reg + 1)?;
                    self.fan(out, i, label, Some(f64::from(rpm)));
                }
                for (i, label) in NCT6687_TEMPS.iter().enumerate() {
                    let reg = 0x100 + 2 * i as u16;
                    if let Some(t) = nct6687_temp(self.read(reg)?, self.read(reg + 1)?) {
                        let kind = format!("temp{i}");
                        self.push(out, &kind, TEMPS, label, Unit::Celsius, t);
                    }
                }
                for (reg, label, scale, range) in NCT6687_VOLTS {
                    let v = nct6687_volts(self.read(*reg)?, self.read(reg + 1)?) * scale;
                    if range.contains(&v) {
                        let kind = format!("volt/{reg:x}");
                        self.push(out, &kind, VOLTS, label, Unit::Volt, v);
                    }
                }
            }
            ChipKind::Ite => {
                const LO: [u16; 5] = [0x0D, 0x0E, 0x0F, 0x80, 0x82];
                const HI: [u16; 5] = [0x18, 0x19, 0x1A, 0x81, 0x83];
                for i in 0..LO.len() {
                    let count = self.read16(HI[i], LO[i])?;
                    // A header with no fan reads as "no speed": keep it hidden, or at 0 RPM
                    // once a fan has been seen there (it may have stopped).
                    let rpm = ite_fan_rpm(count).or(Some(0.0));
                    self.fan(out, i, &format!("Fan {}", i + 1), rpm);
                }
                for i in 0..3u16 {
                    if let Some(t) = ite_temp(self.read(0x29 + i)?) {
                        let (kind, label) = (format!("temp{i}"), format!("Temperature {}", i + 1));
                        self.push(out, &kind, TEMPS, &label, Unit::Celsius, t);
                    }
                }
            }
        }
        Ok(())
    }
}

impl SuperIoBackend {
    /// Probe both config ports. Errors if no supported chip answers.
    pub fn new() -> Result<Self, String> {
        let isa = GlobalMutex::open(ISA_BUS_MUTEX);
        let mut devices = Vec::new();
        let mut unknown = None;
        for slot in 0..PORTS.len() {
            // The module keeps one selected port per handle, so each chip gets its own.
            let module = Module::load("LpcIO", LPC_IO)?;
            let found = locked(isa.as_ref(), || detect(&module, slot))
                .ok_or("Super I/O busy (another monitoring tool holds it)")??;
            let Some((chip, base)) = found else {
                continue;
            };
            let fans = match chip.kind {
                ChipKind::Nct67xx { fans } => fans,
                ChipKind::Nct6687 => NCT6687_FANS.len(),
                ChipKind::Ite => 5,
            };
            let d = Device {
                module,
                chip,
                base,
                prefix: format!("sio{slot}"),
                fans_seen: vec![false; fans],
            };
            if locked(isa.as_ref(), || d.verify()) == Some(true) {
                devices.push(d);
            } else {
                unknown = Some(d.chip.name);
            }
        }
        if devices.is_empty() {
            return Err(match unknown {
                Some(name) => format!("{name} did not answer at its reported address"),
                None => "no supported Super I/O chip found".into(),
            });
        }
        Ok(Self { devices, isa })
    }

    /// Names of the chips being read, e.g. "Nuvoton NCT6687D".
    pub fn chips(&self) -> Vec<String> {
        self.devices.iter().map(|d| d.chip.name.clone()).collect()
    }
}

impl SensorBackend for SuperIoBackend {
    fn name(&self) -> &str {
        "PawnIO Super I/O"
    }

    fn poll(&mut self, out: &mut Vec<Reading>) -> Result<(), String> {
        let Self { devices, isa } = self;
        // Skip a sample rather than wait if another tool is mid-access.
        locked(isa.as_ref(), || {
            devices.iter_mut().try_for_each(|d| d.poll(out))
        })
        .unwrap_or(Ok(()))
    }
}
