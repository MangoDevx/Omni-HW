//! The window icon, decoded from the app's `.ico` at startup (no image library needed: the icon
//! holds an uncompressed 32-bit bitmap).

const ICO: &[u8] = include_bytes!("../../../Assets/Brand/OmniHardwareIcon.ico");

fn u16_at(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
}

/// Decode the largest 32-bit uncompressed (BMP) image in an `.ico` file into RGBA, top row first.
/// PNG-compressed entries are skipped. Returns (rgba, width, height).
pub fn decode_ico(b: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
    if u16_at(b, 2)? != 1 {
        return None; // not an icon
    }
    let count = u16_at(b, 4)? as usize;
    let mut best: Option<(Vec<u8>, u32, u32)> = None;
    for i in 0..count {
        let e = 6 + 16 * i;
        let size = u32_at(b, e + 8)? as usize;
        let off = u32_at(b, e + 12)? as usize;
        let img = b.get(off..off + size)?;
        // BITMAPINFOHEADER: the height covers the colour image plus the AND mask.
        if u32_at(img, 0)? != 40 || u16_at(img, 14)? != 32 || u32_at(img, 16)? != 0 {
            continue;
        }
        let w = u32_at(img, 4)? as usize;
        let h = u32_at(img, 8)? as usize / 2;
        let pixels = img.get(40..40 + w * h * 4)?;
        let mut rgba = vec![0u8; w * h * 4];
        // Rows are stored bottom-up as BGRA.
        for y in 0..h {
            let src = &pixels[(h - 1 - y) * w * 4..(h - y) * w * 4];
            let dst = &mut rgba[y * w * 4..(y + 1) * w * 4];
            for (d, s) in dst
                .as_chunks_mut::<4>()
                .0
                .iter_mut()
                .zip(src.as_chunks::<4>().0)
            {
                *d = [s[2], s[1], s[0], s[3]];
            }
        }
        if best.as_ref().is_none_or(|(_, bw, _)| (w as u32) > *bw) {
            best = Some((rgba, w as u32, h as u32));
        }
    }
    best
}

/// The app icon for the window title bar and taskbar.
pub fn window_icon() -> Option<egui::IconData> {
    let (rgba, width, height) = decode_ico(ICO)?;
    Some(egui::IconData {
        rgba,
        width,
        height,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_icon_decodes() {
        let (rgba, w, h) = decode_ico(ICO).expect("app icon decodes");
        assert_eq!((w, h), (256, 256));
        assert_eq!(rgba.len(), 256 * 256 * 4);
        assert!(
            rgba.as_chunks::<4>().0.iter().any(|p| p[3] > 0),
            "not fully transparent"
        );
    }

    #[test]
    fn tiny_synthetic_icon() {
        // 2x1 icon: header, one BMP entry with blue then red pixels (stored bottom-up, BGRA).
        let mut bmp = Vec::new();
        for v in [40u32, 2, 2] {
            bmp.extend(v.to_le_bytes());
        }
        bmp.extend(1u16.to_le_bytes());
        bmp.extend(32u16.to_le_bytes());
        bmp.extend([0u8; 24]);
        bmp.extend([255, 0, 0, 255, 0, 0, 255, 255]); // BGRA: blue, red
        let mut ico = vec![0, 0, 1, 0, 1, 0];
        ico.extend([2, 1, 0, 0, 1, 0, 32, 0]);
        ico.extend((bmp.len() as u32).to_le_bytes());
        ico.extend(22u32.to_le_bytes());
        ico.extend(&bmp);
        let (rgba, w, h) = decode_ico(&ico).unwrap();
        assert_eq!((w, h), (2, 1));
        assert_eq!(rgba, vec![0, 0, 255, 255, 255, 0, 0, 255]);
        assert!(decode_ico(b"nope").is_none());
    }
}
