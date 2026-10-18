//! Disk speed test I/O: unbuffered, write-through overlapped reads and writes on one temporary
//! file through an I/O completion port.

use crate::disktest::{Phase, Progress, Speeds};
use std::alloc::{Layout, alloc, dealloc};
use std::path::Path;
use std::ptr;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_IO_PENDING, GENERIC_READ, GENERIC_WRITE, HANDLE,
};
use windows::Win32::Storage::FileSystem::{
    CREATE_ALWAYS, CreateFileW, DELETE, DeleteFileW, FILE_ATTRIBUTE_TEMPORARY, FILE_BEGIN,
    FILE_FLAG_DELETE_ON_CLOSE, FILE_FLAG_NO_BUFFERING, FILE_FLAG_OVERLAPPED,
    FILE_FLAG_WRITE_THROUGH, FILE_SHARE_MODE, GetDiskFreeSpaceExW, GetDiskFreeSpaceW, ReadFile,
    SetEndOfFile, SetFilePointerEx, WriteFile,
};
use windows::Win32::System::IO::{
    CancelIoEx, CreateIoCompletionPort, GetQueuedCompletionStatus, OVERLAPPED, OVERLAPPED_0,
    OVERLAPPED_0_0,
};
use windows::core::{HSTRING, Owned};

const SEQ_BLOCK: usize = 1 << 20;
const RND_BLOCK: usize = 4096;
/// How long to wait for any one I/O before giving up on the test.
const IO_TIMEOUT_MS: u32 = 30_000;

/// A zeroed heap buffer aligned for unbuffered I/O, freed on drop.
struct AlignedBuf {
    ptr: *mut u8,
    layout: Layout,
}

impl AlignedBuf {
    fn new(len: usize, align: usize) -> Result<AlignedBuf, String> {
        let layout = Layout::from_size_align(len, align).map_err(|e| e.to_string())?;
        // SAFETY: `layout` has a non-zero size (callers never ask for 0 bytes).
        let ptr = unsafe { alloc(layout) };
        if ptr.is_null() {
            return Err("Out of memory for the test buffers".into());
        }
        Ok(AlignedBuf { ptr, layout })
    }

    /// Fill with non-zero pseudo-random bytes so compressing controllers can't shortcut it.
    fn fill_random(&mut self, rng: &mut XorShift) {
        for i in (0..self.layout.size()).step_by(8) {
            let v = rng.next() | 0x0101_0101_0101_0101;
            // SAFETY: the size is a multiple of 4096, so 8 bytes from `i` stay in bounds.
            unsafe { ptr::write_unaligned(self.ptr.add(i).cast::<u64>(), v) };
        }
    }
}

impl Drop for AlignedBuf {
    fn drop(&mut self) {
        // SAFETY: allocated in `new` with this same layout.
        unsafe { dealloc(self.ptr, self.layout) };
    }
}

/// Tiny xorshift64 for random offsets and fill data.
struct XorShift(u64);

impl XorShift {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
}

/// The open test file, deleted when dropped (also by the kernel on process exit, through
/// `FILE_FLAG_DELETE_ON_CLOSE`).
struct TestFile {
    handle: Option<Owned<HANDLE>>,
    path: HSTRING,
}

impl Drop for TestFile {
    fn drop(&mut self) {
        drop(self.handle.take());
        // SAFETY: NUL-terminated path; failing (already deleted on close) is fine.
        let _ = unsafe { DeleteFileW(&self.path) };
    }
}

/// How a phase picks its offsets.
#[derive(Clone, Copy)]
enum Pattern {
    /// One pass from the start.
    Sequential,
    /// Block-aligned random offsets, at most one file's worth of blocks.
    Random,
}

/// One measured phase.
struct Job {
    phase: Phase,
    block: usize,
    depth: usize,
    write: bool,
    pattern: Pattern,
    /// Stop issuing new I/O after this long.
    cap: Duration,
}

/// The file, its completion port and one buffer + OVERLAPPED per queue slot.
struct Engine {
    file: TestFile,
    port: Owned<HANDLE>,
    bytes: u64,
    bufs: Vec<AlignedBuf>,
    slots: Box<[OVERLAPPED]>,
    rng: XorShift,
    /// I/O may still be in flight after a timeout: never free the buffers.
    leaked: bool,
}

impl Drop for Engine {
    fn drop(&mut self) {
        if self.leaked {
            std::mem::forget(std::mem::take(&mut self.bufs));
            std::mem::forget(std::mem::take(&mut self.slots));
        }
    }
}

/// Bytes per sector of the volume holding `mount`, if readable.
fn sector_size(mount: &str) -> Option<u32> {
    let mut sector = 0u32;
    // SAFETY: NUL-terminated root path and a valid out pointer to a local.
    unsafe {
        GetDiskFreeSpaceW(
            &HSTRING::from(format!(r"{}\", mount.trim_end_matches('\\'))),
            None,
            Some(&mut sector),
            None,
            None,
        )
    }
    .ok()?;
    Some(sector)
}

fn free_bytes(dir: &Path) -> Option<u64> {
    let mut free = 0u64;
    // SAFETY: NUL-terminated directory path and a valid out pointer to a local.
    unsafe { GetDiskFreeSpaceExW(&HSTRING::from(dir.as_os_str()), Some(&mut free), None, None) }
        .ok()?;
    Some(free)
}

/// Write the test file at `path`, run every phase and delete the file.
pub(crate) fn measure(
    path: &Path,
    mount: &str,
    bytes: u64,
    need_free: u64,
    cap: Duration,
    progress: &Progress,
) -> Result<Speeds, String> {
    let dir = path.parent().unwrap_or(path);
    if free_bytes(dir).is_some_and(|f| f < need_free) {
        return Err(format!(
            "No volume on this drive with enough free space (needs {:.1} GB free)",
            need_free as f64 / 1e9
        ));
    }
    let align = sector_size(mount).unwrap_or(512).max(4096) as usize;
    if SEQ_BLOCK % align != 0 || RND_BLOCK % align != 0 {
        return Err(format!(
            "Unsupported sector size ({align} bytes) on {mount}"
        ));
    }

    let mut e = Engine::open(path, mount, bytes, align)?;
    progress.start(Phase::Preparing);
    let seq = |phase, write, cap| Job {
        phase,
        block: SEQ_BLOCK,
        depth: 8,
        write,
        pattern: Pattern::Sequential,
        cap,
    };
    let rnd = |phase, depth, write| Job {
        phase,
        block: RND_BLOCK,
        depth,
        write,
        pattern: Pattern::Random,
        cap,
    };
    // Fill the whole file first so reads never hit unwritten (zero-returned) space.
    e.run(&seq(Phase::Preparing, true, Duration::MAX), progress)?;
    Ok(Speeds {
        seq_write: e.run(&seq(Phase::SeqWrite, true, cap), progress)?,
        seq_read: e.run(&seq(Phase::SeqRead, false, cap), progress)?,
        rnd_read_q32: e.run(&rnd(Phase::RandomReadQ32, 32, false), progress)?,
        rnd_read: e.run(&rnd(Phase::RandomRead, 1, false), progress)?,
        rnd_write: e.run(&rnd(Phase::RandomWrite, 1, true), progress)?,
    })
}

impl Engine {
    fn open(path: &Path, mount: &str, bytes: u64, align: usize) -> Result<Engine, String> {
        let wpath = HSTRING::from(path.as_os_str());
        // SAFETY: NUL-terminated path that outlives the call; no security attributes or
        // template.
        let h = unsafe {
            CreateFileW(
                &wpath,
                GENERIC_READ.0 | GENERIC_WRITE.0 | DELETE.0,
                FILE_SHARE_MODE(0),
                None,
                CREATE_ALWAYS,
                FILE_FLAG_NO_BUFFERING
                    | FILE_FLAG_WRITE_THROUGH
                    | FILE_FLAG_OVERLAPPED
                    | FILE_FLAG_DELETE_ON_CLOSE
                    | FILE_ATTRIBUTE_TEMPORARY,
                None,
            )
        }
        .map_err(|err| {
            if err.code() == ERROR_ACCESS_DENIED.to_hresult() {
                format!("Can't write a test file to {mount} (try running as administrator)")
            } else {
                format!("Can't create the test file on {mount}: {}", err.message())
            }
        })?;
        let file = TestFile {
            // SAFETY: CreateFileW succeeded, so `h` is a fresh handle owned by nobody else.
            handle: Some(unsafe { Owned::new(h) }),
            path: wpath,
        };
        let fail = |what: &str, err: windows::core::Error| format!("{what}: {}", err.message());
        // SAFETY: valid file handle; the file pointer is only used by SetEndOfFile.
        unsafe {
            SetFilePointerEx(h, bytes as i64, None, FILE_BEGIN)
                .and_then(|()| SetEndOfFile(h))
                .map_err(|err| fail("Can't size the test file", err))?;
        }
        // SAFETY: valid overlapped file handle; a new port, no completion key.
        let port = unsafe { CreateIoCompletionPort(h, None, 0, 1) }
            .map_err(|err| fail("Can't set up the test I/O", err))?;
        let mut rng = XorShift(0x9E37_79B9_7F4A_7C15 ^ u64::from(std::process::id()));
        let mut bufs = Vec::new();
        for _ in 0..32 {
            let mut b = AlignedBuf::new(SEQ_BLOCK, align)?;
            b.fill_random(&mut rng);
            bufs.push(b);
        }
        Ok(Engine {
            file,
            // SAFETY: CreateIoCompletionPort made a new port handle owned by nobody else.
            port: unsafe { Owned::new(port) },
            bytes,
            bufs,
            slots: vec![OVERLAPPED::default(); 32].into_boxed_slice(),
            rng,
            leaked: false,
        })
    }

    fn handle(&self) -> HANDLE {
        self.file.handle.as_deref().copied().unwrap_or_default()
    }

    /// Start one read or write on slot `i`.
    fn issue(&mut self, i: usize, offset: u64, job: &Job) -> windows::core::Result<()> {
        let ov = &mut self.slots[i];
        *ov = OVERLAPPED {
            Anonymous: OVERLAPPED_0 {
                Anonymous: OVERLAPPED_0_0 {
                    Offset: offset as u32,
                    OffsetHigh: (offset >> 32) as u32,
                },
            },
            ..Default::default()
        };
        let ov: *mut OVERLAPPED = ov;
        let h = self.handle();
        let buf = self.bufs[i].ptr;
        // SAFETY: the buffer and OVERLAPPED are owned by the engine and stay alive and
        // untouched until the completion for this slot is dequeued (or are leaked); `block` is
        // within the buffer and aligned, as unbuffered I/O requires.
        let r = unsafe {
            if job.write {
                WriteFile(
                    h,
                    Some(std::slice::from_raw_parts(buf, job.block)),
                    None,
                    Some(ov),
                )
            } else {
                ReadFile(
                    h,
                    Some(std::slice::from_raw_parts_mut(buf, job.block)),
                    None,
                    Some(ov),
                )
            }
        };
        match r {
            Err(err) if err.code() != ERROR_IO_PENDING.to_hresult() => Err(err),
            _ => Ok(()),
        }
    }

    /// Run one phase; returns MB/s (10^6 bytes per second).
    fn run(&mut self, job: &Job, progress: &Progress) -> Result<f64, String> {
        progress.start(job.phase);
        let blocks = self.bytes / job.block as u64;
        let mut issued = 0u64;
        let mut next = |rng: &mut XorShift| -> Option<u64> {
            if issued >= blocks {
                return None;
            }
            let block = match job.pattern {
                Pattern::Sequential => issued,
                Pattern::Random => rng.next() % blocks,
            };
            issued += 1;
            Some(block * job.block as u64)
        };

        let start = Instant::now();
        let (mut done_bytes, mut done_ops, mut outstanding) = (0u64, 0u64, 0usize);
        let mut error: Option<String> = None;
        for i in 0..job.depth {
            let Some(off) = next(&mut self.rng) else {
                break;
            };
            match self.issue(i, off, job) {
                Ok(()) => outstanding += 1,
                Err(err) => {
                    error = Some(err.message());
                    break;
                }
            }
        }
        let mut end = start;
        let mut cancelled = false;
        while outstanding > 0 {
            let (mut n, mut key, mut ov) = (0u32, 0usize, ptr::null_mut::<OVERLAPPED>());
            // SAFETY: valid port handle and out pointers to locals.
            let r = unsafe {
                GetQueuedCompletionStatus(*self.port, &mut n, &mut key, &mut ov, IO_TIMEOUT_MS)
            };
            if ov.is_null() {
                // Timed out (or the port failed) with I/O still in flight.
                if cancelled {
                    self.leaked = true;
                    return Err("The drive stopped responding during the test".into());
                }
                // SAFETY: valid file handle; cancels this process's I/O on it.
                let _ = unsafe { CancelIoEx(self.handle(), None) };
                cancelled = true;
                error.get_or_insert_with(|| "The drive stopped responding during the test".into());
                continue;
            }
            outstanding -= 1;
            if let Err(err) = r {
                error.get_or_insert_with(|| err.message());
                continue;
            }
            done_bytes += u64::from(n);
            done_ops += 1;
            end = Instant::now();
            let elapsed = end - start;
            progress.set(
                (done_ops as f64 / blocks as f64)
                    .max(elapsed.as_secs_f64() / job.cap.as_secs_f64()),
            );
            if error.is_some() || elapsed >= job.cap {
                continue;
            }
            // SAFETY: `ov` is one of our slots (the only OVERLAPPEDs queued on this port).
            let slot = unsafe { ov.offset_from(self.slots.as_ptr()) } as usize;
            if let Some(off) = next(&mut self.rng) {
                match self.issue(slot, off, job) {
                    Ok(()) => outstanding += 1,
                    Err(err) => error = Some(err.message()),
                }
            }
        }
        if let Some(err) = error {
            return Err(format!("Disk I/O failed during the test: {err}"));
        }
        let secs = (end - start).as_secs_f64();
        Ok(if secs > 0.0 {
            done_bytes as f64 / secs / 1e6
        } else {
            0.0
        })
    }
}
