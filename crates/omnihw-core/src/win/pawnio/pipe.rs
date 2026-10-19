//! The sensor service's named pipe. The server only ever writes: each client that connects
//! gets one response and the server closes its end; nothing a client sends is read (the pipe is
//! outbound-only). Unprivileged users may connect for reading only.

use crate::sensor_service::{MAX_RESPONSE, PIPE_NAME};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_BROKEN_PIPE, ERROR_FILE_NOT_FOUND, ERROR_HANDLE_EOF, ERROR_IO_PENDING,
    ERROR_MORE_DATA, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, ERROR_PIPE_NOT_CONNECTED, GENERIC_READ,
    HANDLE, INVALID_HANDLE_VALUE, WAIT_TIMEOUT, WIN32_ERROR,
};
use windows::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, FILE_SHARE_NONE,
    OPEN_EXISTING, PIPE_ACCESS_OUTBOUND, ReadFile, WriteFile,
};
use windows::Win32::System::IO::{
    CancelIoEx, GetOverlappedResult, GetOverlappedResultEx, OVERLAPPED,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, GetNamedPipeServerSessionId, PIPE_REJECT_REMOTE_CLIENTS,
    PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT, WaitNamedPipeW,
};
use windows::Win32::System::Threading::CreateEventW;
use windows::core::HSTRING;

/// SYSTEM and administrators: full control. Authenticated users: read only, so they can't
/// write to the pipe or create instances of it (which needs write access).
const PIPE_SDDL: &str = "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GR;;;AU)";
const OUT_BUFFER: u32 = 256 * 1024;

/// An owned kernel handle, closed on drop.
struct Owned(HANDLE);

impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: a valid handle owned by this struct, closed once.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

/// Answer clients until `stop` is set (then call [`wake`] to unblock the wait), one at a time:
/// each gets `respond()`'s text.
pub fn serve(stop: &AtomicBool, mut respond: impl FnMut() -> String) -> Result<(), String> {
    let mut sd = PSECURITY_DESCRIPTOR::default();
    // SAFETY: the SDDL string outlives the call; `sd` receives a LocalAlloc'd descriptor that is
    // kept for the life of the server (it's needed for every pipe instance).
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            &HSTRING::from(PIPE_SDDL),
            SDDL_REVISION_1,
            &mut sd,
            None,
        )
    }
    .map_err(|e| format!("pipe security: {}", msg(&e)))?;
    let sa = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: sd.0,
        bInheritHandle: false.into(),
    };
    let mut first = true;
    while !stop.load(Ordering::Relaxed) {
        // The first instance must be ours: fail rather than join a pipe someone else created.
        let mode = if first {
            PIPE_ACCESS_OUTBOUND | FILE_FLAG_FIRST_PIPE_INSTANCE
        } else {
            PIPE_ACCESS_OUTBOUND
        };
        // SAFETY: name and security attributes outlive the call.
        let h = unsafe {
            CreateNamedPipeW(
                &HSTRING::from(PIPE_NAME),
                mode,
                PIPE_TYPE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                PIPE_UNLIMITED_INSTANCES,
                OUT_BUFFER,
                0,
                0,
                Some(&sa),
            )
        };
        if h == INVALID_HANDLE_VALUE {
            let e = windows::core::Error::from_thread();
            if first {
                return Err(format!("couldn't create the pipe: {}", msg(&e)));
            }
            std::thread::sleep(Duration::from_millis(200));
            continue;
        }
        first = false;
        let pipe = Owned(h);
        // SAFETY: a valid pipe handle; blocking wait for a client.
        let connected = match unsafe { ConnectNamedPipe(pipe.0, None) } {
            Ok(()) => true,
            Err(e) => e.code() == ERROR_PIPE_CONNECTED.to_hresult(),
        };
        if !connected || stop.load(Ordering::Relaxed) {
            continue;
        }
        // The response fits the pipe buffer, so this never waits on the client. Closing the
        // handle (rather than disconnecting) leaves the data readable, then the client sees the
        // end of the pipe; a client that never reads can't hold the server up.
        write_all(&pipe, respond().as_bytes());
    }
    Ok(())
}

fn write_all(pipe: &Owned, mut data: &[u8]) {
    while !data.is_empty() {
        let mut n = 0;
        // SAFETY: valid pipe; `data` lives for the synchronous call.
        if unsafe { WriteFile(pipe.0, Some(data), Some(&mut n), None) }.is_err() || n == 0 {
            return;
        }
        data = &data[n as usize..];
    }
}

/// Connect once and drop the connection, so a server blocked waiting for a client notices
/// `stop`.
pub fn wake() {
    let _ = open(Duration::from_millis(100));
}

fn open(wait: Duration) -> Result<Owned, windows::core::Error> {
    let name = HSTRING::from(PIPE_NAME);
    for attempt in 0..2 {
        // SAFETY: the name outlives the call; read-only overlapped client handle.
        match unsafe {
            CreateFileW(
                &name,
                GENERIC_READ.0,
                FILE_SHARE_NONE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED,
                None,
            )
        } {
            Ok(h) => return Ok(Owned(h)),
            Err(e) if attempt == 0 && e.code() == ERROR_PIPE_BUSY.to_hresult() => {
                // SAFETY: the name outlives the call.
                let _ = unsafe { WaitNamedPipeW(&name, wait.as_millis() as u32) };
            }
            // Between two clients the server may briefly have no instance listening.
            Err(e) if attempt == 0 && e.code() == ERROR_FILE_NOT_FOUND.to_hresult() => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(e),
        }
    }
    Err(ERROR_PIPE_BUSY.into())
}

/// Fetch one response from the service, giving up after `timeout`.
pub fn fetch(timeout: Duration) -> Result<String, String> {
    let started = Instant::now();
    let pipe = open(timeout).map_err(|e| {
        if e.code() == ERROR_FILE_NOT_FOUND.to_hresult() {
            "the sensor service isn't running".to_string()
        } else {
            format!("couldn't reach the sensor service: {}", msg(&e))
        }
    })?;
    // Services run in session 0; a pipe served from a user session is an impostor.
    let mut session = u32::MAX;
    // SAFETY: valid pipe handle and out pointer.
    if unsafe { GetNamedPipeServerSessionId(pipe.0, &mut session) }.is_err() || session != 0 {
        return Err("the sensor pipe isn't served by the Omni-HW service".into());
    }
    let event = Owned(
        // SAFETY: a manual-reset, initially unsignalled event for the overlapped reads.
        unsafe { CreateEventW(None, true, false, None) }.map_err(|e| e.message().to_string())?,
    );
    let mut data = Vec::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let left = timeout.saturating_sub(started.elapsed());
        let mut ov = OVERLAPPED {
            hEvent: event.0,
            ..Default::default()
        };
        let mut n = 0u32;
        // SAFETY: `buf` and `ov` outlive the I/O: on every path below the read has completed
        // (or been cancelled and waited for) before they go out of scope.
        let started_read = unsafe { ReadFile(pipe.0, Some(&mut buf), None, Some(&mut ov)) };
        let result = match started_read {
            // SAFETY: the read completed synchronously; `ov` describes it.
            Ok(()) => unsafe { GetOverlappedResult(pipe.0, &ov, &mut n, false) },
            Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => {
                // SAFETY: waits on the pending read described by `ov`.
                match unsafe {
                    GetOverlappedResultEx(pipe.0, &ov, &mut n, left.as_millis() as u32, false)
                } {
                    Err(e) if e.code() == WIN32_ERROR(WAIT_TIMEOUT.0).to_hresult() => {
                        // SAFETY: cancel the pending read, then wait until it has really
                        // stopped writing into `buf` before returning.
                        unsafe {
                            let _ = CancelIoEx(pipe.0, Some(&ov));
                            let _ = GetOverlappedResult(pipe.0, &ov, &mut n, true);
                        }
                        return Err("the sensor service didn't answer in time".into());
                    }
                    r => r,
                }
            }
            Err(e) => Err(e),
        };
        match result {
            Ok(()) if n == 0 => break,
            Ok(()) => {}
            Err(e) if e.code() == ERROR_MORE_DATA.to_hresult() => {}
            Err(e)
                // The server hangs up once everything is read: end of data.
                if e.code() == ERROR_BROKEN_PIPE.to_hresult()
                    || e.code() == ERROR_PIPE_NOT_CONNECTED.to_hresult()
                    || e.code() == ERROR_HANDLE_EOF.to_hresult() =>
            {
                break;
            }
            Err(e) => return Err(format!("reading from the sensor service: {}", msg(&e))),
        }
        data.extend_from_slice(&buf[..n as usize]);
        if data.len() > MAX_RESPONSE {
            return Err("the sensor service sent too much data".into());
        }
    }
    String::from_utf8(data).map_err(|_| "the sensor service sent invalid text".into())
}

/// A Windows error message without its trailing period, to embed in a sentence.
fn msg(e: &windows::core::Error) -> String {
    e.message().trim_end_matches(['.', ' ']).to_string()
}
