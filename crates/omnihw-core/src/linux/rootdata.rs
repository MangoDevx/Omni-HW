//! Data only root can read: the raw SMBIOS table (memory modules, CPU socket), NVMe SMART
//! logs, and SATA SMART and IDENTIFY data.
//!
//! Omni-HW itself runs as the user. When asked, a short-lived copy runs as root through
//! `pkexec` (`omnihw --root-read`): it [`collect`]s the raw bytes, prints them as text and
//! exits. The app then parses them like any other source and merges them in ([`apply`]). The
//! helper only reads; it takes no input besides its flag.

use super::Root;
use crate::SystemInfo;
use crate::storage::BusType;

/// One line per item, raw bytes in hex:
/// `smbios <hex>`, `nvme <block> <hex>`, `ata-smart <block> <hex>`, `ata-identify <block> <hex>`.
pub fn collect(root: &Root) -> String {
    let mut out = String::new();
    if let Some(b) = super::dmi::raw_smbios(root) {
        push(&mut out, "smbios", None, &b);
    }
    let drives = super::block::drives(root);
    for (drive, name) in drives.iter().zip(super::block::drive_names(root)) {
        match drive.bus {
            BusType::Nvme => {
                if let Some(b) =
                    super::nvme::controller_of(&name).and_then(super::nvme::smart_log_raw)
                {
                    push(&mut out, "nvme", Some(&name), &b);
                }
            }
            BusType::Sata | BusType::Ata => {
                if let Some(b) = super::ata::smart_data(&name) {
                    push(&mut out, "ata-smart", Some(&name), &b);
                }
                if let Some(b) = super::ata::identify(&name) {
                    push(&mut out, "ata-identify", Some(&name), &b);
                }
            }
            _ => {}
        }
    }
    out
}

fn push(out: &mut String, kind: &str, block: Option<&str>, bytes: &[u8]) {
    out.push_str(kind);
    if let Some(b) = block {
        out.push(' ');
        out.push_str(b);
    }
    out.push(' ');
    out.push_str(&hex::encode(bytes));
    out.push('\n');
}

/// Merge [`collect`]'s output into `info`. `names` are the drives' block device names in
/// [`crate::storage::Drive::index`] order ([`super::block::drive_names`]). Unknown or
/// malformed lines are skipped. Returns how many items were applied.
pub fn apply(info: &mut SystemInfo, names: &[String], text: &str) -> usize {
    let mut applied = 0;
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let (Some(kind), Some(a)) = (parts.next(), parts.next()) else {
            continue;
        };
        if kind == "smbios" {
            if let Ok(b) = hex::decode(a)
                && let Ok(s) = crate::smbios::Smbios::parse_raw(&b)
            {
                info.smbios = Some(s);
                applied += 1;
            }
            continue;
        }
        let Some(bytes) = parts.next().and_then(|h| hex::decode(h).ok()) else {
            continue;
        };
        let Some(drive) = names
            .iter()
            .position(|n| n == a)
            .and_then(|i| info.drives.iter_mut().find(|d| d.index == i as u32))
        else {
            continue;
        };
        let ok = match kind {
            "nvme" => {
                drive.nvme_health = crate::storage::parse_nvme_health(&bytes);
                drive.nvme_health.is_some()
            }
            "ata-smart" => {
                drive.ata_smart = crate::smart::parse_ata_smart(&bytes);
                drive.ata_smart.is_some()
            }
            "ata-identify" => {
                drive.sata_link = crate::storage::parse_sata_link(&bytes);
                drive.sata_link.is_some()
            }
            _ => false,
        };
        applied += usize::from(ok);
    }
    applied
}

/// Whether reading as root would add anything: memory modules missing from SMBIOS, or a
/// drive without its health data.
pub fn would_add(info: &SystemInfo) -> bool {
    let no_modules = info
        .smbios
        .as_ref()
        .is_none_or(|s| s.populated_memory().next().is_none());
    no_modules
        || info.drives.iter().any(|d| match d.bus {
            BusType::Nvme => d.nvme_health.is_none(),
            BusType::Sata | BusType::Ata => d.ata_smart.is_none(),
            _ => false,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Drive;

    fn info() -> SystemInfo {
        SystemInfo {
            drives: vec![
                Drive {
                    index: 0,
                    bus: BusType::Nvme,
                    ..Default::default()
                },
                Drive {
                    index: 1,
                    bus: BusType::Sata,
                    ..Default::default()
                },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn round_trip() {
        let names = vec!["nvme0n1".to_string(), "sda".to_string()];
        let table = crate::smbios::tests::sample_table();
        let raw = super::super::dmi::wrap_raw((3, 4), &table);
        let mut nvme = vec![0u8; 512];
        nvme[1..3].copy_from_slice(&310u16.to_le_bytes()); // composite temperature, K
        nvme[3] = 100; // available spare
        let smart = crate::smart::tests::sector(&[(9, 100, 100, 1234)]);
        let mut identify = vec![0u8; 512];
        identify[152..154].copy_from_slice(&0b1110u16.to_le_bytes()); // gens 1-3
        identify[154..156].copy_from_slice(&(3u16 << 1).to_le_bytes()); // running gen 3

        let mut text = String::new();
        push(&mut text, "smbios", None, &raw);
        push(&mut text, "nvme", Some("nvme0n1"), &nvme);
        push(&mut text, "ata-smart", Some("sda"), &smart);
        push(&mut text, "ata-identify", Some("sda"), &identify);
        text.push_str("garbage line\nnvme sdz 00\n");

        let mut i = info();
        assert!(would_add(&i));
        assert_eq!(apply(&mut i, &names, &text), 4);
        assert!(i.smbios.is_some());
        assert!(i.drives[0].nvme_health.is_some());
        assert_eq!(
            i.drives[1]
                .ata_smart
                .as_ref()
                .and_then(|s| s.power_on_hours()),
            Some(1234)
        );
        assert_eq!(i.drives[1].sata_link.map(|l| l.current_gen), Some(3));
    }
}
