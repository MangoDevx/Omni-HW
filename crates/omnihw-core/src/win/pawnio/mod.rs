//! Client for the optional PawnIO driver (<https://pawnio.eu>), which runs small signed modules
//! in the kernel to read MSRs, PCI/SMN registers and Super I/O ports.
//!
//! Omni-HW talks to it only through its device IO control interface (load a module blob, call
//! one of its `ioctl_*` functions), which the driver's GPL special exception covers. The module
//! blobs are the unmodified official signed release, embedded from `crates/omnihw-core/pawnio`.
//! The device is restricted to administrators, so the backends here run only inside the
//! elevated sensor service; the app uses [`ServiceBackend`] and [`service_status`], which just
//! read the service's pipe.

mod client;
mod cpu;
mod pipe;
mod superio;

pub use client::{ServiceBackend, service_installed, service_status};
pub use cpu::CpuMsrBackend;
pub use pipe::{serve, wake};
pub use superio::SuperIoBackend;

use super::device::Handle;
use super::registry::{self, Hive};
use crate::lowlevel::{PawnioState as State, PawnioStatus as Status};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_ACCESS_DENIED, GENERIC_READ, GENERIC_WRITE, HANDLE, WAIT_ABANDONED,
    WAIT_OBJECT_0,
};
use windows::Win32::System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject};
use windows::core::HSTRING;

pub(crate) const INTEL_MSR: &[u8] = include_bytes!("../../../pawnio/IntelMSR.bin");
pub(crate) const AMD_FAMILY_17: &[u8] = include_bytes!("../../../pawnio/AMDFamily17.bin");
pub(crate) const LPC_IO: &[u8] = include_bytes!("../../../pawnio/LpcIO.bin");

/// `\Device\PawnIO`, reached from user mode through the GLOBALROOT namespace.
const DEVICE_PATH: &str = r"\\?\GLOBALROOT\Device\PawnIO";
const UNINSTALL_KEY: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\PawnIO";
const SERVICE_KEY: &str = r"SYSTEM\CurrentControlSet\Services\PawnIO";

const DEVICE_TYPE: u32 = 41394;
/// `CTL_CODE(DEVICE_TYPE, function, METHOD_BUFFERED, FILE_ANY_ACCESS)`.
const fn ctl_code(function: u32) -> u32 {
    DEVICE_TYPE << 16 | function << 2
}
const IOCTL_LOAD_BINARY: u32 = ctl_code(0x821);
const IOCTL_EXECUTE_FN: u32 = ctl_code(0x841);
/// Module function names are passed in a fixed 32-byte, NUL-padded field.
const FN_NAME_LEN: usize = 32;

/// Check the installation (registry) and try to open the device. Cheap enough to call on
/// demand, but not every frame.
pub fn status() -> Status {
    let version = version();
    let state = match open_device() {
        Ok(_) => State::Ready,
        Err(_) if !installed() => State::NotInstalled,
        Err(e) if e.code() == ERROR_ACCESS_DENIED.to_hresult() => State::NeedsAdmin,
        Err(e) => State::NotRunning(e.message()),
    };
    Status { state, version }
}

/// Installed driver version, e.g. "2.2.0" (registered as "2.2.0.0").
pub fn version() -> Option<String> {
    let v = registry::string(Hive::LocalMachine, UNINSTALL_KEY, "DisplayVersion")?;
    Some(match v.strip_suffix(".0") {
        Some(t) if t.matches('.').count() == 2 => t.to_string(),
        _ => v,
    })
}

/// Whether the PawnIO driver is installed (registry only; doesn't open the device).
pub fn installed() -> bool {
    registry::key_exists(Hive::LocalMachine, UNINSTALL_KEY)
        || registry::key_exists(Hive::LocalMachine, SERVICE_KEY)
}

/// The installer's uninstall command line, as registered with Windows.
pub fn uninstall_command() -> Option<String> {
    registry::string(Hive::LocalMachine, UNINSTALL_KEY, "UninstallString")
}

fn open_device() -> windows::core::Result<Handle> {
    Handle::try_open(DEVICE_PATH, (GENERIC_READ | GENERIC_WRITE).0)
}

/// A device handle with one module loaded (the driver keeps one module per handle).
pub(crate) struct Module {
    handle: Handle,
    name: &'static str,
}

impl Module {
    pub(crate) fn load(name: &'static str, blob: &[u8]) -> Result<Module, String> {
        let handle = open_device().map_err(|e| format!("PawnIO: {}", e.message()))?;
        handle
            .try_ioctl(IOCTL_LOAD_BINARY, blob, &mut [])
            .map_err(|e| format!("{name}: {}", e.message()))?;
        Ok(Module { handle, name })
    }

    /// Call module function `func` with `input`, filling `out`; returns how many values the
    /// function wrote.
    pub(crate) fn call(&self, func: &str, input: &[u64], out: &mut [u64]) -> Result<usize, String> {
        // The name must fit its field with a terminating NUL.
        if func.len() >= FN_NAME_LEN {
            return Err(format!("{}: function name too long: {func}", self.name));
        }
        let mut buf = vec![0u8; FN_NAME_LEN + input.len() * 8];
        buf[..func.len()].copy_from_slice(func.as_bytes());
        for (chunk, v) in buf[FN_NAME_LEN..].chunks_exact_mut(8).zip(input) {
            chunk.copy_from_slice(&v.to_le_bytes());
        }
        let mut raw = vec![0u8; out.len() * 8];
        let got = self
            .handle
            .try_ioctl(IOCTL_EXECUTE_FN, buf.as_slice(), &mut raw)
            .map_err(|e| format!("{} {func}: {}", self.name, e.message()))?;
        for (v, chunk) in out.iter_mut().zip(raw.chunks_exact(8)) {
            *v = u64::from_le_bytes(chunk.try_into().unwrap_or_default());
        }
        Ok(got / 8)
    }

    /// Call a function taking and returning one value.
    pub(crate) fn call1(&self, func: &str, arg: u64) -> Result<u64, String> {
        let mut out = [0];
        self.call(func, &[arg], &mut out)?;
        Ok(out[0])
    }
}

/// A named mutex other hardware monitors (HWiNFO, AIDA64, LibreHardwareMonitor) also take
/// around the same hardware, so two tools never interleave multi-step register accesses.
pub(crate) struct GlobalMutex(HANDLE);

// SAFETY: a mutex handle is a process-wide kernel object reference usable from any thread.
// `with` waits and releases on the same thread, as mutex ownership requires.
unsafe impl Send for GlobalMutex {}

/// Super I/O (LPC/ISA port) access.
pub(crate) const ISA_BUS_MUTEX: &str = r"Global\Access_ISABUS.HTP.Method";
/// PCI configuration space access (AMD SMN reads go through it).
pub(crate) const PCI_MUTEX: &str = r"Global\Access_PCI";

impl GlobalMutex {
    pub(crate) fn open(name: &str) -> Option<GlobalMutex> {
        // SAFETY: the name is a NUL-terminated HSTRING that outlives the call; default security.
        unsafe { CreateMutexW(None, false, &HSTRING::from(name)) }
            .ok()
            .map(GlobalMutex)
    }

    /// Run `f` holding the mutex; `None` if another tool held it for longer than `timeout_ms`.
    pub(crate) fn with<T>(&self, timeout_ms: u32, f: impl FnOnce() -> T) -> Option<T> {
        // SAFETY: `self.0` is a live mutex handle owned by this struct.
        let w = unsafe { WaitForSingleObject(self.0, timeout_ms) };
        if w != WAIT_OBJECT_0 && w != WAIT_ABANDONED {
            return None;
        }
        let r = f();
        // SAFETY: this thread owns the mutex after the successful wait above.
        let _ = unsafe { ReleaseMutex(self.0) };
        Some(r)
    }
}

impl Drop for GlobalMutex {
    fn drop(&mut self) {
        // SAFETY: the handle came from CreateMutexW and is closed exactly once.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

/// Run `f` with a mutex if one could be opened, or directly if not (`None` only on timeout).
pub(crate) fn locked<T>(m: Option<&GlobalMutex>, f: impl FnOnce() -> T) -> Option<T> {
    match m {
        Some(m) => m.with(20, f),
        None => Some(f()),
    }
}
