//! Read-only registry helpers for HKLM / HKCU values (no administrator rights needed).

use super::ffi::{HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegGetValueW, wide};
use std::ffi::c_void;
use std::ptr;

const HKEY_CURRENT_USER: isize = 0x8000_0001u32 as i32 as isize;
const RRF_RT_REG_MULTI_SZ: u32 = 0x20;
const KEY_READ: u32 = 0x2_0019;
const ERROR_SUCCESS: i32 = 0;

#[link(name = "advapi32")]
unsafe extern "system" {
    fn RegOpenKeyExW(
        key: isize,
        subkey: *const u16,
        options: u32,
        sam: u32,
        result: *mut isize,
    ) -> i32;
    fn RegCloseKey(key: isize) -> i32;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Hive {
    LocalMachine,
    CurrentUser,
}

impl Hive {
    fn handle(self) -> isize {
        match self {
            Hive::LocalMachine => HKEY_LOCAL_MACHINE,
            Hive::CurrentUser => HKEY_CURRENT_USER,
        }
    }
}

/// Raw value read; `None` if the key or value is missing or has another type.
fn get(hive: Hive, subkey: &str, value: &str, kind: u32, buf: &mut [u8]) -> Option<usize> {
    let (k, v) = (wide(subkey), wide(value));
    let mut len = buf.len() as u32;
    // SAFETY: buffer size in bytes passed in `len`; strings are NUL-terminated.
    let st = unsafe {
        RegGetValueW(
            hive.handle(),
            k.as_ptr(),
            v.as_ptr(),
            kind,
            ptr::null_mut(),
            buf.as_mut_ptr() as *mut c_void,
            &mut len,
        )
    };
    (st == ERROR_SUCCESS).then_some(len as usize)
}

fn utf16(buf: &[u8], len: usize) -> Vec<u16> {
    buf[..len.min(buf.len())]
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect()
}

pub(crate) fn dword(hive: Hive, subkey: &str, value: &str) -> Option<u32> {
    let mut buf = [0u8; 4];
    get(hive, subkey, value, RRF_RT_REG_DWORD, &mut buf)?;
    Some(u32::from_le_bytes(buf))
}

pub(crate) fn string(hive: Hive, subkey: &str, value: &str) -> Option<String> {
    let mut buf = vec![0u8; 1024];
    let len = get(hive, subkey, value, RRF_RT_REG_SZ, &mut buf)?;
    let w = utf16(&buf, len);
    let end = w.iter().position(|&c| c == 0).unwrap_or(w.len());
    Some(String::from_utf16_lossy(&w[..end]))
}

/// REG_MULTI_SZ as a list of non-empty strings.
pub(crate) fn multi_string(hive: Hive, subkey: &str, value: &str) -> Option<Vec<String>> {
    let mut buf = vec![0u8; 16 * 1024];
    let len = get(hive, subkey, value, RRF_RT_REG_MULTI_SZ, &mut buf)?;
    Some(
        utf16(&buf, len)
            .split(|&c| c == 0)
            .filter(|s| !s.is_empty())
            .map(String::from_utf16_lossy)
            .collect(),
    )
}

pub(crate) fn key_exists(hive: Hive, subkey: &str) -> bool {
    let k = wide(subkey);
    let mut h: isize = 0;
    // SAFETY: valid out pointer; key closed below.
    let st = unsafe { RegOpenKeyExW(hive.handle(), k.as_ptr(), 0, KEY_READ, &mut h) };
    if st == ERROR_SUCCESS {
        // SAFETY: handle opened above.
        unsafe { RegCloseKey(h) };
        true
    } else {
        false
    }
}
