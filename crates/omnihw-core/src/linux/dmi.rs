//! SMBIOS from `/sys/firmware/dmi/tables` (root only), else the world-readable subset in
//! `/sys/class/dmi/id`.

use super::Root;
use crate::smbios::{Bios, Board, Smbios, SystemInfo};

/// SMBIOS (major, minor) from the entry point structure: "_SM_" (2.x) or "_SM3_" (3.x).
pub fn entry_point_version(ep: &[u8]) -> Option<(u8, u8)> {
    if ep.starts_with(b"_SM3_") {
        Some((*ep.get(7)?, *ep.get(8)?))
    } else if ep.starts_with(b"_SM_") {
        Some((*ep.get(6)?, *ep.get(7)?))
    } else {
        None
    }
}

/// The table wrapped in a Windows `RawSMBIOSData` header, so [`Smbios::parse_raw`] reads it.
pub fn wrap_raw(version: (u8, u8), table: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(8 + table.len());
    v.extend_from_slice(&[0, version.0, version.1, 0]);
    v.extend_from_slice(&(table.len() as u32).to_le_bytes());
    v.extend_from_slice(table);
    v
}

/// Raw table in `RawSMBIOSData` layout; `None` unless running as root.
pub fn raw_smbios(root: &Root) -> Option<Vec<u8>> {
    let table = root.read_bytes("/sys/firmware/dmi/tables/DMI")?;
    if table.is_empty() {
        return None;
    }
    let version = root
        .read_bytes("/sys/firmware/dmi/tables/smbios_entry_point")
        .and_then(|ep| entry_point_version(&ep))
        .unwrap_or((0, 0));
    Some(wrap_raw(version, &table))
}

/// BIOS, system and board names from `/sys/class/dmi/id` (no memory or CPU socket details).
pub fn from_dmi_id(root: &Root) -> Option<Smbios> {
    let get = |f: &str| {
        root.read(&format!("/sys/class/dmi/id/{f}"))
            .unwrap_or_default()
    };
    let bios = Bios {
        vendor: get("bios_vendor"),
        version: get("bios_version"),
        release_date: get("bios_date"),
    };
    let system = SystemInfo {
        manufacturer: get("sys_vendor"),
        product: get("product_name"),
        version: get("product_version"),
        family: get("product_family"),
    };
    let board = Board {
        manufacturer: get("board_vendor"),
        product: get("board_name"),
        version: get("board_version"),
    };
    let out = Smbios {
        bios: (!bios.vendor.is_empty() || !bios.version.is_empty()).then_some(bios),
        system: (!system.manufacturer.is_empty() || !system.product.is_empty()).then_some(system),
        board: (!board.manufacturer.is_empty() || !board.product.is_empty()).then_some(board),
        ..Default::default()
    };
    (out.bios.is_some() || out.system.is_some() || out.board.is_some()).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::linux::Fixture;

    #[test]
    fn entry_points() {
        let mut ep3 = b"_SM3_".to_vec();
        ep3.extend_from_slice(&[0, 0x18, 3, 6, 0]);
        assert_eq!(entry_point_version(&ep3), Some((3, 6)));
        let mut ep2 = b"_SM_".to_vec();
        ep2.extend_from_slice(&[0, 0x1f, 2, 8]);
        assert_eq!(entry_point_version(&ep2), Some((2, 8)));
        assert_eq!(entry_point_version(b"junk"), None);
    }

    #[test]
    fn raw_table_parses() {
        let table = crate::smbios::tests::sample_table();
        let s = Smbios::parse_raw(&wrap_raw((3, 4), &table)).unwrap();
        assert_eq!(s.version, (3, 4));
        assert_eq!(s, {
            let mut t = Smbios::parse_table(&table);
            t.version = (3, 4);
            t
        });
    }

    #[test]
    fn dmi_id_fallback() {
        let f = Fixture::new();
        assert_eq!(from_dmi_id(&f.root()), None);
        f.file("sys/class/dmi/id/board_vendor", "ASUSTeK COMPUTER INC.\n")
            .file("sys/class/dmi/id/board_name", "ROG STRIX B650E-F\n")
            .file("sys/class/dmi/id/bios_vendor", "American Megatrends Inc.\n")
            .file("sys/class/dmi/id/bios_version", "1813\n");
        let s = from_dmi_id(&f.root()).unwrap();
        assert_eq!(s.board.unwrap().product, "ROG STRIX B650E-F");
        assert_eq!(s.bios.unwrap().version, "1813");
        assert!(s.system.is_none());
    }
}
