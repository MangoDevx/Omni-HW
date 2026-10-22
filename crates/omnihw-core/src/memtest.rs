//! Quick memory self-test: multi-threaded read/write/copy bandwidth, single-core read bandwidth
//! and single-threaded pointer-chase latency, compared with what the installed DIMMs should
//! deliver.
//!
//! Reads work like AIDA64's: a hand-written assembly loop streams the buffer into vector
//! registers (AVX 256-bit loads, SSE2 128-bit without AVX) with no arithmetic in the way. The
//! headline read runs it on one pinned thread per logical processor (all but one), which
//! saturates the memory channels; the single-core read runs it on one performance core at high
//! priority, which shows what one thread gets (capped by the core, not the RAM).
//!
//! Runs only when the user asks; takes a few seconds and changes nothing.

use crate::smbios::MemoryDevice;
use crate::topology::{LogicalId, Topology};
use std::hint::black_box;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

const MIB: u64 = 1024 * 1024;
const GIB: u64 = 1024 * MIB;
/// Timed passes per bandwidth kernel; the best one is kept.
const PASSES: usize = 5;
/// The single-core read pass is quick, so it gets more passes.
const SINGLE_READ_PASSES: usize = 10;
/// Region the latency test chases pointers over.
const LATENCY_BYTES: u64 = 256 * MIB;
/// u64s per 64-byte cache line.
const LINE: usize = 8;
/// Latency test budget: stop after this many loads or this much time, whichever is first.
const LATENCY_MAX_LOADS: u64 = 20_000_000;
const LATENCY_MAX_TIME: Duration = Duration::from_secs(1);
const LATENCY_BATCH: u64 = 1_000_000;

/// Bandwidth reads as Good at or above this % of theoretical, Low above `LOW_PCT`.
const GOOD_PCT: f64 = 55.0;
const LOW_PCT: f64 = 35.0;
/// Latency reads as Low up to this multiple of the typical upper bound.
const LATENCY_LOW_FACTOR: f64 = 1.3;

/// Measured bandwidth (GB/s, 10^9 bytes) and latency.
#[derive(Debug, Clone, Default)]
pub struct MemTestResult {
    /// Multi-threaded streaming read; what the verdict is based on.
    pub read_gbs: f64,
    /// Streaming read on one core.
    pub read_single_gbs: f64,
    pub write_gbs: f64,
    /// Bytes read plus bytes written per second (the AIDA64 convention).
    pub copy_gbs: f64,
    /// Average time per dependent load over a random walk.
    pub latency_ns: f64,
    /// Threads used by the multi-threaded passes.
    pub threads: usize,
    /// The logical processor the single-core read was pinned to (None if pinning failed).
    pub read_cpu: Option<LogicalId>,
    pub buffer_mb: u64,
    pub expected: Option<MemExpectation>,
}

/// What the installed memory should deliver, from SMBIOS.
#[derive(Debug, Clone, Default)]
pub struct MemExpectation {
    /// MT/s × 8 bytes × channels / 1000.
    pub theoretical_gbs: f64,
    pub channels: u32,
    /// The slowest populated DIMM's configured speed.
    pub mts: u32,
    pub memory_type: String,
    /// SMBIOS doesn't report channels, so the count is guessed from DIMMs and slots.
    pub channels_estimated: bool,
    /// Typical latency range for this memory type on this test (with 4 KB pages).
    pub latency_typical_ns: (f64, f64),
    /// The slowest populated DIMM's rated speed (0 if unknown), for the XMP/EXPO hint.
    pub rated_mts: u32,
}

/// How a measurement compares with what's expected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Good,
    Low,
    VeryLow,
}

impl Verdict {
    pub fn name(self) -> &'static str {
        match self {
            Verdict::Good => "Good",
            Verdict::Low => "Low",
            Verdict::VeryLow => "Very low",
        }
    }
}

impl MemTestResult {
    /// Read bandwidth as a % of theoretical.
    pub fn efficiency_pct(&self) -> Option<f64> {
        let e = self.expected.as_ref()?;
        (e.theoretical_gbs > 0.0).then(|| self.read_gbs / e.theoretical_gbs * 100.0)
    }

    pub fn bandwidth_verdict(&self) -> Option<Verdict> {
        let pct = self.efficiency_pct()?;
        Some(if pct >= GOOD_PCT {
            Verdict::Good
        } else if pct >= LOW_PCT {
            Verdict::Low
        } else {
            Verdict::VeryLow
        })
    }

    /// Latency verdict ("Good" means it's no worse than typical; lower is better).
    pub fn latency_verdict(&self) -> Option<Verdict> {
        let upper = self.expected.as_ref()?.latency_typical_ns.1;
        Some(if self.latency_ns <= upper {
            Verdict::Good
        } else if self.latency_ns <= upper * LATENCY_LOW_FACTOR {
            Verdict::Low
        } else {
            Verdict::VeryLow
        })
    }
}

/// Test phases, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Preparing,
    Read,
    ReadSingle,
    Write,
    Copy,
    Latency,
    Done,
}

impl Phase {
    /// Phases that do work (all but `Done`).
    pub const COUNT: u8 = 6;

    pub fn name(self) -> &'static str {
        match self {
            Phase::Preparing => "preparing",
            Phase::Read => "read bandwidth",
            Phase::ReadSingle => "single-core read",
            Phase::Write => "write bandwidth",
            Phase::Copy => "copy bandwidth",
            Phase::Latency => "latency",
            Phase::Done => "done",
        }
    }
}

/// Shared progress a running test updates and the UI reads.
#[derive(Debug, Default)]
pub struct Progress {
    phase: AtomicU8,
}

impl Progress {
    fn set(&self, p: Phase) {
        self.phase.store(p as u8, Ordering::Relaxed);
    }

    /// (phase, 1-based step, total steps) for the UI.
    pub fn read(&self) -> (Phase, u8, u8) {
        let i = self.phase.load(Ordering::Relaxed);
        let phase = match i {
            0 => Phase::Preparing,
            1 => Phase::Read,
            2 => Phase::ReadSingle,
            3 => Phase::Write,
            4 => Phase::Copy,
            5 => Phase::Latency,
            _ => Phase::Done,
        };
        (phase, (i + 1).min(Phase::COUNT), Phase::COUNT)
    }
}

/// What `dimms` (the populated modules) should deliver; `slots` is the board's slot count.
/// `None` without DIMMs or a known speed.
pub fn expectation(dimms: &[&MemoryDevice], slots: u32) -> Option<MemExpectation> {
    let speed = |d: &&MemoryDevice| {
        if d.configured_speed_mts > 0 {
            d.configured_speed_mts
        } else {
            d.speed_mts
        }
    };
    let populated: Vec<_> = dimms.iter().filter(|d| d.is_populated()).collect();
    let mts = populated.iter().map(|d| speed(d)).min()?;
    if mts == 0 {
        return None;
    }
    let rated_mts = populated
        .iter()
        .map(|d| d.speed_mts)
        .filter(|&s| s > 0)
        .min()
        .unwrap_or(0);
    let n = populated.len() as u32;
    let channels = if slots <= 4 {
        n.min(2)
    } else {
        n.min(slots / 2)
    }
    .max(1);
    let memory_type = populated[0].memory_type.to_string();
    Some(MemExpectation {
        theoretical_gbs: mts as f64 * 8.0 * channels as f64 / 1000.0,
        channels,
        mts,
        latency_typical_ns: typical_latency(&memory_type),
        memory_type,
        channels_estimated: true,
        rated_mts,
    })
}

/// Typical latency range on this test. Higher than AIDA64's figures because the walk runs on
/// 4 KB pages, so most loads also miss the TLB.
fn typical_latency(memory_type: &str) -> (f64, f64) {
    if memory_type.contains("DDR5") {
        (65.0, 120.0)
    } else if memory_type.contains("DDR4") {
        (55.0, 110.0)
    } else {
        (50.0, 130.0)
    }
}

/// Buffer size for `available_bytes` of free memory.
fn buffer_bytes(available_bytes: u64) -> Result<u64, String> {
    if available_bytes < GIB {
        Err("Not enough free memory to run the test (needs at least 1 GB free).".into())
    } else if available_bytes > 4 * GIB {
        Ok(GIB)
    } else {
        Ok(256 * MIB)
    }
}

/// Run the test, sized for `available_bytes` of free memory. Blocks for a few seconds and uses
/// all but one logical processor during the multi-threaded phases.
pub fn run(
    available_bytes: u64,
    expected: Option<MemExpectation>,
    progress: &Progress,
) -> Result<MemTestResult, String> {
    progress.set(Phase::Preparing);
    let bytes = buffer_bytes(available_bytes)?;
    let threads = std::thread::available_parallelism()
        .map(|n| n.get().saturating_sub(1))
        .unwrap_or(1)
        .max(1);
    let topology = crate::win::topology_buffer().and_then(|b| Topology::parse(&b).ok());
    let cpus = Cpus {
        single: topology.as_ref().and_then(read_core),
        all: topology.as_ref().map_or_else(Vec::new, worker_cpus),
    };
    let r = measure(bytes, threads, &cpus, PASSES, LATENCY_MAX_LOADS, progress);
    progress.set(Phase::Done);
    let mut r = r?;
    r.expected = expected;
    Ok(r)
}

/// Where the read passes pin their threads.
#[derive(Default)]
struct Cpus {
    /// The single-core read's processor.
    single: Option<LogicalId>,
    /// The multi-threaded read's processors, worker `i` on `all[i]` (unpinned past the end).
    all: Vec<LogicalId>,
}

/// The whole test on a `bytes` buffer with `threads` workers, reads pinned per `cpus`.
fn measure(
    bytes: u64,
    threads: usize,
    cpus: &Cpus,
    passes: usize,
    max_loads: u64,
    progress: &Progress,
) -> Result<MemTestResult, String> {
    let words = usize::try_from(bytes / 8).map_err(|_| "Test buffer too large")?;
    let mut buf: Vec<u64> = Vec::new();
    buf.try_reserve_exact(words)
        .map_err(|_| "Couldn't allocate the test buffer (not enough free memory).")?;
    // Writing every word also faults in every page before anything is timed.
    buf.resize(words, 1);
    let gbs = |b: usize, secs: f64| b as f64 / secs / 1e9;
    let total = words * 8;

    progress.set(Phase::Read);
    let parts: Vec<&[u64]> = split(&mut buf, threads).into_iter().map(|c| &*c).collect();
    let read_bytes: usize = parts.iter().map(|c| stream_len(c)).sum();
    let t = best_time(parts, passes, &cpus.all, |c| {
        stream_read(c);
    })?;
    let read_gbs = gbs(read_bytes, t);

    progress.set(Phase::ReadSingle);
    let (single_bytes, t, read_cpu) =
        read_single_core(&buf, cpus.single, passes.max(SINGLE_READ_PASSES))?;
    let read_single_gbs = gbs(single_bytes, t);

    progress.set(Phase::Write);
    let parts: Vec<&mut [u64]> = split(&mut buf, threads);
    let t = best_time(parts, passes, &[], |c| {
        c.fill(black_box(0x5A5A_5A5A_5A5A_5A5A));
        black_box(c);
    })?;
    let write_gbs = gbs(total, t);

    progress.set(Phase::Copy);
    let half = words / 2;
    let (src, dst) = buf.split_at_mut(half);
    let dst = &mut dst[..half];
    let parts: Vec<(&mut [u64], &mut [u64])> = split(src, threads)
        .into_iter()
        .zip(split(dst, threads))
        .collect();
    let t = best_time(parts, passes, &[], |(s, d)| {
        d.copy_from_slice(black_box(s));
        black_box(d);
    })?;
    // Each copied byte is read once and written once.
    let copy_gbs = gbs(half * 8 * 2, t);

    progress.set(Phase::Latency);
    let lines = (bytes.min(LATENCY_BYTES) / 64) as usize;
    let latency_ns = latency(&mut buf[..lines * LINE], max_loads);

    Ok(MemTestResult {
        read_gbs,
        read_single_gbs,
        write_gbs,
        copy_gbs,
        latency_ns,
        threads,
        read_cpu,
        buffer_mb: bytes / MIB,
        expected: None,
    })
}

/// The core to read on: the last performance core's first logical processor. Performance so a
/// hybrid CPU doesn't land on an E-core; last because Windows steers most interrupts and DPCs to
/// processor 0.
fn read_core(t: &Topology) -> Option<LogicalId> {
    t.cores
        .iter()
        .filter(|c| t.is_p_core(c))
        .filter_map(|c| c.logical.first().copied())
        .max()
}

/// The multi-threaded read's processors: every logical processor but group 0's processor 0
/// (left free, as it takes most interrupts), first logical of each core before SMT siblings, so
/// a short worker list still covers every core.
fn worker_cpus(t: &Topology) -> Vec<LogicalId> {
    let depth = t.cores.iter().map(|c| c.logical.len()).max().unwrap_or(0);
    (0..depth)
        .flat_map(|i| {
            t.cores
                .iter()
                .filter_map(move |c| c.logical.get(i).copied())
        })
        .filter(|&id| {
            id != LogicalId {
                group: 0,
                number: 0,
            }
        })
        .collect()
}

/// Stream all of `buf` through one thread pinned to `cpu`, `passes` times: (bytes per pass,
/// fastest pass in seconds, the processor actually pinned to). Runs on its own thread so the
/// caller's affinity and priority are left alone.
fn read_single_core(
    buf: &[u64],
    cpu: Option<LogicalId>,
    passes: usize,
) -> Result<(usize, f64, Option<LogicalId>), String> {
    std::thread::scope(|s| {
        std::thread::Builder::new()
            .name("omnihw-mem-read".into())
            .spawn_scoped(s, || {
                let pinned = cpu.filter(|&id| pin(id, true));
                // Warm-up pass: settles the core's clock and fills the TLB.
                let bytes = stream_read(buf);
                let mut best = f64::MAX;
                for _ in 0..passes {
                    let t = Instant::now();
                    stream_read(buf);
                    best = best.min(t.elapsed().as_secs_f64());
                }
                (bytes, best.max(1e-9), pinned)
            })
            .map_err(|_| "Couldn't start the test thread.".to_string())?
            .join()
            .map_err(|_| "The read test thread failed.".to_string())
    })
}

/// Pin the calling thread to `id`, at high priority if `boost`; false if pinning failed.
#[cfg(windows)]
fn pin(id: LogicalId, boost: bool) -> bool {
    crate::win::memtest::pin_current_thread(id, boost)
}

#[cfg(not(windows))]
fn pin(_: LogicalId, _: bool) -> bool {
    false
}

/// A 64-byte cache line, aligned so the kernels can use aligned vector loads.
#[repr(C, align(64))]
struct Line([u8; 64]);

/// Bytes of `buf` that `stream_read` reads: the whole kernel blocks in its 64-byte aligned middle.
fn stream_len(buf: &[u64]) -> usize {
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: `Line` is plain bytes, so any u64 bit pattern is a valid `Line`.
        let (_, lines, _) = unsafe { buf.align_to::<Line>() };
        let block = if std::arch::is_x86_feature_detected!("avx") {
            256
        } else {
            128
        };
        lines.len() * 64 / block * block
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        buf.len() * 8
    }
}

/// Read every aligned cache line of `buf` once into vector registers; the bytes read.
fn stream_read(buf: &[u64]) -> usize {
    // SAFETY: `Line` is plain bytes, so any u64 bit pattern is a valid `Line`.
    let (_, lines, _) = unsafe { buf.align_to::<Line>() };
    #[cfg(target_arch = "x86_64")]
    {
        let ptr = lines.as_ptr().cast::<u8>();
        if std::arch::is_x86_feature_detected!("avx") {
            let blocks = lines.len() / 4;
            if blocks > 0 {
                // SAFETY: AVX is present; `blocks` 256-byte blocks from `ptr` lie inside `lines`,
                // and `ptr` is 64-byte aligned.
                unsafe { stream_avx(ptr, blocks) };
            }
            blocks * 256
        } else {
            let blocks = lines.len() / 2;
            if blocks > 0 {
                // SAFETY: as above, with 128-byte blocks; SSE2 is baseline on x86-64.
                unsafe { stream_sse2(ptr, blocks) };
            }
            blocks * 128
        }
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        black_box(read_sum(black_box(buf)));
        buf.len() * 8
    }
}

/// Load `blocks` 256-byte blocks from 32-byte aligned `ptr` into ymm0-7, eight loads per block.
/// Nothing is computed from the loaded data, so the loop is bound only by memory.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx")]
unsafe fn stream_avx(ptr: *const u8, blocks: usize) {
    // SAFETY: the caller guarantees `blocks` > 0 and that the range is readable and aligned.
    unsafe {
        std::arch::asm!(
            "2:",
            "vmovdqa ymm0, ymmword ptr [{p}]",
            "vmovdqa ymm1, ymmword ptr [{p} + 32]",
            "vmovdqa ymm2, ymmword ptr [{p} + 64]",
            "vmovdqa ymm3, ymmword ptr [{p} + 96]",
            "vmovdqa ymm4, ymmword ptr [{p} + 128]",
            "vmovdqa ymm5, ymmword ptr [{p} + 160]",
            "vmovdqa ymm6, ymmword ptr [{p} + 192]",
            "vmovdqa ymm7, ymmword ptr [{p} + 224]",
            "add {p}, 256",
            "dec {n}",
            "jnz 2b",
            "vzeroupper",
            p = inout(reg) ptr => _,
            n = inout(reg) blocks => _,
            out("ymm0") _, out("ymm1") _, out("ymm2") _, out("ymm3") _,
            out("ymm4") _, out("ymm5") _, out("ymm6") _, out("ymm7") _,
            options(nostack, readonly),
        );
    }
}

/// `stream_avx` for CPUs without AVX: 128-byte blocks into xmm0-7.
#[cfg(target_arch = "x86_64")]
unsafe fn stream_sse2(ptr: *const u8, blocks: usize) {
    // SAFETY: the caller guarantees `blocks` > 0 and that the range is readable and aligned.
    unsafe {
        std::arch::asm!(
            "2:",
            "movdqa xmm0, xmmword ptr [{p}]",
            "movdqa xmm1, xmmword ptr [{p} + 16]",
            "movdqa xmm2, xmmword ptr [{p} + 32]",
            "movdqa xmm3, xmmword ptr [{p} + 48]",
            "movdqa xmm4, xmmword ptr [{p} + 64]",
            "movdqa xmm5, xmmword ptr [{p} + 80]",
            "movdqa xmm6, xmmword ptr [{p} + 96]",
            "movdqa xmm7, xmmword ptr [{p} + 112]",
            "add {p}, 128",
            "dec {n}",
            "jnz 2b",
            p = inout(reg) ptr => _,
            n = inout(reg) blocks => _,
            out("xmm0") _, out("xmm1") _, out("xmm2") _, out("xmm3") _,
            out("xmm4") _, out("xmm5") _, out("xmm6") _, out("xmm7") _,
            options(nostack, readonly),
        );
    }
}

/// `buf` cut into `n` nearly equal mutable chunks (fewer if it's shorter than `n`).
fn split(buf: &mut [u64], n: usize) -> Vec<&mut [u64]> {
    let size = buf.len().div_ceil(n.max(1)).max(1);
    buf.chunks_mut(size).collect()
}

/// Sum with independent accumulators so the loop is limited by memory, not add latency. The read
/// pass's fallback off x86-64.
#[cfg_attr(target_arch = "x86_64", allow(dead_code))]
fn read_sum(s: &[u64]) -> u64 {
    let mut acc = [0u64; 8];
    let (chunks, rest) = s.as_chunks::<8>();
    for c in chunks {
        for (a, &x) in acc.iter_mut().zip(c) {
            *a = a.wrapping_add(x);
        }
    }
    let rest = rest.iter().fold(0u64, |a, &x| a.wrapping_add(x));
    acc.iter().fold(rest, |a, &x| a.wrapping_add(x))
}

/// Run `work` on every part, one thread each, `passes` times; the fastest pass's wall time in
/// seconds. Thread `i` is pinned to `cpus[i]` if there is one. Threads start each pass together
/// on a spin gate so spawn cost isn't timed.
fn best_time<T: Send>(
    parts: Vec<T>,
    passes: usize,
    cpus: &[LogicalId],
    work: impl Fn(&mut T) + Sync,
) -> Result<f64, String> {
    let ready = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let go = AtomicUsize::new(0);
    let abort = AtomicBool::new(false);
    std::thread::scope(|s| {
        let n = parts.len();
        for (i, mut p) in parts.into_iter().enumerate() {
            let (ready, done, go, abort, work) = (&ready, &done, &go, &abort, &work);
            let cpu = cpus.get(i).copied();
            let spawned = std::thread::Builder::new()
                .name("omnihw-mem-worker".into())
                .spawn_scoped(s, move || {
                    if let Some(id) = cpu {
                        pin(id, false);
                    }
                    for pass in 1..=passes {
                        ready.fetch_add(1, Ordering::AcqRel);
                        while go.load(Ordering::Acquire) < pass {
                            if abort.load(Ordering::Relaxed) {
                                return;
                            }
                            std::hint::spin_loop();
                        }
                        work(&mut p);
                        done.fetch_add(1, Ordering::AcqRel);
                    }
                });
            if spawned.is_err() {
                abort.store(true, Ordering::Relaxed);
                return Err("Couldn't start the test threads.".to_string());
            }
        }
        let mut best = f64::MAX;
        for pass in 1..=passes {
            while ready.load(Ordering::Acquire) < n * pass {
                std::hint::spin_loop();
            }
            let t = Instant::now();
            go.store(pass, Ordering::Release);
            while done.load(Ordering::Acquire) < n * pass {
                std::hint::spin_loop();
            }
            best = best.min(t.elapsed().as_secs_f64());
        }
        Ok(best.max(1e-9))
    })
}

/// splitmix64: a tiny, good-enough PRNG for shuffling.
struct SplitMix(u64);

impl SplitMix {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `0..n` (n > 0).
    fn below(&mut self, n: usize) -> usize {
        ((self.next() as u128 * n as u128) >> 64) as usize
    }
}

/// A random cyclic permutation of `0..n` (Sattolo's algorithm): following `i -> p[i]` visits
/// every index once before returning to the start.
fn sattolo(n: usize, seed: u64) -> Vec<u32> {
    let mut p: Vec<u32> = (0..n as u32).collect();
    let mut rng = SplitMix(seed);
    for i in (1..n).rev() {
        p.swap(i, rng.below(i));
    }
    p
}

/// Average ns per dependent load, walking a random cycle through every 64-byte line of `buf`.
/// Uses 4 KB pages, so most loads also miss the TLB and the figure reads somewhat higher than
/// AIDA64's (which uses large pages).
fn latency(buf: &mut [u64], max_loads: u64) -> f64 {
    let lines = buf.len() / LINE;
    if lines < 2 {
        return 0.0;
    }
    for (i, next) in sattolo(lines, 0x0123_4567_89AB_CDEF)
        .into_iter()
        .enumerate()
    {
        buf[i * LINE] = next as u64 * LINE as u64;
    }
    let buf = &*buf;
    let chase = |mut p: usize, loads: u64| {
        for _ in 0..loads {
            p = buf[p] as usize;
        }
        black_box(p)
    };
    // Warm up (and settle the clock), then time batches until the budget runs out.
    let batch = LATENCY_BATCH.min(max_loads);
    let mut p = chase(0, batch.min(lines as u64));
    let start = Instant::now();
    let mut loads = 0;
    while loads < max_loads && start.elapsed() < LATENCY_MAX_TIME {
        p = chase(p, batch);
        loads += batch;
    }
    black_box(p);
    start.elapsed().as_secs_f64() * 1e9 / loads.max(1) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dimm(speed: u32, configured: u32) -> MemoryDevice {
        MemoryDevice {
            size_mb: 16384,
            memory_type: "DDR4",
            speed_mts: speed,
            configured_speed_mts: configured,
            ..Default::default()
        }
    }

    fn expect(dimms: &[MemoryDevice], slots: u32) -> Option<MemExpectation> {
        let refs: Vec<_> = dimms.iter().collect();
        expectation(&refs, slots)
    }

    #[test]
    fn channels_from_dimms_and_slots() {
        let d = dimm(3200, 3200);
        let ch = |n: usize, slots| expect(&vec![d.clone(); n], slots).unwrap().channels;
        assert_eq!(ch(1, 4), 1);
        assert_eq!(ch(2, 4), 2);
        assert_eq!(ch(4, 4), 2);
        assert_eq!(ch(8, 8), 4);
        assert_eq!(ch(2, 8), 2);
        assert_eq!(ch(1, 0), 1);
        let e = expect(&[d.clone(), d], 4).unwrap();
        assert!((e.theoretical_gbs - 51.2).abs() < 1e-9);
        assert!(e.channels_estimated);
        assert_eq!(e.memory_type, "DDR4");
        assert_eq!(e.latency_typical_ns, (55.0, 110.0));
    }

    #[test]
    fn speed_is_slowest_configured() {
        let e = expect(&[dimm(3600, 3200), dimm(3200, 2933)], 4).unwrap();
        assert_eq!(e.mts, 2933);
        assert_eq!(e.rated_mts, 3200);
        // Configured speed unknown: fall back to rated.
        assert_eq!(expect(&[dimm(2666, 0)], 2).unwrap().mts, 2666);
        // Empty slots don't count.
        let empty = MemoryDevice::default();
        assert_eq!(expect(&[dimm(3200, 3200), empty], 4).unwrap().channels, 1);
    }

    #[test]
    fn no_expectation_without_dimms_or_speed() {
        assert!(expect(&[], 4).is_none());
        assert!(expect(&[MemoryDevice::default()], 4).is_none());
        assert!(expect(&[dimm(0, 0)], 4).is_none());
    }

    fn result(read: f64, latency: f64) -> MemTestResult {
        MemTestResult {
            read_gbs: read,
            latency_ns: latency,
            expected: expect(&[dimm(3200, 3200), dimm(3200, 3200)], 4),
            ..Default::default()
        }
    }

    #[test]
    fn verdicts() {
        // 51.2 GB/s theoretical, DDR4 latency up to 110 ns.
        assert_eq!(result(40.0, 80.0).bandwidth_verdict(), Some(Verdict::Good));
        assert_eq!(result(25.0, 80.0).bandwidth_verdict(), Some(Verdict::Low));
        assert_eq!(
            result(15.0, 80.0).bandwidth_verdict(),
            Some(Verdict::VeryLow)
        );
        assert_eq!(result(40.0, 110.0).latency_verdict(), Some(Verdict::Good));
        assert_eq!(result(40.0, 130.0).latency_verdict(), Some(Verdict::Low));
        assert_eq!(
            result(40.0, 150.0).latency_verdict(),
            Some(Verdict::VeryLow)
        );
        assert!((result(25.6, 0.0).efficiency_pct().unwrap() - 50.0).abs() < 1e-9);
        let none = MemTestResult::default();
        assert_eq!(none.bandwidth_verdict(), None);
        assert_eq!(none.latency_verdict(), None);
        assert_eq!(none.efficiency_pct(), None);
    }

    #[test]
    fn sattolo_is_one_cycle() {
        for n in [2, 3, 10, 1000] {
            let p = sattolo(n, n as u64);
            let (mut i, mut seen) = (0usize, vec![false; n]);
            for _ in 0..n {
                assert!(!seen[i]);
                seen[i] = true;
                i = p[i] as usize;
            }
            assert_eq!(i, 0);
            assert!(seen.iter().all(|&s| s));
        }
    }

    #[test]
    fn buffer_sizes() {
        assert!(buffer_bytes(GIB - 1).is_err());
        assert_eq!(buffer_bytes(2 * GIB), Ok(256 * MIB));
        assert_eq!(buffer_bytes(16 * GIB), Ok(GIB));
    }

    #[test]
    fn kernels_and_smoke() {
        assert_eq!(read_sum(&(1..=19).collect::<Vec<u64>>()), 190);
        let mut v = vec![0u64; 10];
        assert_eq!(split(&mut v, 3).len(), 3);
        assert_eq!(split(&mut v, 20).len(), 10);
        let r = measure(
            4 * MIB,
            2,
            &Cpus::default(),
            1,
            10_000,
            &Progress::default(),
        )
        .unwrap();
        assert!(r.read_gbs > 0.0 && r.read_single_gbs > 0.0);
        assert!(r.write_gbs > 0.0 && r.copy_gbs > 0.0);
        assert!(r.latency_ns > 0.0);
        assert_eq!(r.buffer_mb, 4);
    }

    #[test]
    fn stream_read_covers_aligned_buffer() {
        let v = vec![7u64; 4096 + 3];
        let n = stream_read(&v);
        assert_eq!(n, stream_len(&v));
        // Whole blocks of the 64-byte aligned middle: short by at most alignment plus one block.
        assert!(n > 0 && v.len() * 8 - n < 256 + 64);
        assert_eq!(n % 128, 0);
    }

    #[test]
    fn read_core_prefers_last_p_core() {
        let t = Topology::parse(&crate::topology::build::hybrid_2p4e()).unwrap();
        let id = read_core(&t).unwrap();
        assert!(t.is_p_core(&t.cores[t.core_of(id).unwrap()]));
        assert!(
            id > LogicalId {
                group: 0,
                number: 0
            }
        );
        assert!(read_core(&Topology::default()).is_none());
    }

    #[test]
    fn worker_cpus_cover_cores_first_and_skip_cpu0() {
        // 2 P-cores with SMT (0-1, 2-3) + 4 E-cores (4-7).
        let t = Topology::parse(&crate::topology::build::hybrid_2p4e()).unwrap();
        let ids: Vec<u8> = worker_cpus(&t).iter().map(|id| id.number).collect();
        assert_eq!(ids, [2, 4, 5, 6, 7, 1, 3]);
        assert!(worker_cpus(&Topology::default()).is_empty());
    }

    #[test]
    fn progress_steps() {
        let p = Progress::default();
        assert_eq!(p.read(), (Phase::Preparing, 1, 6));
        p.set(Phase::ReadSingle);
        assert_eq!(p.read(), (Phase::ReadSingle, 3, 6));
        p.set(Phase::Latency);
        assert_eq!(p.read(), (Phase::Latency, 6, 6));
        p.set(Phase::Done);
        assert_eq!(p.read(), (Phase::Done, 6, 6));
    }
}
