//! UTF-16 helpers shared by the Win32 wrappers.

use windows::core::GUID;

/// Text up to the first NUL (or the whole slice), with invalid UTF-16 replaced.
pub(crate) fn from_wide(w: &[u16]) -> String {
    let end = w.iter().position(|&c| c == 0).unwrap_or(w.len());
    String::from_utf16_lossy(&w[..end])
}

/// Little-endian UTF-16 code units from a byte buffer (a trailing odd byte is ignored).
pub(crate) fn units(b: &[u8]) -> Vec<u16> {
    b.as_chunks::<2>()
        .0
        .iter()
        .map(|&c| u16::from_le_bytes(c))
        .collect()
}

/// [`from_wide`] for a byte buffer holding UTF-16 (registry and device properties).
pub(crate) fn from_wide_bytes(b: &[u8]) -> String {
    from_wide(&units(b))
}

/// A copy of `w` up to its first NUL, NUL-terminated, for passing as a `PCWSTR`.
pub(crate) fn nul_terminated(w: &[u16]) -> Vec<u16> {
    let end = w.iter().position(|&c| c == 0).unwrap_or(w.len());
    let mut out = Vec::with_capacity(end + 1);
    out.extend_from_slice(&w[..end]);
    out.push(0);
    out
}

/// Lower-case hyphenated GUID text ("381b4222-f694-41f0-9685-ff5bb260df2e").
pub(crate) fn guid_text(g: &GUID) -> String {
    format!("{g:?}").to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn wide_strings() {
        assert_eq!(from_wide(&w("abc\0def")), "abc");
        assert_eq!(from_wide(&w("abc")), "abc");
        assert_eq!(from_wide(&[]), "");
        assert_eq!(from_wide_bytes(&[b'h', 0, b'i', 0, 0, 0, b'x', 0]), "hi");
        assert_eq!(from_wide_bytes(&[b'h', 0, b'i']), "h");
        assert_eq!(nul_terminated(&w("ab\0cd")), w("ab\0"));
        assert_eq!(nul_terminated(&w("ab")), w("ab\0"));
    }

    #[test]
    fn guids() {
        let g = GUID::from_u128(0x381b4222_f694_41f0_9685_ff5bb260df2e);
        assert_eq!(guid_text(&g), "381b4222-f694-41f0-9685-ff5bb260df2e");
    }
}
