//! (vendor, family, model) -> codename / microarchitecture / process node.

use crate::cpuid::Vendor;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Codename {
    pub codename: &'static str,
    pub microarch: &'static str,
    pub process: &'static str,
}

struct Entry {
    vendor: Vendor,
    family: u32,
    models: (u32, u32),
    name: Codename,
}

const fn e(
    vendor: Vendor,
    family: u32,
    lo: u32,
    hi: u32,
    codename: &'static str,
    microarch: &'static str,
    process: &'static str,
) -> Entry {
    Entry {
        vendor,
        family,
        models: (lo, hi),
        name: Codename {
            codename,
            microarch,
            process,
        },
    }
}

use Vendor::{Amd as A, Intel as I};

#[rustfmt::skip]
static TABLE: &[Entry] = &[
    // Intel client
    e(I, 6, 0x2A, 0x2A, "Sandy Bridge", "Sandy Bridge", "32 nm"),
    e(I, 6, 0x2D, 0x2D, "Sandy Bridge-E", "Sandy Bridge", "32 nm"),
    e(I, 6, 0x3A, 0x3A, "Ivy Bridge", "Ivy Bridge", "22 nm"),
    e(I, 6, 0x3E, 0x3E, "Ivy Bridge-E", "Ivy Bridge", "22 nm"),
    e(I, 6, 0x3C, 0x3C, "Haswell", "Haswell", "22 nm"),
    e(I, 6, 0x45, 0x46, "Haswell", "Haswell", "22 nm"),
    e(I, 6, 0x3F, 0x3F, "Haswell-E", "Haswell", "22 nm"),
    e(I, 6, 0x3D, 0x3D, "Broadwell", "Broadwell", "14 nm"),
    e(I, 6, 0x47, 0x47, "Broadwell", "Broadwell", "14 nm"),
    e(I, 6, 0x4F, 0x4F, "Broadwell-E", "Broadwell", "14 nm"),
    e(I, 6, 0x4E, 0x4E, "Skylake", "Skylake", "14 nm"),
    e(I, 6, 0x5E, 0x5E, "Skylake", "Skylake", "14 nm"),
    e(I, 6, 0x55, 0x55, "Skylake-SP / Cascade Lake", "Skylake", "14 nm"),
    e(I, 6, 0x8E, 0x8E, "Kaby Lake / Whiskey Lake", "Skylake", "14 nm"),
    e(I, 6, 0x9E, 0x9E, "Kaby Lake / Coffee Lake", "Skylake", "14 nm"),
    e(I, 6, 0xA5, 0xA6, "Comet Lake", "Skylake", "14 nm"),
    e(I, 6, 0xA7, 0xA7, "Rocket Lake", "Cypress Cove", "14 nm"),
    e(I, 6, 0x66, 0x66, "Cannon Lake", "Palm Cove", "10 nm"),
    e(I, 6, 0x7D, 0x7E, "Ice Lake", "Sunny Cove", "10 nm"),
    e(I, 6, 0x6A, 0x6C, "Ice Lake-SP", "Sunny Cove", "10 nm"),
    e(I, 6, 0x8C, 0x8D, "Tiger Lake", "Willow Cove", "10 nm SuperFin"),
    e(I, 6, 0x97, 0x97, "Alder Lake-S", "Golden Cove + Gracemont", "Intel 7"),
    e(I, 6, 0x9A, 0x9A, "Alder Lake-P", "Golden Cove + Gracemont", "Intel 7"),
    e(I, 6, 0xBE, 0xBE, "Alder Lake-N", "Gracemont", "Intel 7"),
    e(I, 6, 0xB7, 0xB7, "Raptor Lake-S", "Raptor Cove + Gracemont", "Intel 7"),
    e(I, 6, 0xBA, 0xBA, "Raptor Lake-P", "Raptor Cove + Gracemont", "Intel 7"),
    e(I, 6, 0xBF, 0xBF, "Raptor Lake-S", "Raptor Cove + Gracemont", "Intel 7"),
    e(I, 6, 0xAA, 0xAC, "Meteor Lake", "Redwood Cove + Crestmont", "Intel 4"),
    e(I, 6, 0xBD, 0xBD, "Lunar Lake", "Lion Cove + Skymont", "TSMC N3B"),
    e(I, 6, 0xC5, 0xC6, "Arrow Lake", "Lion Cove + Skymont", "TSMC N3B"),
    e(I, 6, 0xB5, 0xB5, "Arrow Lake-U", "Redwood Cove + Crestmont", "Intel 3"),
    e(I, 6, 0xCC, 0xCC, "Panther Lake", "Cougar Cove + Darkmont", "Intel 18A"),
    // Intel server
    e(I, 6, 0x8F, 0x8F, "Sapphire Rapids", "Golden Cove", "Intel 7"),
    e(I, 6, 0xCF, 0xCF, "Emerald Rapids", "Raptor Cove", "Intel 7"),
    e(I, 6, 0xAD, 0xAE, "Granite Rapids", "Redwood Cove", "Intel 3"),
    e(I, 6, 0xAF, 0xAF, "Sierra Forest", "Crestmont", "Intel 3"),
    e(I, 6, 0xDD, 0xDD, "Clearwater Forest", "Darkmont", "Intel 18A"),
    // AMD family 17h (Zen / Zen+ / Zen 2)
    e(A, 0x17, 0x00, 0x07, "Summit Ridge / Naples", "Zen", "GF 14 nm"),
    e(A, 0x17, 0x08, 0x0F, "Pinnacle Ridge / Colfax", "Zen+", "GF 12 nm"),
    e(A, 0x17, 0x10, 0x17, "Raven Ridge", "Zen", "GF 14 nm"),
    e(A, 0x17, 0x18, 0x1F, "Picasso", "Zen+", "GF 12 nm"),
    e(A, 0x17, 0x20, 0x2F, "Dali / Pollock", "Zen", "GF 14 nm"),
    e(A, 0x17, 0x30, 0x3F, "Rome / Castle Peak", "Zen 2", "TSMC 7 nm"),
    e(A, 0x17, 0x60, 0x67, "Renoir", "Zen 2", "TSMC 7 nm"),
    e(A, 0x17, 0x68, 0x6F, "Lucienne", "Zen 2", "TSMC 7 nm"),
    e(A, 0x17, 0x70, 0x7F, "Matisse", "Zen 2", "TSMC 7 nm"),
    e(A, 0x17, 0x90, 0x9F, "Van Gogh", "Zen 2", "TSMC 7 nm"),
    e(A, 0x17, 0xA0, 0xAF, "Mendocino", "Zen 2", "TSMC 6 nm"),
    // AMD family 19h (Zen 3 / Zen 4)
    e(A, 0x19, 0x00, 0x0F, "Milan", "Zen 3", "TSMC 7 nm"),
    e(A, 0x19, 0x10, 0x1F, "Genoa", "Zen 4", "TSMC 5 nm"),
    e(A, 0x19, 0x20, 0x2F, "Vermeer", "Zen 3", "TSMC 7 nm"),
    e(A, 0x19, 0x40, 0x4F, "Rembrandt", "Zen 3+", "TSMC 6 nm"),
    e(A, 0x19, 0x50, 0x5F, "Cezanne / Barcelo", "Zen 3", "TSMC 7 nm"),
    e(A, 0x19, 0x60, 0x6F, "Raphael", "Zen 4", "TSMC 5 nm"),
    e(A, 0x19, 0x70, 0x7F, "Phoenix / Hawk Point", "Zen 4", "TSMC 4 nm"),
    e(A, 0x19, 0xA0, 0xAF, "Bergamo / Siena", "Zen 4c", "TSMC 5 nm"),
    // AMD family 1Ah (Zen 5)
    e(A, 0x1A, 0x00, 0x0F, "Turin", "Zen 5", "TSMC 4 nm"),
    e(A, 0x1A, 0x10, 0x1F, "Turin Dense", "Zen 5c", "TSMC 3 nm"),
    e(A, 0x1A, 0x20, 0x2F, "Strix Point", "Zen 5", "TSMC 4 nm"),
    e(A, 0x1A, 0x40, 0x4F, "Granite Ridge", "Zen 5", "TSMC 4 nm"),
    e(A, 0x1A, 0x60, 0x6F, "Krackan Point", "Zen 5", "TSMC 4 nm"),
    e(A, 0x1A, 0x70, 0x7F, "Strix Halo", "Zen 5", "TSMC 4 nm"),
];

/// Look up a codename by displayed family/model. Returns `None` for unknown parts.
pub fn lookup(vendor: Vendor, family: u32, model: u32) -> Option<Codename> {
    TABLE
        .iter()
        .find(|e| {
            e.vendor == vendor && e.family == family && (e.models.0..=e.models.1).contains(&model)
        })
        .map(|e| e.name)
}

/// The socket of a desktop-only AMD family (no mobile or workstation parts share its codename),
/// for when SMBIOS can't be read.
pub fn desktop_socket(codename: &str) -> Option<&'static str> {
    match codename {
        "Matisse" | "Vermeer" => Some("AM4"),
        "Raphael" | "Granite Ridge" => Some("AM5"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_parts() {
        assert_eq!(lookup(I, 6, 0xB7).unwrap().codename, "Raptor Lake-S");
        assert_eq!(lookup(I, 6, 0x9E).unwrap().process, "14 nm");
        assert_eq!(lookup(A, 0x19, 0x61).unwrap().codename, "Raphael");
        assert_eq!(lookup(A, 0x19, 0x21).unwrap().microarch, "Zen 3");
        assert_eq!(lookup(A, 0x1A, 0x44).unwrap().codename, "Granite Ridge");
        assert_eq!(lookup(A, 0x17, 0x71).unwrap().codename, "Matisse");
        let vermeer = lookup(A, 0x19, 0x21).unwrap().codename;
        assert_eq!(desktop_socket(vermeer), Some("AM4"));
        assert_eq!(desktop_socket("Cezanne / Barcelo"), None);
    }

    #[test]
    fn unknown_parts() {
        assert!(lookup(I, 6, 0x01).is_none());
        assert!(lookup(A, 6, 0xB7).is_none());
        assert!(lookup(Vendor::Other, 6, 0xB7).is_none());
    }

    #[test]
    fn ranges_do_not_overlap() {
        for (i, a) in TABLE.iter().enumerate() {
            for b in &TABLE[i + 1..] {
                let overlap = a.vendor == b.vendor
                    && a.family == b.family
                    && a.models.0 <= b.models.1
                    && b.models.0 <= a.models.1;
                assert!(!overlap, "{} overlaps {}", a.name.codename, b.name.codename);
            }
        }
    }
}
