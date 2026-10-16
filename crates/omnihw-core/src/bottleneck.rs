//! Bottleneck analysis: from a recording, tell whether the GPU or the CPU was the limit,
//! or neither (frame cap, V-Sync, engine limit), in plain language with tips.
//!
//! Works on in-memory recordings (sensor ids such as `gpu/<luid>/nv/load`) and on recordings
//! loaded from CSV (ids are "group / label", e.g. "GPU: NVIDIA GeForce RTX 3090 / GPU Core Load").
//! The CPU side uses the *busiest thread*, since games are often limited by one thread while
//! total CPU usage looks low.

use crate::recording::{Column, Recording};

/// GPU load at or above this means the GPU is the limit.
pub const GPU_BOUND: f32 = 90.0;
/// CPU-bound requires the GPU to have headroom (below this)...
pub const GPU_HEADROOM: f32 = 80.0;
/// ...and either one thread at or above this...
pub const THREAD_BUSY: f32 = 90.0;
/// ...or total CPU at or above this.
pub const CPU_TOTAL_BUSY: f32 = 85.0;
/// Samples with GPU and CPU both below this are idle and ignored.
pub const IDLE: f32 = 15.0;
/// Fewer active samples than this gives a low-confidence verdict.
pub const MIN_ACTIVE: usize = 30;
/// A contributing factor must show up in at least this share of active samples.
const FACTOR_SHARE: f64 = 0.10;

/// What limited performance in one sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Limit {
    GpuBound,
    CpuBound,
    /// Neither GPU nor CPU saturated: frame cap, V-Sync or an engine limit.
    NotLimited,
    Idle,
}

/// Overall verdict for a recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    GpuBound,
    CpuBound,
    NotLimited,
    /// No class reached half of the active time.
    Mixed,
    /// Nothing was running.
    Idle,
}

/// Columns the analysis needs, located in a recording.
#[derive(Debug, Clone, PartialEq)]
pub struct Columns {
    pub gpu_load: usize,
    /// e.g. "NVIDIA GeForce RTX 3090"
    pub gpu_name: String,
    pub cpu_total: Option<usize>,
    pub cpu_threads: Vec<usize>,
    pub mem_load: Option<usize>,
    /// VRAM in use, MB.
    pub vram_used: Option<usize>,
}

/// The result of [`analyze`].
#[derive(Debug, Clone, PartialEq)]
pub struct Analysis {
    pub columns: Columns,
    /// Per-sample class; `None` where GPU load is missing.
    pub timeline: Vec<Option<Limit>>,
    /// Busiest CPU thread per sample (NaN when unknown), for charting.
    pub busiest_thread: Vec<f32>,
    pub samples: usize,
    pub active: usize,
    pub idle: usize,
    pub gpu_bound: usize,
    pub cpu_bound: usize,
    pub not_limited: usize,
    pub verdict: Verdict,
    /// Things that made it worse (VRAM or RAM full, single-thread limit).
    pub factors: Vec<String>,
    /// Reasons to treat the verdict with care.
    pub caveats: Vec<String>,
}

fn gpu_name_of(group: &str) -> String {
    group.strip_prefix("GPU: ").unwrap_or(group).to_string()
}

fn is_gpu_vendor_load(c: &Column) -> bool {
    (c.id.starts_with("gpu/") && c.id.ends_with("/nv/load"))
        || (c.group.starts_with("GPU: ") && c.label == "GPU Core Load")
}

fn is_gpu_engine_load(c: &Column) -> bool {
    (c.id.starts_with("gpu/") && c.id.ends_with("/load") && !c.id.contains("/nv/"))
        || (c.group.starts_with("GPU: ") && c.label == "GPU Load (busiest engine)")
}

fn mean(vals: &[f32]) -> f32 {
    let (s, n) = vals
        .iter()
        .filter(|v| v.is_finite())
        .fold((0.0f64, 0usize), |(s, n), &v| (s + v as f64, n + 1));
    if n == 0 {
        f32::NAN
    } else {
        (s / n as f64) as f32
    }
}

/// Find the needed columns. Prefers the vendor's GPU load (what GPU-Z shows) over the
/// busiest-engine figure, and with several GPUs picks the one that worked hardest.
pub fn find_columns(rec: &Recording) -> Result<Columns, String> {
    let busiest_gpu = |pred: fn(&Column) -> bool| {
        rec.columns
            .iter()
            .enumerate()
            .filter(|(_, c)| pred(c))
            .max_by(|(a, _), (b, _)| {
                mean(&rec.values[*a])
                    .partial_cmp(&mean(&rec.values[*b]))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(i, _)| i)
    };
    let gpu_load = busiest_gpu(is_gpu_vendor_load)
        .or_else(|| busiest_gpu(is_gpu_engine_load))
        .ok_or("This recording has no GPU load data. Bottleneck analysis needs GPU load to tell CPU and GPU limits apart.")?;
    let gpu_group = rec.columns[gpu_load].group.clone();
    let gpu_prefix = rec.columns[gpu_load]
        .id
        .rsplit_once("/load")
        .map(|(p, _)| format!("{}/", p.trim_end_matches("/nv")));

    let find = |pred: &dyn Fn(&Column) -> bool| rec.columns.iter().position(pred);
    let in_gpu = |c: &Column| {
        c.group == gpu_group || gpu_prefix.as_deref().is_some_and(|p| c.id.starts_with(p))
    };
    let vram_used =
        find(&|c| in_gpu(c) && (c.id.ends_with("/vram/used") || c.label == "GPU Memory Used"))
            .or_else(|| {
                find(&|c| {
                    in_gpu(c)
                        && (c.id.ends_with("/vram/dedicated")
                            || c.label == "Dedicated Memory Usage")
                })
            });

    let cpu_total = find(&|c| c.id == "cpu/load/total" || c.key() == "CPU Usage / Total CPU Usage");
    let cpu_threads = rec
        .columns
        .iter()
        .enumerate()
        .filter(|(i, c)| {
            Some(*i) != cpu_total
                && ((c.id.starts_with("cpu/load/") && c.id != "cpu/load/total")
                    || (c.group == "CPU Usage"
                        && c.label.ends_with(" Usage")
                        && c.label != "Total CPU Usage"))
        })
        .map(|(i, _)| i)
        .collect();
    let mem_load = find(&|c| c.id == "mem/load" || c.key() == "Memory / Physical Memory Load");

    Ok(Columns {
        gpu_load,
        gpu_name: gpu_name_of(&gpu_group),
        cpu_total,
        cpu_threads,
        mem_load,
        vram_used,
    })
}

/// Classify one sample. `None` GPU load means unknown.
pub fn classify(gpu: f32, busiest_thread: f32, cpu_total: f32) -> Limit {
    let total = if cpu_total.is_finite() {
        cpu_total
    } else {
        busiest_thread
    };
    let total_low = !total.is_finite() || total < IDLE;
    if gpu < IDLE && total_low {
        return Limit::Idle;
    }
    if gpu >= GPU_BOUND {
        return Limit::GpuBound;
    }
    let thread_busy = busiest_thread.is_finite() && busiest_thread >= THREAD_BUSY;
    let total_busy = cpu_total.is_finite() && cpu_total >= CPU_TOTAL_BUSY;
    if gpu < GPU_HEADROOM && (thread_busy || total_busy) {
        Limit::CpuBound
    } else {
        Limit::NotLimited
    }
}

/// Analyze a recording. `vram_total_mb` (the adapter's dedicated VRAM) enables the
/// "VRAM nearly full" factor when known.
pub fn analyze(rec: &Recording, vram_total_mb: Option<f64>) -> Result<Analysis, String> {
    let cols = find_columns(rec)?;
    let n = rec.len();
    let val = |c: Option<usize>, i: usize| c.map_or(f32::NAN, |c| rec.values[c][i]);
    let mut timeline = Vec::with_capacity(n);
    let mut busiest_thread = Vec::with_capacity(n);
    let (mut idle, mut gpu_b, mut cpu_b, mut none) = (0, 0, 0, 0);
    let (mut vram_full, mut ram_full, mut single_thread) = (0usize, 0usize, 0usize);

    for i in 0..n {
        let gpu = rec.values[cols.gpu_load][i];
        let busiest = cols
            .cpu_threads
            .iter()
            .map(|&c| rec.values[c][i])
            .filter(|v| v.is_finite())
            .fold(f32::NAN, f32::max);
        busiest_thread.push(busiest);
        if !gpu.is_finite() {
            timeline.push(None);
            continue;
        }
        let total = val(cols.cpu_total, i);
        let class = classify(gpu, busiest, total);
        timeline.push(Some(class));
        match class {
            Limit::Idle => {
                idle += 1;
                continue;
            }
            Limit::GpuBound => gpu_b += 1,
            Limit::CpuBound => {
                cpu_b += 1;
                if total.is_finite() && total < 50.0 && busiest >= THREAD_BUSY {
                    single_thread += 1;
                }
            }
            Limit::NotLimited => none += 1,
        }
        if let (Some(total_mb), Some(c)) = (vram_total_mb, cols.vram_used) {
            let used = rec.values[c][i];
            if total_mb > 0.0 && used.is_finite() && used as f64 >= total_mb * 0.95 {
                vram_full += 1;
            }
        }
        if val(cols.mem_load, i) >= 90.0 {
            ram_full += 1;
        }
    }

    let active = gpu_b + cpu_b + none;
    let share = |x: usize| {
        if active == 0 {
            0.0
        } else {
            x as f64 / active as f64
        }
    };
    let verdict = if active == 0 {
        Verdict::Idle
    } else {
        let best = [
            (gpu_b, Verdict::GpuBound),
            (cpu_b, Verdict::CpuBound),
            (none, Verdict::NotLimited),
        ]
        .into_iter()
        // On ties keep the earlier class (GPU, then CPU).
        .fold(
            (0, Verdict::GpuBound),
            |best, x| if x.0 > best.0 { x } else { best },
        );
        if share(best.0) >= 0.5 {
            best.1
        } else {
            Verdict::Mixed
        }
    };

    let mut factors = Vec::new();
    if share(vram_full) >= FACTOR_SHARE {
        factors.push(format!(
            "Video memory was nearly full {:.0} % of the time. Running out of VRAM causes stutter as data spills to system RAM; lower texture quality or resolution.",
            share(vram_full) * 100.0
        ));
    }
    if share(ram_full) >= FACTOR_SHARE {
        factors.push(format!(
            "System memory was over 90 % used {:.0} % of the time. Windows may be paging to disk; close background apps or add RAM.",
            share(ram_full) * 100.0
        ));
    }
    if cpu_b > 0 && single_thread * 2 >= cpu_b {
        factors.push(
            "When CPU-bound, one thread was maxed out while total CPU usage stayed low: the game is limited by single-thread speed, so more cores won't help."
                .into(),
        );
    }

    let mut caveats = Vec::new();
    if active > 0 && active < MIN_ACTIVE {
        caveats.push(format!(
            "Only {active} active samples. Record at least a few minutes of gameplay for a reliable verdict."
        ));
    }
    if cols.cpu_threads.is_empty() {
        caveats.push("No per-thread CPU data; CPU limits are judged from total CPU usage only, which can miss single-thread limits.".into());
    }
    let missing = timeline.iter().filter(|t| t.is_none()).count();
    if missing > 0 {
        caveats.push(format!(
            "{missing} samples had no GPU load reading and were skipped."
        ));
    }

    Ok(Analysis {
        columns: cols,
        timeline,
        busiest_thread,
        samples: n,
        active,
        idle,
        gpu_bound: gpu_b,
        cpu_bound: cpu_b,
        not_limited: none,
        verdict,
        factors,
        caveats,
    })
}

impl Analysis {
    /// Percentages of active time: (GPU-bound, CPU-bound, not limited).
    pub fn percentages(&self) -> (f64, f64, f64) {
        if self.active == 0 {
            return (0.0, 0.0, 0.0);
        }
        let p = |x: usize| x as f64 * 100.0 / self.active as f64;
        (p(self.gpu_bound), p(self.cpu_bound), p(self.not_limited))
    }

    /// One-line verdict, e.g. "GPU-bound 87 % of the time".
    pub fn headline(&self) -> String {
        let (g, c, n) = self.percentages();
        match self.verdict {
            Verdict::GpuBound => format!("GPU-bound {g:.0} % of the time"),
            Verdict::CpuBound => format!("CPU-bound {c:.0} % of the time"),
            Verdict::NotLimited => format!("Not limited by CPU or GPU {n:.0} % of the time"),
            Verdict::Mixed => {
                format!("Mixed: GPU-bound {g:.0} %, CPU-bound {c:.0} %, neither {n:.0} %")
            }
            Verdict::Idle => "The system was idle during this recording".into(),
        }
    }

    /// What the verdict means, in plain language.
    pub fn explanation(&self) -> String {
        let gpu = &self.columns.gpu_name;
        match self.verdict {
            Verdict::GpuBound => format!(
                "The {gpu} was running flat out most of the time, so it set the frame rate. This is normal and expected when playing at high settings or resolution: the graphics card is doing its job."
            ),
            Verdict::CpuBound => format!(
                "The {gpu} had spare capacity while the CPU (often a single thread) was maxed out, so the processor set the frame rate. A faster graphics card would not help here."
            ),
            Verdict::NotLimited => format!(
                "Neither the CPU nor the {gpu} was saturated. The frame rate is most likely capped by a frame limiter, V-Sync, the monitor's refresh rate or the game engine itself. That's usually fine."
            ),
            Verdict::Mixed => format!(
                "The limit moved between the CPU and the {gpu} during the session, which is common in games with both busy scenes (CPU-heavy) and demanding visuals (GPU-heavy)."
            ),
            Verdict::Idle => "No game or heavy workload was running. Start a recording, play for a few minutes, then stop it and analyze that recording.".into(),
        }
    }

    /// What to try, depending on the verdict.
    pub fn tips(&self) -> Vec<String> {
        let t: &[&str] = match self.verdict {
            Verdict::GpuBound => &[
                "Want more FPS? Lower resolution or GPU-heavy settings (shadows, ray tracing, anti-aliasing), or enable DLSS/FSR/XeSS upscaling.",
                "If FPS is already high enough, you can raise settings: the CPU has headroom.",
                "Make sure the GPU isn't thermal throttling (see GPU temperature) and that Resizable BAR is on.",
            ],
            Verdict::CpuBound => &[
                "Lower CPU-heavy settings: view distance, crowd/NPC density, physics and simulation detail.",
                "Raising resolution or graphics quality is nearly free here: the GPU has headroom.",
                "Enable a frame cap a little below your typical FPS for smoother frame pacing.",
                "Close background apps (browsers, launchers, overlays) and make sure XMP/EXPO is enabled for your RAM.",
            ],
            Verdict::NotLimited => &[
                "If FPS matches your monitor refresh rate or a frame cap, everything is working as intended.",
                "If FPS is lower than expected, check the in-game frame limiter, V-Sync and the GPU driver's frame-rate limit.",
            ],
            Verdict::Mixed => &[
                "Look at the timeline to see which scenes are CPU-limited and which are GPU-limited.",
                "Upscaling (DLSS/FSR/XeSS) helps GPU-limited scenes; a frame cap smooths out CPU-limited ones.",
            ],
            Verdict::Idle => &["Record while a game is running to get a verdict."],
        };
        t.iter().map(|s| s.to_string()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sensors::Unit;

    /// Build a recording from (id, group, label, values) columns.
    fn rec(cols: &[(&str, &str, &str, Unit, Vec<f32>)]) -> Recording {
        let n = cols[0].4.len();
        Recording {
            started_unix: 0.0,
            columns: cols
                .iter()
                .map(|(id, g, l, u, _)| Column {
                    id: id.to_string(),
                    group: g.to_string(),
                    label: l.to_string(),
                    unit: *u,
                })
                .collect(),
            times: (0..n).map(|i| i as f64).collect(),
            values: cols.iter().map(|c| c.4.clone()).collect(),
        }
    }

    fn rep(v: f32, n: usize) -> Vec<f32> {
        vec![v; n]
    }

    const G: &str = "GPU: NVIDIA GeForce RTX 3090";

    #[test]
    fn classification() {
        assert_eq!(classify(98.0, 60.0, 40.0), Limit::GpuBound);
        assert_eq!(classify(55.0, 99.0, 30.0), Limit::CpuBound);
        assert_eq!(classify(55.0, 70.0, 88.0), Limit::CpuBound);
        assert_eq!(
            classify(85.0, 99.0, 30.0),
            Limit::NotLimited,
            "GPU 80-90 % isn't clearly either"
        );
        assert_eq!(classify(40.0, 50.0, 30.0), Limit::NotLimited);
        assert_eq!(classify(5.0, 10.0, 8.0), Limit::Idle);
        assert_eq!(classify(5.0, f32::NAN, f32::NAN), Limit::Idle);
    }

    #[test]
    fn gpu_bound_with_memory_ids() {
        let r = rec(&[
            (
                "gpu/ef68/nv/load",
                G,
                "GPU Core Load",
                Unit::Percent,
                rep(97.0, 60),
            ),
            (
                "gpu/ef68/load",
                G,
                "GPU Load (busiest engine)",
                Unit::Percent,
                rep(70.0, 60),
            ),
            (
                "cpu/load/total",
                "CPU Usage",
                "Total CPU Usage",
                Unit::Percent,
                rep(35.0, 60),
            ),
            (
                "cpu/load/0,0",
                "CPU Usage",
                "P-core 0 / T0 Usage",
                Unit::Percent,
                rep(60.0, 60),
            ),
            (
                "gpu/ef68/vram/used",
                G,
                "GPU Memory Used",
                Unit::Megabytes,
                rep(23_500.0, 60),
            ),
        ]);
        let a = analyze(&r, Some(24_000.0)).unwrap();
        assert_eq!(a.columns.gpu_load, 0, "vendor load preferred");
        assert_eq!(a.columns.gpu_name, "NVIDIA GeForce RTX 3090");
        assert_eq!(a.columns.vram_used, Some(4));
        assert_eq!(a.verdict, Verdict::GpuBound);
        assert_eq!(a.headline(), "GPU-bound 100 % of the time");
        assert!(a.factors.iter().any(|f| f.contains("Video memory")));
        assert!(a.caveats.is_empty());
        assert!(a.explanation().contains("RTX 3090"));
    }

    #[test]
    fn cpu_bound_single_thread_with_csv_keys() {
        let mut thread0 = rep(99.0, 40);
        thread0.extend(rep(98.0, 10));
        let r = rec(&[
            (
                "CPU Usage / Total CPU Usage",
                "CPU Usage",
                "Total CPU Usage",
                Unit::Percent,
                rep(30.0, 50),
            ),
            (
                "CPU Usage / P-core 0 / T0 Usage",
                "CPU Usage",
                "P-core 0 / T0 Usage",
                Unit::Percent,
                thread0,
            ),
            (
                "CPU Usage / E-core 3 / T15 Usage",
                "CPU Usage",
                "E-core 3 / T15 Usage",
                Unit::Percent,
                rep(20.0, 50),
            ),
            (
                &format!("{G} / GPU Core Load"),
                G,
                "GPU Core Load",
                Unit::Percent,
                rep(55.0, 50),
            ),
            (
                "Memory / Physical Memory Load",
                "Memory",
                "Physical Memory Load",
                Unit::Percent,
                rep(95.0, 50),
            ),
        ]);
        let a = analyze(&r, None).unwrap();
        assert_eq!(a.columns.cpu_threads, vec![1, 2]);
        assert_eq!(a.columns.cpu_total, Some(0));
        assert_eq!(a.verdict, Verdict::CpuBound);
        assert_eq!(a.busiest_thread[0], 99.0);
        assert!(a.factors.iter().any(|f| f.contains("single-thread")));
        assert!(a.factors.iter().any(|f| f.contains("System memory")));
        assert!(a.tips().iter().any(|t| t.contains("view distance")));
    }

    #[test]
    fn engine_load_fallback_mixed_idle_and_caveats() {
        let mut gpu = rep(95.0, 10);
        gpu.extend(rep(50.0, 10));
        gpu.extend(rep(5.0, 10));
        let mut cpu = rep(40.0, 10);
        cpu.extend(rep(95.0, 10));
        cpu.extend(rep(5.0, 10));
        let mut threads = cpu.clone();
        threads[0] = f32::NAN;
        let r = rec(&[
            (
                "gpu/1/load",
                "GPU: Radeon",
                "GPU Load (busiest engine)",
                Unit::Percent,
                gpu,
            ),
            (
                "cpu/load/total",
                "CPU Usage",
                "Total CPU Usage",
                Unit::Percent,
                cpu,
            ),
            (
                "cpu/load/0,1",
                "CPU Usage",
                "CPU 1 Usage",
                Unit::Percent,
                threads,
            ),
        ]);
        let a = analyze(&r, None).unwrap();
        assert_eq!(a.columns.gpu_name, "Radeon");
        assert_eq!(
            (a.gpu_bound, a.cpu_bound, a.idle, a.active),
            (10, 10, 10, 20)
        );
        assert_eq!(
            a.verdict,
            Verdict::GpuBound,
            "ties go to the first class at 50 %"
        );
        assert!(
            a.caveats
                .iter()
                .any(|c| c.contains("Only 20 active samples"))
        );
        assert_eq!(a.timeline[25], Some(Limit::Idle));
    }

    #[test]
    fn mixed_and_not_limited_and_idle_verdicts() {
        let mut gpu = rep(95.0, 40);
        gpu.extend(rep(50.0, 30));
        gpu.extend(rep(50.0, 30));
        let mut cpu = rep(40.0, 40);
        cpu.extend(rep(95.0, 30));
        cpu.extend(rep(40.0, 30));
        let r = rec(&[
            ("gpu/1/nv/load", G, "GPU Core Load", Unit::Percent, gpu),
            (
                "cpu/load/total",
                "CPU Usage",
                "Total CPU Usage",
                Unit::Percent,
                cpu,
            ),
        ]);
        let a = analyze(&r, None).unwrap();
        assert_eq!(a.verdict, Verdict::Mixed);
        assert!(a.headline().starts_with("Mixed: GPU-bound 40 %"));
        assert!(
            a.caveats
                .iter()
                .any(|c| c.contains("No per-thread CPU data"))
        );

        let capped = rec(&[
            (
                "gpu/1/nv/load",
                G,
                "GPU Core Load",
                Unit::Percent,
                rep(60.0, 40),
            ),
            (
                "cpu/load/total",
                "CPU Usage",
                "Total CPU Usage",
                Unit::Percent,
                rep(30.0, 40),
            ),
        ]);
        assert_eq!(analyze(&capped, None).unwrap().verdict, Verdict::NotLimited);

        let idle = rec(&[
            (
                "gpu/1/nv/load",
                G,
                "GPU Core Load",
                Unit::Percent,
                rep(2.0, 40),
            ),
            (
                "cpu/load/total",
                "CPU Usage",
                "Total CPU Usage",
                Unit::Percent,
                rep(4.0, 40),
            ),
        ]);
        let a = analyze(&idle, None).unwrap();
        assert_eq!(a.verdict, Verdict::Idle);
        assert_eq!(a.percentages(), (0.0, 0.0, 0.0));
    }

    #[test]
    fn picks_busiest_gpu_and_reports_missing() {
        let mut busy = rep(97.0, 40);
        busy[3] = f32::NAN;
        let r = rec(&[
            (
                "gpu/a/nv/load",
                "GPU: Idle Card",
                "GPU Core Load",
                Unit::Percent,
                rep(1.0, 40),
            ),
            (
                "gpu/b/nv/load",
                "GPU: Busy Card",
                "GPU Core Load",
                Unit::Percent,
                busy,
            ),
            (
                "gpu/b/vram/dedicated",
                "GPU: Busy Card",
                "Dedicated Memory Usage",
                Unit::Megabytes,
                rep(100.0, 40),
            ),
            (
                "cpu/load/total",
                "CPU Usage",
                "Total CPU Usage",
                Unit::Percent,
                rep(20.0, 40),
            ),
        ]);
        let a = analyze(&r, None).unwrap();
        assert_eq!(a.columns.gpu_name, "Busy Card");
        assert_eq!(a.columns.vram_used, Some(2));
        assert_eq!(a.timeline[3], None);
        assert!(
            a.caveats
                .iter()
                .any(|c| c.contains("1 samples had no GPU load"))
        );
    }

    #[test]
    fn no_gpu_column_is_an_error() {
        let r = rec(&[(
            "cpu/load/total",
            "CPU Usage",
            "Total CPU Usage",
            Unit::Percent,
            rep(50.0, 5),
        )]);
        assert!(analyze(&r, None).unwrap_err().contains("GPU load"));
    }
}
