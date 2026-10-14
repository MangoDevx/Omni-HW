//! Read-only registry helpers for HKLM / HKCU values (no administrator rights needed).

use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, REG_ROUTINE_FLAGS, RRF_RT_REG_BINARY,
    RRF_RT_REG_DWORD, RRF_RT_REG_MULTI_SZ, RRF_RT_REG_SZ, RegCloseKey, RegEnumKeyExW,
    RegEnumValueW, RegGetValueW, RegOpenKeyExW,
};
use windows::core::{HSTRING, PWSTR};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Hive {
    LocalMachine,
    CurrentUser,
}

impl Hive {
    fn handle(self) -> HKEY {
        match self {
            Hive::LocalMachine => HKEY_LOCAL_MACHINE,
            Hive::CurrentUser => HKEY_CURRENT_USER,
        }
    }
}

/// Raw value read; `None` if the key or value is missing or has another type.
pub(crate) fn get(
    hive: Hive,
    subkey: &str,
    value: &str,
    kind: REG_ROUTINE_FLAGS,
    buf: &mut [u8],
) -> Option<usize> {
    let mut len = buf.len() as u32;
    // SAFETY: buffer size in bytes passed in `len`; strings are NUL-terminated.
    let st = unsafe {
        RegGetValueW(
            hive.handle(),
            &HSTRING::from(subkey),
            &HSTRING::from(value),
            kind,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut len),
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

pub(crate) fn binary(hive: Hive, subkey: &str, value: &str) -> Option<Vec<u8>> {
    let mut buf = vec![0u8; 256];
    let len = get(hive, subkey, value, RRF_RT_REG_BINARY, &mut buf)?;
    buf.truncate(len);
    Some(buf)
}

/// Open key, closed on drop.
struct Key(HKEY);

impl Key {
    fn open(hive: Hive, subkey: &str) -> Option<Key> {
        let mut h = HKEY::default();
        // SAFETY: valid out pointer; closed in Drop.
        let st = unsafe {
            RegOpenKeyExW(
                hive.handle(),
                &HSTRING::from(subkey),
                None,
                KEY_READ,
                &mut h,
            )
        };
        (st == ERROR_SUCCESS).then_some(Key(h))
    }
}

impl Drop for Key {
    fn drop(&mut self) {
        // SAFETY: opened in Key::open.
        let _ = unsafe { RegCloseKey(self.0) };
    }
}

/// Names of the direct subkeys of a key.
pub(crate) fn subkeys(hive: Hive, subkey: &str) -> Vec<String> {
    let Some(key) = Key::open(hive, subkey) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut name = [0u16; 256];
    for i in 0.. {
        let mut len = name.len() as u32;
        // SAFETY: buffer length passed in characters.
        let st = unsafe {
            RegEnumKeyExW(
                key.0,
                i,
                Some(PWSTR(name.as_mut_ptr())),
                &mut len,
                None,
                None,
                None,
                None,
            )
        };
        if st != ERROR_SUCCESS {
            break;
        }
        out.push(String::from_utf16_lossy(&name[..len as usize]));
    }
    out
}

/// Names of the values in a key.
pub(crate) fn value_names(hive: Hive, subkey: &str) -> Vec<String> {
    let Some(key) = Key::open(hive, subkey) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut name = [0u16; 512];
    for i in 0.. {
        let mut len = name.len() as u32;
        // SAFETY: name buffer length passed in characters; no data requested.
        let st = unsafe {
            RegEnumValueW(
                key.0,
                i,
                Some(PWSTR(name.as_mut_ptr())),
                &mut len,
                None,
                None,
                None,
                None,
            )
        };
        if st != ERROR_SUCCESS {
            break;
        }
        out.push(String::from_utf16_lossy(&name[..len as usize]));
    }
    out
}

pub(crate) fn key_exists(hive: Hive, subkey: &str) -> bool {
    let mut h = HKEY::default();
    // SAFETY: valid out pointer; key closed below.
    let st = unsafe {
        RegOpenKeyExW(
            hive.handle(),
            &HSTRING::from(subkey),
            None,
            KEY_READ,
            &mut h,
        )
    };
    if st == ERROR_SUCCESS {
        // SAFETY: handle opened above.
        let _ = unsafe { RegCloseKey(h) };
        true
    } else {
        false
    }
}
