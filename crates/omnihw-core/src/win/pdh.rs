//! Minimal PDH query wrapper shared by the sensor backends.

use windows::Win32::System::Performance::{
    PDH_CSTATUS_NEW_DATA, PDH_CSTATUS_VALID_DATA, PDH_FMT, PDH_FMT_COUNTERVALUE_ITEM_W,
    PDH_HCOUNTER, PDH_HQUERY, PDH_MORE_DATA, PdhAddEnglishCounterW, PdhCloseQuery,
    PdhCollectQueryData, PdhGetFormattedCounterArrayW, PdhOpenQueryW,
};
use windows::core::{HSTRING, PCWSTR};

pub(crate) use windows::Win32::System::Performance::PDH_FMT_DOUBLE;
/// `PDH_FMT_DOUBLE | PDH_FMT_NOCAP100`; NOCAP100 (0x8000) is missing from the `windows` crate.
pub(crate) const PDH_FMT_DOUBLE_NOCAP100: PDH_FMT = PDH_FMT(PDH_FMT_DOUBLE.0 | 0x0000_8000);

pub(crate) struct Query {
    handle: PDH_HQUERY,
}

// SAFETY: PDH query handles may be used from any thread; we only use one at a time.
unsafe impl Send for Query {}

impl Drop for Query {
    fn drop(&mut self) {
        // SAFETY: handle came from PdhOpenQueryW and is closed once.
        unsafe { PdhCloseQuery(self.handle) };
    }
}

pub(crate) struct Counter(PDH_HCOUNTER);
// SAFETY: counter handles are owned by the query and used only alongside it.
unsafe impl Send for Counter {}

impl Query {
    pub(crate) fn open() -> Result<Self, String> {
        let mut handle = PDH_HQUERY::default();
        // SAFETY: valid out pointer; null data source = live data.
        let st = unsafe { PdhOpenQueryW(PCWSTR::null(), 0, &mut handle) };
        if st != 0 {
            return Err(format!("PdhOpenQuery failed: 0x{st:08X}"));
        }
        Ok(Query { handle })
    }

    pub(crate) fn add(&self, path: &str) -> Option<Counter> {
        let mut c = PDH_HCOUNTER::default();
        // SAFETY: the path string outlives the call.
        let st = unsafe { PdhAddEnglishCounterW(self.handle, &HSTRING::from(path), 0, &mut c) };
        (st == 0).then_some(Counter(c))
    }

    pub(crate) fn collect(&self) -> Result<(), String> {
        // SAFETY: valid query handle.
        let st = unsafe { PdhCollectQueryData(self.handle) };
        if st != 0 {
            Err(format!("PdhCollectQueryData failed: 0x{st:08X}"))
        } else {
            Ok(())
        }
    }
}

/// Read all instances of a wildcard counter as (instance name, value).
pub(crate) fn read_array(c: &Counter, format: PDH_FMT, buf: &mut Vec<u64>) -> Vec<(String, f64)> {
    let mut size: u32 = 0;
    let mut count: u32 = 0;
    // SAFETY: size query with no buffer.
    let st = unsafe { PdhGetFormattedCounterArrayW(c.0, format, &mut size, &mut count, None) };
    if st != PDH_MORE_DATA || size == 0 {
        return Vec::new();
    }
    let words = (size as usize).div_ceil(8);
    if buf.len() < words {
        buf.resize(words, 0);
    }
    let items = buf.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W;
    // SAFETY: buffer is at least `size` bytes and 8-byte aligned.
    let st =
        unsafe { PdhGetFormattedCounterArrayW(c.0, format, &mut size, &mut count, Some(items)) };
    if st != 0 {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(count as usize);
    for i in 0..count as usize {
        // SAFETY: PDH filled `count` items; names point into the same buffer.
        let item = unsafe { *items.add(i) };
        let ok = matches!(
            item.FmtValue.CStatus,
            PDH_CSTATUS_VALID_DATA | PDH_CSTATUS_NEW_DATA
        );
        if ok {
            let name = if item.szName.is_null() {
                String::new()
            } else {
                // SAFETY: non-null NUL-terminated name inside the PDH buffer.
                String::from_utf16_lossy(unsafe { item.szName.as_wide() })
            };
            // SAFETY: PDH_FMT_DOUBLE was requested, so the union holds a double.
            out.push((name, unsafe { item.FmtValue.Anonymous.doubleValue }));
        }
    }
    out
}
