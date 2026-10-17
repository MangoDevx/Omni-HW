//! Registry helpers for HKLM / HKCU values. Reads need no administrator rights; the writers
//! (used only by power settings changes) usually do for HKLM.

use super::wide::{from_wide, units};
use windows::Win32::Foundation::{ERROR_MORE_DATA, ERROR_SUCCESS, WIN32_ERROR};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, REG_DWORD, REG_ROUTINE_FLAGS, REG_SZ,
    REG_VALUE_TYPE, RRF_RT_REG_BINARY, RRF_RT_REG_DWORD, RRF_RT_REG_MULTI_SZ, RRF_RT_REG_SZ,
    RegEnumKeyExW, RegEnumValueW, RegGetValueW, RegOpenKeyExW, RegSetKeyValueW,
};
use windows::core::{HSTRING, Owned, PWSTR};

/// HKLM key of the network adapter device class; one subkey per adapter driver instance.
pub(crate) const NET_CLASS: &str =
    r"SYSTEM\CurrentControlSet\Control\Class\{4d36e972-e325-11ce-bfc1-08002be10318}";

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

/// `RegGetValueW` into `buf`; returns the status and the byte length read (or needed).
fn get_value(
    hive: Hive,
    subkey: &str,
    value: &str,
    kind: REG_ROUTINE_FLAGS,
    buf: Option<&mut [u8]>,
) -> (WIN32_ERROR, usize) {
    let mut len = buf.as_ref().map_or(0, |b| b.len() as u32);
    let ptr = buf.map(|b| b.as_mut_ptr().cast());
    // SAFETY: the key/value names are NUL-terminated HSTRINGs that outlive the call; `ptr` is
    // either absent (size query) or points to a buffer of `len` bytes.
    let st = unsafe {
        RegGetValueW(
            hive.handle(),
            &HSTRING::from(subkey),
            &HSTRING::from(value),
            kind,
            None,
            ptr,
            Some(&mut len),
        )
    };
    (st, len as usize)
}

/// Raw value read into a fixed buffer; `None` if the key or value is missing, has another
/// type or does not fit.
pub(crate) fn get(
    hive: Hive,
    subkey: &str,
    value: &str,
    kind: REG_ROUTINE_FLAGS,
    buf: &mut [u8],
) -> Option<usize> {
    let (st, len) = get_value(hive, subkey, value, kind, Some(buf));
    (st == ERROR_SUCCESS).then_some(len)
}

/// Raw value of any size; `None` if the key or value is missing or has another type.
fn get_vec(hive: Hive, subkey: &str, value: &str, kind: REG_ROUTINE_FLAGS) -> Option<Vec<u8>> {
    let (st, mut size) = get_value(hive, subkey, value, kind, None);
    if st != ERROR_SUCCESS {
        return None;
    }
    // The value can grow between the size query and the read; retry a few times.
    for _ in 0..3 {
        let mut buf = vec![0u8; size.max(2)];
        match get_value(hive, subkey, value, kind, Some(&mut buf)) {
            (ERROR_SUCCESS, len) => {
                buf.truncate(len);
                return Some(buf);
            }
            (st, needed) if st == ERROR_MORE_DATA => size = needed,
            _ => return None,
        }
    }
    None
}

pub(crate) fn dword(hive: Hive, subkey: &str, value: &str) -> Option<u32> {
    let mut buf = [0u8; 4];
    get(hive, subkey, value, RRF_RT_REG_DWORD, &mut buf)?;
    Some(u32::from_le_bytes(buf))
}

/// REG_SZ (or REG_EXPAND_SZ, expanded) value.
pub(crate) fn string(hive: Hive, subkey: &str, value: &str) -> Option<String> {
    let buf = get_vec(hive, subkey, value, RRF_RT_REG_SZ)?;
    Some(from_wide(&units(&buf)))
}

/// REG_MULTI_SZ as a list of non-empty strings.
pub(crate) fn multi_string(hive: Hive, subkey: &str, value: &str) -> Option<Vec<String>> {
    let buf = get_vec(hive, subkey, value, RRF_RT_REG_MULTI_SZ)?;
    Some(
        units(&buf)
            .split(|&c| c == 0)
            .filter(|s| !s.is_empty())
            .map(String::from_utf16_lossy)
            .collect(),
    )
}

pub(crate) fn binary(hive: Hive, subkey: &str, value: &str) -> Option<Vec<u8>> {
    get_vec(hive, subkey, value, RRF_RT_REG_BINARY)
}

/// An open registry key, closed with `RegCloseKey` on drop.
struct Key(Owned<HKEY>);

impl Key {
    fn open(hive: Hive, subkey: &str) -> Option<Key> {
        let mut h = HKEY::default();
        // SAFETY: the subkey is a NUL-terminated HSTRING that outlives the call; valid out
        // pointer to a local.
        let st = unsafe {
            RegOpenKeyExW(
                hive.handle(),
                &HSTRING::from(subkey),
                None,
                KEY_READ,
                &mut h,
            )
        };
        // SAFETY: on success `h` is a newly opened key owned by nobody else.
        (st == ERROR_SUCCESS).then(|| Key(unsafe { Owned::new(h) }))
    }
}

/// Collect names from an enumeration call: `enumerate(index, buffer, length in/out)` returns
/// `ERROR_SUCCESS` for each entry and an error after the last.
fn enum_names(
    buf_len: usize,
    mut enumerate: impl FnMut(u32, &mut [u16], &mut u32) -> WIN32_ERROR,
) -> Vec<String> {
    let mut out = Vec::new();
    let mut name = vec![0u16; buf_len];
    for i in 0.. {
        let mut len = name.len() as u32;
        if enumerate(i, &mut name, &mut len) != ERROR_SUCCESS {
            break;
        }
        out.push(String::from_utf16_lossy(
            &name[..(len as usize).min(name.len())],
        ));
    }
    out
}

/// Names of the direct subkeys of a key.
pub(crate) fn subkeys(hive: Hive, subkey: &str) -> Vec<String> {
    let Some(key) = Key::open(hive, subkey) else {
        return Vec::new();
    };
    // Key names are at most 255 characters.
    enum_names(256, |i, name, len| {
        // SAFETY: `key` is open; the name buffer's length in characters is passed in `len`.
        unsafe {
            RegEnumKeyExW(
                *key.0,
                i,
                Some(PWSTR(name.as_mut_ptr())),
                len,
                None,
                None,
                None,
                None,
            )
        }
    })
}

/// Names of the values in a key.
pub(crate) fn value_names(hive: Hive, subkey: &str) -> Vec<String> {
    let Some(key) = Key::open(hive, subkey) else {
        return Vec::new();
    };
    enum_names(512, |i, name, len| {
        // SAFETY: `key` is open; the name buffer's length in characters is passed in `len`;
        // no type or data is requested.
        unsafe {
            RegEnumValueW(
                *key.0,
                i,
                Some(PWSTR(name.as_mut_ptr())),
                len,
                None,
                None,
                None,
                None,
            )
        }
    })
}

pub(crate) fn key_exists(hive: Hive, subkey: &str) -> bool {
    Key::open(hive, subkey).is_some()
}

/// `RegSetKeyValueW` with raw `data` of the given type, creating the key if needed.
fn set_raw(
    hive: Hive,
    subkey: &str,
    value: &str,
    kind: REG_VALUE_TYPE,
    data: &[u8],
) -> Result<(), WIN32_ERROR> {
    // SAFETY: the names are NUL-terminated HSTRINGs that outlive the call; the data pointer
    // and size describe `data`.
    let st = unsafe {
        RegSetKeyValueW(
            hive.handle(),
            &HSTRING::from(subkey),
            &HSTRING::from(value),
            kind.0,
            Some(data.as_ptr().cast()),
            data.len() as u32,
        )
    };
    if st == ERROR_SUCCESS { Ok(()) } else { Err(st) }
}

/// Write a REG_DWORD (creating the key if needed). Usually needs administrator rights for HKLM.
pub(crate) fn set_dword(
    hive: Hive,
    subkey: &str,
    value: &str,
    data: u32,
) -> Result<(), WIN32_ERROR> {
    set_raw(hive, subkey, value, REG_DWORD, &data.to_le_bytes())
}

/// Write a REG_SZ value. Usually needs administrator rights for HKLM.
pub(crate) fn set_string(
    hive: Hive,
    subkey: &str,
    value: &str,
    data: &str,
) -> Result<(), WIN32_ERROR> {
    // UTF-16LE including the terminating NUL, as REG_SZ requires.
    let bytes: Vec<u8> = data
        .encode_utf16()
        .chain(std::iter::once(0))
        .flat_map(u16::to_le_bytes)
        .collect();
    set_raw(hive, subkey, value, REG_SZ, &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CURRENT_VERSION: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";

    #[test]
    fn reads_well_known_values() {
        let product = string(Hive::LocalMachine, CURRENT_VERSION, "ProductName");
        assert!(product.is_some_and(|p| p.starts_with("Windows")));
        assert!(dword(Hive::LocalMachine, CURRENT_VERSION, "UBR").is_some());
        // Wrong type: ProductName is REG_SZ, not REG_DWORD.
        assert_eq!(
            dword(Hive::LocalMachine, CURRENT_VERSION, "ProductName"),
            None
        );
        assert_eq!(
            string(Hive::LocalMachine, CURRENT_VERSION, "NoSuchValue"),
            None
        );
        assert!(key_exists(Hive::LocalMachine, CURRENT_VERSION));
        assert!(!key_exists(
            Hive::LocalMachine,
            r"SOFTWARE\NoSuchKey\OmniHW"
        ));
        assert!(!subkeys(Hive::LocalMachine, r"SOFTWARE\Microsoft").is_empty());
        assert!(!value_names(Hive::LocalMachine, CURRENT_VERSION).is_empty());
    }
}
