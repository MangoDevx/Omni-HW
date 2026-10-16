//! The Retro font family: Tahoma from the Windows fonts folder (read once, sanity-checked),
//! in front of egui's default fonts.

use egui::{FontData, FontDefinitions, FontFamily};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

/// Font family name the Retro text styles use (Tahoma first, then egui's defaults).
pub(super) const FAMILY: &str = "omnihw-retro";
/// Font data key for Tahoma.
const TAHOMA_KEY: &str = "omnihw-tahoma";
/// Where Windows keeps Tahoma.
const TAHOMA_PATH: &str = r"C:\Windows\Fonts\tahoma.ttf";

/// Tahoma has been looked for (it is read at most once per run).
pub(super) static TAHOMA_TRIED: AtomicBool = AtomicBool::new(false);

/// egui's default fonts plus the Retro family; Tahoma leads it when given.
pub(super) fn font_definitions(tahoma: Option<Vec<u8>>) -> FontDefinitions {
    let mut defs = FontDefinitions::default();
    let mut family = defs
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    if let Some(data) = tahoma {
        defs.font_data
            .insert(TAHOMA_KEY.into(), Arc::new(FontData::from_owned(data)));
        family.insert(0, TAHOMA_KEY.into());
    }
    defs.families
        .insert(FontFamily::Name(FAMILY.into()), family);
    defs
}

/// Tahoma from the Windows fonts folder, if present and well-formed (egui panics on a font it
/// can't parse, so the file is sanity-checked first).
pub(super) fn load_tahoma() -> Option<Vec<u8>> {
    let data = std::fs::read(TAHOMA_PATH).ok()?;
    looks_like_font(&data).then_some(data)
}

/// Structural check of a TrueType/OpenType file: known signature, a table directory that fits
/// in the file, and the tables a text renderer needs.
fn looks_like_font(data: &[u8]) -> bool {
    const REQUIRED: [&[u8; 4]; 5] = [b"cmap", b"head", b"hhea", b"hmtx", b"maxp"];
    let be16 = |i: usize| data.get(i..i + 2).map(|b| u16::from_be_bytes([b[0], b[1]]));
    let be32 = |i: usize| {
        data.get(i..i + 4)
            .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize)
    };
    let Some(signature) = data.get(0..4) else {
        return false;
    };
    if ![&[0, 1, 0, 0][..], b"true", b"OTTO"].contains(&signature) {
        return false;
    }
    let Some(count) = be16(4) else {
        return false;
    };
    let mut found = 0;
    for i in 0..usize::from(count) {
        let rec = 12 + 16 * i;
        let (Some(tag), Some(offset), Some(len)) =
            (data.get(rec..rec + 4), be32(rec + 8), be32(rec + 12))
        else {
            return false;
        };
        if offset.checked_add(len).is_none_or(|end| end > data.len()) {
            return false;
        }
        if REQUIRED.iter().any(|t| &t[..] == tag) {
            found += 1;
        }
    }
    found == REQUIRED.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn font_sanity_check() {
        assert!(!looks_like_font(b""));
        assert!(!looks_like_font(b"not a font at all, just text"));
        // A minimal table directory with the required tables (empty, but in bounds).
        let tags = [b"cmap", b"head", b"hhea", b"hmtx", b"maxp"];
        let mut f = vec![0, 1, 0, 0, 0, tags.len() as u8, 0, 0, 0, 0, 0, 0];
        for tag in tags {
            f.extend_from_slice(tag);
            f.extend_from_slice(&[0; 4]); // checksum
            f.extend_from_slice(&12u32.to_be_bytes()); // offset
            f.extend_from_slice(&0u32.to_be_bytes()); // length
        }
        assert!(looks_like_font(&f));
        // A table that runs past the end of the file is rejected.
        let n = f.len();
        f[n - 4..].copy_from_slice(&1_000_000u32.to_be_bytes());
        assert!(!looks_like_font(&f));
        // Retro font family is always registered.
        let defs = font_definitions(None);
        assert!(defs.families.contains_key(&FontFamily::Name(FAMILY.into())));
    }
}
