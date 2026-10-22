//! Disk speed test I/O: direct (`O_DIRECT`), write-through (`O_DSYNC`) positional reads and
//! writes on one temporary file, with the queue depth made of worker threads that each keep one
//! I/O in flight. The file is unlinked as soon as it's open, so the kernel frees it when the
//! test ends, even if Omni-HW is killed.

use crate::disktest::{Phase, Progress, Speeds};
use std::alloc::{Layout, alloc, dealloc};
use std::os::unix::fs::{FileExt, OpenOptionsExt};
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

const SEQ_BLOCK: usize = 1 << 20;
const RND_BLOCK: usize = 4096;
/// Alignment for direct I/O; covers 512-byte and 4K logical sectors.
const ALIGN: usize = 4096;

/// A heap buffer aligned for direct I/O, freed on drop.
struct AlignedBuf {
    ptr: *mut u8,
    layout: Layout,
}

// SAFETY: the buffer is plain memory owned by one worker thread at a time.
unsafe impl Send for AlignedBuf {}

impl AlignedBuf {
    fn new(len: usize, rng: &mut XorShift) -> Result<AlignedBuf, String> {
        let layout = Layout::from_size_align(len, ALIGN).map_err(|e| e.to_string())?;
        // SAFETY: `layout` has a non-zero size (callers never ask for 0 bytes).
        let ptr = unsafe { alloc(layout) };
        if ptr.is_null() {
            return Err("Out of memory for the test buffers".into());
        }
        let mut b = AlignedBuf { ptr, layout };
        // Non-zero pseudo-random bytes, so compressing controllers can't shortcut the writes.
        for chunk in b.as_mut().as_chunks_mut::<8>().0 {
            chunk.copy_from_slice(&(rng.next() | 0x0101_0101_0101_0101).to_ne_bytes());
        }
        Ok(b)
    }

    fn as_mut(&mut self) -> &mut [u8] {
        // SAFETY: `ptr` points to `layout.size()` bytes owned by this buffer.
        unsafe { std::slice::from_raw_parts_mut(self.ptr, self.layout.size()) }
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

fn free_bytes(dir: &Path) -> Option<u64> {
    let s = rustix::fs::statvfs(dir).ok()?;
    Some(s.f_bavail * s.f_frsize)
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
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .custom_flags(libc::O_DIRECT | libc::O_DSYNC)
        .open(path)
        .map_err(|e| match e.raw_os_error() {
            Some(libc::EINVAL) => {
                format!("The file system on {mount} doesn't support direct I/O")
            }
            Some(libc::EACCES) | Some(libc::EPERM) | Some(libc::EROFS) => {
                format!("Can't write a test file to {}", dir.display())
            }
            _ => format!("Can't create the test file on {mount}: {e}"),
        })?;
    // Unlinked right away: the space is freed when `file` closes, on every exit path.
    let _ = std::fs::remove_file(path);
    file.set_len(bytes)
        .map_err(|e| format!("Can't size the test file: {e}"))?;

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
    run(
        &file,
        bytes,
        &seq(Phase::Preparing, true, Duration::MAX),
        progress,
    )?;
    Ok(Speeds {
        seq_write: run(&file, bytes, &seq(Phase::SeqWrite, true, cap), progress)?,
        seq_read: run(&file, bytes, &seq(Phase::SeqRead, false, cap), progress)?,
        rnd_read_q32: run(
            &file,
            bytes,
            &rnd(Phase::RandomReadQ32, 32, false),
            progress,
        )?,
        rnd_read: run(&file, bytes, &rnd(Phase::RandomRead, 1, false), progress)?,
        rnd_write: run(&file, bytes, &rnd(Phase::RandomWrite, 1, true), progress)?,
    })
}

/// Run one phase; returns MB/s (10^6 bytes per second).
fn run(file: &std::fs::File, bytes: u64, job: &Job, progress: &Progress) -> Result<f64, String> {
    progress.start(job.phase);
    let blocks = bytes / job.block as u64;
    let issued = AtomicU64::new(0);
    let done_ops = AtomicU64::new(0);
    let done_bytes = AtomicU64::new(0);
    // Nanoseconds from `start` to the last completed I/O.
    let last_ns = AtomicU64::new(0);
    let stop = AtomicBool::new(false);
    let error: Mutex<Option<String>> = Mutex::new(None);
    let start = Instant::now();
    let seed = 0x9E37_79B9_7F4A_7C15 ^ u64::from(std::process::id());

    std::thread::scope(|s| {
        for t in 0..job.depth {
            let (issued, done_ops, done_bytes, last_ns, stop, error) =
                (&issued, &done_ops, &done_bytes, &last_ns, &stop, &error);
            s.spawn(move || {
                let mut rng =
                    XorShift(seed.wrapping_add((t as u64 + 1).wrapping_mul(0x2545_F491_4F6C_DD1D)));
                let mut buf = match AlignedBuf::new(job.block, &mut rng) {
                    Ok(b) => b,
                    Err(e) => {
                        error.lock().unwrap().get_or_insert(e);
                        stop.store(true, Ordering::Relaxed);
                        return;
                    }
                };
                while !stop.load(Ordering::Relaxed) && start.elapsed() < job.cap {
                    let n = issued.fetch_add(1, Ordering::Relaxed);
                    if n >= blocks {
                        break;
                    }
                    let block = match job.pattern {
                        Pattern::Sequential => n,
                        Pattern::Random => rng.next() % blocks,
                    };
                    let off = block * job.block as u64;
                    let r = if job.write {
                        file.write_all_at(buf.as_mut(), off)
                    } else {
                        file.read_exact_at(buf.as_mut(), off)
                    };
                    if let Err(e) = r {
                        error.lock().unwrap().get_or_insert(e.to_string());
                        stop.store(true, Ordering::Relaxed);
                        break;
                    }
                    let elapsed = start.elapsed();
                    last_ns.fetch_max(elapsed.as_nanos() as u64, Ordering::Relaxed);
                    done_bytes.fetch_add(job.block as u64, Ordering::Relaxed);
                    let ops = done_ops.fetch_add(1, Ordering::Relaxed) + 1;
                    progress.set(
                        (ops as f64 / blocks as f64)
                            .max(elapsed.as_secs_f64() / job.cap.as_secs_f64()),
                    );
                }
            });
        }
    });
    if let Some(err) = error.into_inner().unwrap_or_default() {
        return Err(format!("Disk I/O failed during the test: {err}"));
    }
    let secs = last_ns.load(Ordering::Relaxed) as f64 / 1e9;
    Ok(if secs > 0.0 {
        done_bytes.load(Ordering::Relaxed) as f64 / secs / 1e6
    } else {
        0.0
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_file_round_trip() {
        // A tiny run against the system temp folder; skipped where direct I/O isn't supported
        // (tmpfs, some CI file systems).
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("disktest.tmp");
        let progress = Progress::default();
        match measure(&path, "test", 8 << 20, 0, Duration::from_secs(2), &progress) {
            Ok(s) => {
                assert!(s.seq_write > 0.0 && s.seq_read > 0.0 && s.rnd_read_q32 > 0.0);
                assert!(!path.exists(), "the test file is removed");
            }
            Err(e) => assert!(e.contains("direct I/O"), "{e}"),
        }
    }
}
