//! Bounds-checked little-endian integer reads from raw firmware/IOCTL buffers.

/// `N` bytes at offset `o`, or `None` if the buffer is too short.
fn bytes<const N: usize>(b: &[u8], o: usize) -> Option<[u8; N]> {
    b.get(o..o.checked_add(N)?)?.try_into().ok()
}

pub(crate) fn u8_at(b: &[u8], o: usize) -> Option<u8> {
    b.get(o).copied()
}

pub(crate) fn u16_at(b: &[u8], o: usize) -> Option<u16> {
    bytes(b, o).map(u16::from_le_bytes)
}

pub(crate) fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    bytes(b, o).map(u32::from_le_bytes)
}

pub(crate) fn u64_at(b: &[u8], o: usize) -> Option<u64> {
    bytes(b, o).map(u64::from_le_bytes)
}

pub(crate) fn u128_at(b: &[u8], o: usize) -> Option<u128> {
    bytes(b, o).map(u128::from_le_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads() {
        let b = [1, 2, 3, 4, 5, 6, 7, 8];
        assert_eq!(u8_at(&b, 7), Some(8));
        assert_eq!(u16_at(&b, 0), Some(0x0201));
        assert_eq!(u32_at(&b, 4), Some(0x0807_0605));
        assert_eq!(u64_at(&b, 0), Some(0x0807_0605_0403_0201));
        assert_eq!(u32_at(&b, 5), None);
        assert_eq!(u128_at(&b, 0), None);
        assert_eq!(u16_at(&b, usize::MAX), None);
    }
}
