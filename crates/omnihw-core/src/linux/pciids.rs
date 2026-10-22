//! PCI device names from the system's `pci.ids` database (shipped by `hwdata` / `pciutils`).

use super::Root;

const PATHS: [&str; 4] = [
    "/usr/share/hwdata/pci.ids",
    "/usr/share/misc/pci.ids",
    "/usr/share/pci.ids",
    "/var/lib/pciutils/pci.ids",
];

/// Raw device name for `vendor:device`, e.g. "AD102 [GeForce RTX 4090]".
pub fn device_name(db: &str, vendor: u32, device: u32) -> Option<String> {
    let vendor = format!("{vendor:04x}");
    let device = format!("{device:04x}");
    let mut in_vendor = false;
    for line in db.lines() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        // Device classes ("C 03 ...") follow all vendors.
        if line.starts_with("C ") {
            break;
        }
        if !line.starts_with('\t') {
            if in_vendor {
                return None;
            }
            in_vendor = line
                .get(..4)
                .is_some_and(|v| v.eq_ignore_ascii_case(&vendor));
            continue;
        }
        if in_vendor && !line.starts_with("\t\t") {
            let l = &line[1..];
            if l.get(..4).is_some_and(|d| d.eq_ignore_ascii_case(&device)) {
                return Some(l[4..].trim().to_string());
            }
        }
    }
    None
}

/// Marketing name: the bracketed part if there is one ("GeForce RTX 4090"), else the whole name.
/// Several names in one bracket ("Radeon RX 6800/6800 XT / 6900 XT") are kept as-is.
pub fn marketing_name(raw: &str) -> &str {
    match (raw.find('['), raw.rfind(']')) {
        (Some(a), Some(b)) if b > a + 1 => raw[a + 1..b].trim(),
        _ => raw.trim(),
    }
}

/// Look up `vendor:device` in the first readable `pci.ids`.
pub fn lookup(root: &Root, vendor: u32, device: u32) -> Option<String> {
    PATHS
        .iter()
        .find_map(|p| root.read(p))
        .and_then(|db| device_name(&db, vendor, device))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DB: &str = "# comment\n\
1002  Advanced Micro Devices, Inc. [AMD/ATI]\n\
\t73bf  Navi 21 [Radeon RX 6800/6800 XT / 6900 XT]\n\
\t\t1043 04f2  Radeon RX 6800 XT\n\
10de  NVIDIA Corporation\n\
\t2684  AD102 [GeForce RTX 4090]\n\
\t2685  AD102\n\
8086  Intel Corporation\n\
C 03  Display controller\n";

    #[test]
    fn lookups() {
        assert_eq!(
            device_name(DB, 0x10de, 0x2684).as_deref(),
            Some("AD102 [GeForce RTX 4090]")
        );
        assert_eq!(
            device_name(DB, 0x1002, 0x73BF).as_deref(),
            Some("Navi 21 [Radeon RX 6800/6800 XT / 6900 XT]")
        );
        assert_eq!(device_name(DB, 0x10de, 0x9999), None);
        assert_eq!(device_name(DB, 0x8086, 0x1234), None);
    }

    #[test]
    fn marketing() {
        assert_eq!(
            marketing_name("AD102 [GeForce RTX 4090]"),
            "GeForce RTX 4090"
        );
        assert_eq!(marketing_name("AD102"), "AD102");
    }
}
