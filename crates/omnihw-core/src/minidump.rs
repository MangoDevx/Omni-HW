//! Kernel crash dump analysis: the stop code, its four parameters and the driver a blue screen
//! points at, read from the small memory dumps Windows writes to `C:\Windows\Minidump`.
//!
//! These are *kernel* dumps (`PAGE` + `DU64` header, `DumpType` 4 = triage), not the user-mode
//! `MDMP` minidumps that crates like `minidump` read; `kdmp-parser` handles full and bitmap
//! kernel dumps but rejects triage dumps. So this is a small focused parser: only the header,
//! the triage driver list and the saved call stack, every offset and length bounds-checked.
//!
//! Layout sources:
//! - `DUMP_HEADER64`, `TRIAGE_DUMP64`, `DUMP_STRING` and `DUMP_TYPE_TRIAGE`: the Windows
//!   `Dump.h` published with the Singularity RDK
//!   (<https://git.crustywindo.ws/modeco80/singrdk/src/commit/4e1c631ec85a8b05ec2a7c0d1805c47ba4f2138d/base/Windows/Inc/Dump.h>).
//! - Header field offsets (BugCheckCode 0x38, ContextRecord 0x348, Exception 0xF00, DumpType
//!   0xF98, SystemTime 0xFA8; header size 0x2000): kdmp-parser's `kdmp-parser-structs.h`
//!   (<https://github.com/0vercl0k/kdmp-parser>), which asserts them.
//! - Triage offsets are from the start of the file, and a driver entry is 0x90 bytes
//!   (`ULONG DriverNameOffset`, padding, then `KLDR_DATA_TABLE_ENTRY64`): WinDbg's
//!   `TRIAGE_DUMP64` output for a real minidump (ContextOffset 0x348, ExceptionOffset 0xF00,
//!   0xA0 drivers from 0x8BF8 up to the string pool at 0xE5F8), e.g.
//!   <https://forums.tomshardware.com/threads/system-service-exception-svchost-exe-ntkrnlmp-exe.2531761/>.
//! - `KLDR_DATA_TABLE_ENTRY64`: DllBase at 0x30, SizeOfImage at 0x40 (Geoff Chappell,
//!   <https://www.geoffchappell.com/studies/windows/km/ntoskrnl/inc/api/ntldr/ldr_data_table_entry.htm>).
//! - AMD64 `CONTEXT.Rip` at 0xF8, `EXCEPTION_RECORD64.ExceptionAddress` at 0x10 (winnt.h).

use crate::le::{u32_at, u64_at};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Most bytes read from one dump file (small dumps are 0.25-3 MB).
pub const MAX_READ: u64 = 16 << 20;
/// Most dump files read, newest first.
pub const MAX_DUMPS: usize = 20;

const SIGNATURE: &[u8; 4] = b"PAGE";
const VALID_64: &[u8; 4] = b"DU64";
const VALID_32: &[u8; 4] = b"DUMP";
/// `DUMP_HEADER64` field offsets.
const BUGCHECK_CODE: usize = 0x38;
const BUGCHECK_PARAMS: usize = 0x40;
const CONTEXT: usize = 0x348;
const EXCEPTION: usize = 0xF00;
const DUMP_TYPE: usize = 0xF98;
const SYSTEM_TIME: usize = 0xFA8;
/// `TRIAGE_DUMP64` follows the 0x2000-byte header.
const TRIAGE: usize = 0x2000;
const DUMP_TYPE_TRIAGE: u32 = 4;
/// `CONTEXT.Rip` (AMD64) and `EXCEPTION_RECORD64.ExceptionAddress`.
const CONTEXT_RIP: usize = 0xF8;
const EXCEPTION_ADDRESS: usize = 0x10;
/// `TRIAGE_DUMP64` field offsets.
const T_CONTEXT_OFFSET: usize = 0x0C;
const T_EXCEPTION_OFFSET: usize = 0x10;
const T_CALL_STACK_OFFSET: usize = 0x28;
const T_CALL_STACK_SIZE: usize = 0x2C;
const T_DRIVER_LIST_OFFSET: usize = 0x30;
const T_DRIVER_COUNT: usize = 0x34;
/// `DUMP_DRIVER_ENTRY64`: name offset, then `KLDR_DATA_TABLE_ENTRY64` at 8.
const DRIVER_ENTRY_SIZE: usize = 0x90;
const D_BASE: usize = 8 + 0x30;
const D_SIZE: usize = 8 + 0x40;
/// Sanity limits for values read from the file.
const MAX_DRIVERS: usize = 4096;
const MAX_NAME_CHARS: usize = 128;
const MAX_STACK: usize = 64 << 10;

/// What kind of module a driver is, for choosing the culprit and the fix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriverKind {
    Graphics,
    Network,
    Storage,
    Audio,
    /// Another third-party driver (or one not in the table).
    Other,
    /// Part of Windows' storage stack (storport, stornvme, ...).
    WindowsStorage,
    /// Another Windows driver (network stack, DirectX kernel, ...).
    Windows,
    /// The Windows kernel itself and its core modules: almost never the real cause.
    Kernel,
}

impl DriverKind {
    /// Whether this is part of Windows rather than a vendor's driver.
    pub fn is_windows(self) -> bool {
        matches!(
            self,
            DriverKind::WindowsStorage | DriverKind::Windows | DriverKind::Kernel
        )
    }

    /// Lower is a more likely culprit.
    fn rank(self) -> u8 {
        match self {
            DriverKind::Kernel => 2,
            DriverKind::WindowsStorage | DriverKind::Windows => 1,
            _ => 0,
        }
    }
}

/// Well-known drivers (lowercase file name), what they belong to and their kind.
const KNOWN: &[(&str, &str, DriverKind)] = &[
    (
        "nvlddmkm.sys",
        "NVIDIA graphics driver",
        DriverKind::Graphics,
    ),
    ("amdkmdag.sys", "AMD graphics driver", DriverKind::Graphics),
    ("atikmdag.sys", "AMD graphics driver", DriverKind::Graphics),
    ("atikmpag.sys", "AMD graphics driver", DriverKind::Graphics),
    (
        "igdkmd64.sys",
        "Intel graphics driver",
        DriverKind::Graphics,
    ),
    (
        "igdkmdn64.sys",
        "Intel graphics driver",
        DriverKind::Graphics,
    ),
    (
        "igdkmdnd64.sys",
        "Intel graphics driver",
        DriverKind::Graphics,
    ),
    (
        "rt640x64.sys",
        "Realtek network driver",
        DriverKind::Network,
    ),
    ("rtwlane.sys", "Realtek Wi-Fi driver", DriverKind::Network),
    (
        "rtwlanu.sys",
        "Realtek USB Wi-Fi driver",
        DriverKind::Network,
    ),
    (
        "rtux64w10.sys",
        "Realtek USB network driver",
        DriverKind::Network,
    ),
    ("e1d68x64.sys", "Intel network driver", DriverKind::Network),
    (
        "e2f68.sys",
        "Intel 2.5G network driver",
        DriverKind::Network,
    ),
    ("netwtw08.sys", "Intel Wi-Fi driver", DriverKind::Network),
    ("netwtw10.sys", "Intel Wi-Fi driver", DriverKind::Network),
    ("netwtw14.sys", "Intel Wi-Fi driver", DriverKind::Network),
    (
        "athw8x.sys",
        "Qualcomm Atheros Wi-Fi driver",
        DriverKind::Network,
    ),
    ("bcmwl63a.sys", "Broadcom Wi-Fi driver", DriverKind::Network),
    (
        "iastorac.sys",
        "Intel RST storage driver",
        DriverKind::Storage,
    ),
    (
        "iastora.sys",
        "Intel RST storage driver",
        DriverKind::Storage,
    ),
    (
        "iastorav.sys",
        "Intel RST storage driver",
        DriverKind::Storage,
    ),
    ("rcraid.sys", "AMD RAID driver", DriverKind::Storage),
    ("secnvme.sys", "Samsung NVMe driver", DriverKind::Storage),
    ("rtkvhd64.sys", "Realtek audio driver", DriverKind::Audio),
    (
        "nvhda64v.sys",
        "NVIDIA HDMI audio driver",
        DriverKind::Audio,
    ),
    ("vgk.sys", "Riot Vanguard anti-cheat", DriverKind::Other),
    ("easyanticheat.sys", "Easy Anti-Cheat", DriverKind::Other),
    ("bedaisy.sys", "BattlEye anti-cheat", DriverKind::Other),
    (
        "storport.sys",
        "Windows storage stack",
        DriverKind::WindowsStorage,
    ),
    (
        "stornvme.sys",
        "Windows storage stack",
        DriverKind::WindowsStorage,
    ),
    (
        "storahci.sys",
        "Windows storage stack",
        DriverKind::WindowsStorage,
    ),
    (
        "disk.sys",
        "Windows storage stack",
        DriverKind::WindowsStorage,
    ),
    (
        "classpnp.sys",
        "Windows storage stack",
        DriverKind::WindowsStorage,
    ),
    ("ntfs.sys", "Windows file system", DriverKind::Windows),
    ("ndis.sys", "Windows network stack", DriverKind::Windows),
    ("tcpip.sys", "Windows network stack", DriverKind::Windows),
    ("netio.sys", "Windows network stack", DriverKind::Windows),
    (
        "dxgkrnl.sys",
        "Windows graphics kernel",
        DriverKind::Windows,
    ),
    (
        "dxgmms2.sys",
        "Windows graphics kernel",
        DriverKind::Windows,
    ),
    (
        "watchdog.sys",
        "Windows display watchdog",
        DriverKind::Windows,
    ),
    ("usbxhci.sys", "Windows USB driver", DriverKind::Windows),
    ("acpi.sys", "Windows ACPI driver", DriverKind::Windows),
    (
        "win32k.sys",
        "Windows graphics subsystem",
        DriverKind::Windows,
    ),
    (
        "win32kbase.sys",
        "Windows graphics subsystem",
        DriverKind::Windows,
    ),
    (
        "win32kfull.sys",
        "Windows graphics subsystem",
        DriverKind::Windows,
    ),
    ("ntoskrnl.exe", "Windows kernel", DriverKind::Kernel),
    ("ntkrnlmp.exe", "Windows kernel", DriverKind::Kernel),
    ("ntkrnlpa.exe", "Windows kernel", DriverKind::Kernel),
    ("ntkrpamp.exe", "Windows kernel", DriverKind::Kernel),
    ("hal.dll", "Windows kernel", DriverKind::Kernel),
    ("kdcom.dll", "Windows kernel", DriverKind::Kernel),
    ("pshed.dll", "Windows kernel", DriverKind::Kernel),
    ("ci.dll", "Windows kernel", DriverKind::Kernel),
    ("clfs.sys", "Windows kernel", DriverKind::Kernel),
    ("cng.sys", "Windows kernel", DriverKind::Kernel),
    ("ksecdd.sys", "Windows kernel", DriverKind::Kernel),
    ("wdf01000.sys", "Windows kernel", DriverKind::Kernel),
    ("fltmgr.sys", "Windows kernel", DriverKind::Kernel),
    ("tm.sys", "Windows kernel", DriverKind::Kernel),
];

/// Friendly name and kind of a driver file name; unknown drivers are [`DriverKind::Other`].
pub fn driver_info(module: &str) -> (Option<&'static str>, DriverKind) {
    KNOWN
        .iter()
        .find(|(n, _, _)| n.eq_ignore_ascii_case(module))
        .map_or((None, DriverKind::Other), |&(_, f, k)| (Some(f), k))
}

/// The module a crash points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Culprit {
    /// File name, e.g. `nvlddmkm.sys`.
    pub module: String,
    /// What it belongs to, e.g. "NVIDIA graphics driver", for well-known drivers.
    pub friendly: Option<&'static str>,
    pub kind: DriverKind,
}

impl Culprit {
    fn new(module: &str) -> Culprit {
        let (friendly, kind) = driver_info(module);
        Culprit {
            module: module.to_string(),
            friendly,
            kind,
        }
    }

    /// "nvlddmkm.sys (NVIDIA graphics driver)" or just the file name.
    pub fn label(&self) -> String {
        match self.friendly {
            Some(f) => format!("{} ({f})", self.module),
            None => self.module.clone(),
        }
    }
}

/// One crash dump's findings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DumpAnalysis {
    /// File name, e.g. `101226-12345-01.dmp`.
    pub file: String,
    /// (year, month, day) of the crash (UTC), from the header or else the file's date.
    pub date: Option<(i64, u32, u32)>,
    /// Stop (bugcheck) code.
    pub code: u32,
    pub params: [u64; 4],
    /// The driver the crash points at, if one could be found.
    pub culprit: Option<Culprit>,
    /// Why the file couldn't be analysed (the other fields are then empty).
    pub error: Option<String>,
}

/// A loaded driver from the triage dump's driver list.
#[derive(Debug, Clone)]
struct Driver {
    name: String,
    base: u64,
    size: u64,
}

impl Driver {
    fn contains(&self, addr: u64) -> bool {
        addr >= self.base && addr - self.base < self.size
    }
}

/// Analyse a kernel crash dump (`bytes` may be just its first [`MAX_READ`] bytes). Only
/// 64-bit dumps are supported; the culprit needs a small (triage) dump.
pub fn analyze(bytes: &[u8]) -> Result<DumpAnalysis, String> {
    if bytes.get(0..4) != Some(SIGNATURE) {
        return Err("Not a Windows kernel crash dump".into());
    }
    match bytes.get(4..8) {
        Some(v) if v == VALID_64 => {}
        Some(v) if v == VALID_32 => return Err("Unsupported dump (32-bit Windows)".into()),
        _ => return Err("Unsupported dump format".into()),
    }
    let too_short = || "Crash dump is truncated".to_string();
    let code = u32_at(bytes, BUGCHECK_CODE).ok_or_else(too_short)?;
    let mut params = [0u64; 4];
    for (i, p) in params.iter_mut().enumerate() {
        *p = u64_at(bytes, BUGCHECK_PARAMS + i * 8).ok_or_else(too_short)?;
    }
    let date = u64_at(bytes, SYSTEM_TIME).and_then(filetime_date);
    let culprit = (u32_at(bytes, DUMP_TYPE) == Some(DUMP_TYPE_TRIAGE))
        .then(|| culprit(bytes, &params))
        .flatten();
    Ok(DumpAnalysis {
        file: String::new(),
        date,
        code,
        params,
        culprit,
        error: None,
    })
}

/// A u32 file offset/size from the triage header, as usize.
fn triage_u32(b: &[u8], field: usize) -> Option<usize> {
    u32_at(b, TRIAGE + field).map(|v| v as usize)
}

/// The triage dump's driver list (empty if missing or malformed).
fn drivers(b: &[u8]) -> Vec<Driver> {
    let (Some(off), Some(count)) = (
        triage_u32(b, T_DRIVER_LIST_OFFSET),
        triage_u32(b, T_DRIVER_COUNT),
    ) else {
        return Vec::new();
    };
    if off == 0 || count > MAX_DRIVERS {
        return Vec::new();
    }
    (0..count)
        .map_while(|i| {
            let e = off.checked_add(i * DRIVER_ENTRY_SIZE)?;
            b.get(e..e.checked_add(DRIVER_ENTRY_SIZE)?)?;
            Some((
                u32_at(b, e)? as usize,
                u64_at(b, e + D_BASE)?,
                u32_at(b, e + D_SIZE)?,
            ))
        })
        .filter_map(|(name_off, base, size)| {
            let name = dump_string(b, name_off)?;
            (size > 0).then(|| Driver {
                name,
                base,
                size: u64::from(size),
            })
        })
        .collect()
}

/// A `DUMP_STRING` (u32 length, then NUL-terminated UTF-16) at `off`: the file name part,
/// printable characters only. The length is read as a cap only, since sources disagree on
/// whether it counts bytes or characters; the terminating NUL ends the name.
fn dump_string(b: &[u8], off: usize) -> Option<String> {
    let len = u32_at(b, off)? as usize;
    let start = off.checked_add(4)?;
    let units: Vec<u16> = (0..len.min(MAX_NAME_CHARS))
        .map_while(|i| {
            let o = start.checked_add(i * 2)?;
            let c = u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?);
            (c != 0).then_some(c)
        })
        .collect();
    let s = String::from_utf16_lossy(&units);
    let name = s.rsplit(['\\', '/']).next().unwrap_or("");
    let clean: String = name.chars().filter(|c| !c.is_control()).collect();
    (!clean.trim().is_empty()).then(|| clean.trim().to_string())
}

/// The driver the crash points at. Candidate addresses, most direct first: the instruction
/// pointer and exception address, then the bugcheck parameters, then return addresses on the
/// saved call stack. A vendor driver hit directly wins, then a Windows driver hit directly,
/// then the same from the stack; the kernel itself only when nothing else matches.
fn culprit(b: &[u8], params: &[u64; 4]) -> Option<Culprit> {
    let drivers = drivers(b);
    if drivers.is_empty() {
        return None;
    }
    let mut direct: Vec<u64> = Vec::new();
    let mut push = |v: Option<u64>| {
        if let Some(v) = v.filter(|&v| v != 0)
            && !direct.contains(&v)
        {
            direct.push(v);
        }
    };
    push(u64_at(b, CONTEXT + CONTEXT_RIP));
    push(u64_at(b, EXCEPTION + EXCEPTION_ADDRESS));
    for (field, inner) in [
        (T_CONTEXT_OFFSET, CONTEXT_RIP),
        (T_EXCEPTION_OFFSET, EXCEPTION_ADDRESS),
    ] {
        if let Some(o) = triage_u32(b, field).filter(|&o| o != 0) {
            push(o.checked_add(inner).and_then(|o| u64_at(b, o)));
        }
    }
    for &p in params {
        push(Some(p));
    }
    let stack: Vec<u64> = match (
        triage_u32(b, T_CALL_STACK_OFFSET),
        triage_u32(b, T_CALL_STACK_SIZE),
    ) {
        (Some(off), Some(size)) if off != 0 => {
            let end = off.saturating_add(size.min(MAX_STACK)).min(b.len());
            b.get(off..end)
                .unwrap_or(&[])
                .chunks_exact(8)
                .map(|c| u64::from_le_bytes(c.try_into().unwrap_or_default()))
                .collect()
        }
        _ => Vec::new(),
    };
    // (priority, order) of the best hit so far; lower is better.
    let mut best: Option<((u8, u8), &Driver)> = None;
    for (phase, addrs) in [(0u8, &direct), (1u8, &stack)] {
        for &a in addrs.iter() {
            let Some(d) = drivers.iter().find(|d| d.contains(a)) else {
                continue;
            };
            let rank = driver_info(&d.name).1.rank();
            let key = if rank == 2 { (2, phase) } else { (phase, rank) };
            if best.is_none_or(|(k, _)| key < k) {
                best = Some((key, d));
            }
        }
    }
    best.map(|(_, d)| Culprit::new(&d.name))
}

/// (year, month, day) of a Windows FILETIME (100 ns ticks since 1601), if plausible.
fn filetime_date(ft: u64) -> Option<(i64, u32, u32)> {
    const EPOCH_DIFF: i64 = 11_644_473_600;
    let secs = (ft / 10_000_000) as i64 - EPOCH_DIFF;
    let d = crate::civil_from_days(secs.div_euclid(86_400));
    (2000..2200).contains(&d.0).then_some(d)
}

fn system_time_date(t: SystemTime) -> Option<(i64, u32, u32)> {
    let secs = t.duration_since(UNIX_EPOCH).ok()?.as_secs() as i64;
    Some(crate::civil_from_days(secs / 86_400))
}

/// Where Windows writes small memory dumps: `%SystemRoot%\Minidump`.
pub fn dump_folder() -> PathBuf {
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
    Path::new(&root).join("Minidump")
}

/// Analyse the newest `max` `.dmp` files in `dir`, newest first. A missing folder means no
/// dumps; a file that can't be read or parsed is listed with its error.
pub fn read_folder(dir: &Path, max: usize) -> Result<Vec<DumpAnalysis>, String> {
    use std::io::Read;
    if matches!(dir.try_exists(), Ok(false)) {
        return Ok(Vec::new());
    }
    let entries =
        std::fs::read_dir(dir).map_err(|e| format!("Couldn't read {}: {e}", dir.display()))?;
    let mut files: Vec<(SystemTime, PathBuf)> = entries
        .filter_map(Result::ok)
        .filter(|e| {
            e.path()
                .extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("dmp"))
        })
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            m.is_file()
                .then(|| (m.modified().unwrap_or(UNIX_EPOCH), e.path()))
        })
        .collect();
    files.sort_by_key(|f| std::cmp::Reverse(f.0));
    files.truncate(max);
    Ok(files
        .into_iter()
        .map(|(modified, path)| {
            let mut bytes = Vec::new();
            let read = std::fs::File::open(&path)
                .and_then(|f| f.take(MAX_READ).read_to_end(&mut bytes))
                .map_err(|e| format!("Couldn't read the file: {e}"));
            let mut a = read
                .and_then(|_| analyze(&bytes))
                .unwrap_or_else(|e| DumpAnalysis {
                    error: Some(e),
                    ..Default::default()
                });
            a.file = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            a.date = a.date.or_else(|| system_time_date(modified));
            a
        })
        .collect())
}

/// Tabs and line breaks would break the text encoding.
fn field(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

/// Encode results as text, one dump per line (for the elevated helper's result file).
pub fn to_text(dumps: &[DumpAnalysis]) -> String {
    let mut out = String::new();
    for d in dumps {
        let date = d
            .date
            .map(|(y, m, d)| format!("{y}-{m:02}-{d:02}"))
            .unwrap_or_default();
        let p = d.params.map(|p| format!("{p:X}"));
        let module = d.culprit.as_ref().map(|c| field(&c.module));
        out.push_str(&format!(
            "{}\t{date}\t{:X}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            field(&d.file),
            d.code,
            p[0],
            p[1],
            p[2],
            p[3],
            module.unwrap_or_default(),
            d.error.as_deref().map(field).unwrap_or_default()
        ));
    }
    out
}

/// Decode [`to_text`] output; malformed lines are skipped.
pub fn from_text(text: &str) -> Vec<DumpAnalysis> {
    text.lines()
        .take(MAX_DUMPS * 5)
        .filter_map(|line| {
            let f: Vec<&str> = line.split('\t').collect();
            let [file, date, code, p0, p1, p2, p3, module, error] = f.as_slice() else {
                return None;
            };
            let hex = |s: &str| u64::from_str_radix(s, 16).ok();
            let date = if date.is_empty() {
                None
            } else {
                let mut it = date.split('-');
                Some((
                    it.next()?.parse().ok()?,
                    it.next()?.parse().ok()?,
                    it.next()?.parse().ok()?,
                ))
            };
            Some(DumpAnalysis {
                file: file.to_string(),
                date,
                code: u32::from_str_radix(code, 16).ok()?,
                params: [hex(p0)?, hex(p1)?, hex(p2)?, hex(p3)?],
                culprit: (!module.is_empty()).then(|| Culprit::new(module)),
                error: (!error.is_empty()).then(|| error.to_string()),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NV_BASE: u64 = 0xFFFF_F800_1000_0000;
    const NT_BASE: u64 = 0xFFFF_F800_0200_0000;

    fn put32(b: &mut [u8], o: usize, v: u32) {
        b[o..o + 4].copy_from_slice(&v.to_le_bytes());
    }

    fn put64(b: &mut [u8], o: usize, v: u64) {
        b[o..o + 8].copy_from_slice(&v.to_le_bytes());
    }

    /// A 64-bit triage dump with these drivers (name, base, size), RIP and stack values.
    fn dump(
        code: u32,
        params: [u64; 4],
        drivers: &[(&str, u64, u32)],
        rip: u64,
        stack: &[u64],
    ) -> Vec<u8> {
        let list = 0x3000;
        let pool = list + drivers.len() * DRIVER_ENTRY_SIZE;
        let stack_off = 0x6000;
        let mut b = vec![0u8; 0x8000];
        b[0..4].copy_from_slice(SIGNATURE);
        b[4..8].copy_from_slice(VALID_64);
        put32(&mut b, BUGCHECK_CODE, code);
        for (i, p) in params.iter().enumerate() {
            put64(&mut b, BUGCHECK_PARAMS + i * 8, *p);
        }
        put64(&mut b, CONTEXT + CONTEXT_RIP, rip);
        put32(&mut b, DUMP_TYPE, DUMP_TYPE_TRIAGE);
        // 2026-10-12 00:00 UTC as a FILETIME.
        put64(
            &mut b,
            SYSTEM_TIME,
            (20_738 * 86_400 + 11_644_473_600) * 10_000_000,
        );
        put32(&mut b, TRIAGE + T_CONTEXT_OFFSET, CONTEXT as u32);
        put32(&mut b, TRIAGE + T_EXCEPTION_OFFSET, EXCEPTION as u32);
        put32(&mut b, TRIAGE + T_CALL_STACK_OFFSET, stack_off as u32);
        put32(&mut b, TRIAGE + T_CALL_STACK_SIZE, (stack.len() * 8) as u32);
        put32(&mut b, TRIAGE + T_DRIVER_LIST_OFFSET, list as u32);
        put32(&mut b, TRIAGE + T_DRIVER_COUNT, drivers.len() as u32);
        let mut s = pool;
        for (i, (name, base, size)) in drivers.iter().enumerate() {
            let e = list + i * DRIVER_ENTRY_SIZE;
            put32(&mut b, e, s as u32);
            put64(&mut b, e + D_BASE, *base);
            put32(&mut b, e + D_SIZE, *size);
            let units: Vec<u16> = name.encode_utf16().collect();
            put32(&mut b, s, (units.len() * 2) as u32);
            for (j, u) in units.iter().enumerate() {
                b[s + 4 + j * 2..s + 6 + j * 2].copy_from_slice(&u.to_le_bytes());
            }
            s += 4 + units.len() * 2 + 2;
        }
        for (i, v) in stack.iter().enumerate() {
            put64(&mut b, stack_off + i * 8, *v);
        }
        b
    }

    fn drivers_list() -> Vec<(&'static str, u64, u32)> {
        vec![
            ("ntoskrnl.exe", NT_BASE, 0x100_0000),
            (
                r"\SystemRoot\System32\drivers\nvlddmkm.sys",
                NV_BASE,
                0x400_0000,
            ),
            ("rt640x64.sys", 0xFFFF_F800_2000_0000, 0x10_0000),
        ]
    }

    #[test]
    fn culprit_from_parameter_beats_kernel_rip() {
        let b = dump(
            0x133,
            [1, 0x1E00, NV_BASE + 0x1234, 0],
            &drivers_list(),
            NT_BASE + 0x500,
            &[],
        );
        let a = analyze(&b).unwrap();
        assert_eq!(a.code, 0x133);
        assert_eq!(a.params, [1, 0x1E00, NV_BASE + 0x1234, 0]);
        assert_eq!(a.date, Some((2026, 10, 12)));
        let c = a.culprit.unwrap();
        assert_eq!(c.module, "nvlddmkm.sys");
        assert_eq!(c.kind, DriverKind::Graphics);
        assert_eq!(c.label(), "nvlddmkm.sys (NVIDIA graphics driver)");
    }

    #[test]
    fn culprit_from_stack_and_kernel_only() {
        // Nothing direct outside the kernel: a vendor driver on the stack is the culprit.
        let b = dump(
            0x9F,
            [3, 0, 0, 0],
            &drivers_list(),
            NT_BASE + 0x500,
            &[5, NT_BASE + 0x10, 0xFFFF_F800_2000_0042],
        );
        assert_eq!(analyze(&b).unwrap().culprit.unwrap().module, "rt640x64.sys");
        // Only the kernel matches: reported as the kernel.
        let b = dump(
            0x1A,
            [0x41790, 0, 0, 0],
            &drivers_list(),
            NT_BASE + 0x500,
            &[7],
        );
        let c = analyze(&b).unwrap().culprit.unwrap();
        assert_eq!(
            (c.module.as_str(), c.kind),
            ("ntoskrnl.exe", DriverKind::Kernel)
        );
        // No address in any driver.
        let b = dump(0x1A, [0; 4], &drivers_list(), 0x1234, &[]);
        assert_eq!(analyze(&b).unwrap().culprit, None);
    }

    #[test]
    fn rejects_other_formats_and_survives_garbage() {
        assert!(analyze(b"").is_err());
        assert!(analyze(b"MDMP\x93\xa7\0\0").is_err());
        let e = analyze(b"PAGEDUMP\0\0\0\0").unwrap_err();
        assert!(e.contains("32-bit"), "{e}");
        assert!(analyze(b"PAGEDU64").is_err());
        // Truncated anywhere: never panics.
        let full = dump(
            0x133,
            [1, 2, NV_BASE, 4],
            &drivers_list(),
            NT_BASE,
            &[NV_BASE],
        );
        for n in (0..full.len()).step_by(61) {
            let _ = analyze(&full[..n]);
        }
        // Header only: code and parameters, no culprit.
        let a = analyze(&full[..0x1000]).unwrap();
        assert_eq!((a.code, a.culprit), (0x133, None));
        // Huge counts, sizes and offsets are bounded.
        let mut bad = full.clone();
        put32(&mut bad, TRIAGE + T_DRIVER_COUNT, u32::MAX);
        put32(&mut bad, TRIAGE + T_CALL_STACK_SIZE, u32::MAX);
        assert_eq!(analyze(&bad).unwrap().culprit, None);
        put32(&mut bad, TRIAGE + T_DRIVER_COUNT, 3);
        put32(&mut bad, TRIAGE + T_DRIVER_LIST_OFFSET, u32::MAX - 4);
        put32(&mut bad, TRIAGE + T_CALL_STACK_OFFSET, u32::MAX - 4);
        assert_eq!(analyze(&bad).unwrap().culprit, None);
        let mut bad = full.clone();
        put32(
            &mut bad,
            0x3000 + DRIVER_ENTRY_SIZE,
            (full.len() - 2) as u32,
        );
        // NVIDIA's name is unreadable: its entry is skipped, the kernel still matches.
        assert_eq!(
            analyze(&bad).unwrap().culprit.unwrap().module,
            "ntoskrnl.exe"
        );
        // Random bytes after a valid signature.
        let mut x: u32 = 12345;
        let mut rnd = full.clone();
        for v in rnd[8..].iter_mut() {
            x = x.wrapping_mul(1_103_515_245).wrapping_add(12345);
            *v = (x >> 16) as u8;
        }
        let _ = analyze(&rnd);
    }

    #[test]
    fn text_round_trip() {
        let dumps = vec![
            DumpAnalysis {
                file: "101226-1-01.dmp".into(),
                date: Some((2026, 10, 12)),
                code: 0x133,
                params: [1, 0x1E00, u64::MAX, 0],
                culprit: Some(Culprit::new("nvlddmkm.sys")),
                error: None,
            },
            DumpAnalysis {
                file: "bad\tname.dmp".into(),
                error: Some("Crash dump\nis truncated".into()),
                ..Default::default()
            },
        ];
        let back = from_text(&to_text(&dumps));
        assert_eq!(back[0], dumps[0]);
        assert_eq!(back[1].file, "bad name.dmp");
        assert_eq!(back[1].error.as_deref(), Some("Crash dump is truncated"));
        assert!(from_text("garbage\nx\ty").is_empty());
    }

    #[test]
    fn reads_a_folder() {
        let dir = std::env::temp_dir().join(format!("omnihw-minidump-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(read_folder(&dir, MAX_DUMPS), Ok(Vec::new()));
        std::fs::create_dir_all(&dir).unwrap();
        let b = dump(0x116, [0, NV_BASE + 8, 0, 0], &drivers_list(), 0, &[]);
        std::fs::write(dir.join("a.dmp"), &b).unwrap();
        std::fs::write(dir.join("b.dmp"), b"junk").unwrap();
        std::fs::write(dir.join("c.txt"), &b).unwrap();
        let mut r = read_folder(&dir, MAX_DUMPS).unwrap();
        r.sort_by(|a, b| a.file.cmp(&b.file));
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].culprit.as_ref().unwrap().module, "nvlddmkm.sys");
        assert!(r[1].error.is_some() && r[1].date.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
