//! The single background thread: gathers static info once, then polls sensor
//! backends on an interval and asks the UI to repaint after each sample.

use crate::notify::Notifier;
use omnihw_core::SystemInfo;
use omnihw_core::alerts::{self, AlertEvent, AlertRule, Alerts};
use omnihw_core::power::{Action, PowerInfo};
use omnihw_core::recording::Recording;
use omnihw_core::sensors::{SensorBackend, SensorTable};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const DEFAULT_INTERVAL_MS: u64 = 1000;
/// Fired alerts kept for the log.
const ALERT_LOG_LEN: usize = 200;
/// How often power plan settings are re-read (devices and wake history only on request).
const POWER_SETTINGS_EVERY: Duration = Duration::from_secs(30);

/// Latest sensor state, shared with the UI.
#[derive(Default)]
pub struct Snapshot {
    pub table: SensorTable,
    /// Backend errors from the most recent poll.
    pub errors: Vec<String>,
    pub samples: u64,
    /// Active recording, appended to on every sample.
    pub recording: Option<Recorder>,
    /// The most recently stopped recording, kept for viewing.
    pub last_recording: Option<(Recording, PathBuf)>,
    /// Alert rules and their state, evaluated on every sample.
    pub alerts: Alerts,
    /// Fired alerts, newest last.
    pub alert_log: Vec<AlertEvent>,
    /// Power settings, devices and wake history (None until first read).
    pub power: Option<PowerInfo>,
    /// Bumped whenever `power` changes, so the UI can tell when to recompute what depends on it.
    pub power_rev: u64,
    /// A power change or elevated query is in progress.
    pub power_busy: bool,
    /// Result of the last power change, for the status bar.
    pub power_msg: Option<String>,
}

/// An in-progress recording: kept in memory for charts and streamed to a CSV file.
pub struct Recorder {
    pub rec: Recording,
    pub path: PathBuf,
    file: BufWriter<File>,
    started: Instant,
    /// Set if writing the CSV failed; recording continues in memory.
    pub write_error: Option<String>,
}

impl Snapshot {
    /// The recording to show: the active one, else the last stopped one.
    pub fn viewed_recording(&self) -> Option<(&Recording, &PathBuf, bool)> {
        match &self.recording {
            Some(r) => Some((&r.rec, &r.path, true)),
            None => self.last_recording.as_ref().map(|(r, p)| (r, p, false)),
        }
    }
}

struct Shared {
    info: OnceLock<SystemInfo>,
    snapshot: Mutex<Snapshot>,
    interval_ms: AtomicU64,
    reset: AtomicBool,
    /// Main window handle for notifications (0 until the UI reports it).
    hwnd: AtomicIsize,
    test_notification: AtomicBool,
    /// Windows notifications for fired alerts (they always show in-app).
    alert_notifications: AtomicBool,
    /// Re-read all power info (devices, wake history) on the next sample.
    power_refresh: AtomicBool,
}

/// Where alert rules are saved: `%APPDATA%/OmniHW/alerts.tsv`.
fn alerts_path() -> Option<PathBuf> {
    Some(
        PathBuf::from(std::env::var_os("APPDATA")?)
            .join("OmniHW")
            .join("alerts.tsv"),
    )
}

fn load_alert_rules() -> Vec<AlertRule> {
    alerts_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|t| alerts::from_config(&t))
        .unwrap_or_default()
}

pub struct Sampler {
    shared: Arc<Shared>,
    ctx: egui::Context,
    /// Dropping this wakes and stops the thread.
    _stop: mpsc::Sender<()>,
}

impl Sampler {
    pub fn start(ctx: egui::Context) -> Self {
        let shared = Arc::new(Shared {
            info: OnceLock::new(),
            snapshot: Mutex::new(Snapshot {
                alerts: Alerts::new(load_alert_rules()),
                ..Default::default()
            }),
            interval_ms: AtomicU64::new(DEFAULT_INTERVAL_MS),
            reset: AtomicBool::new(false),
            hwnd: AtomicIsize::new(0),
            test_notification: AtomicBool::new(false),
            alert_notifications: AtomicBool::new(true),
            power_refresh: AtomicBool::new(false),
        });
        let (stop, stop_rx) = mpsc::channel::<()>();
        let s = shared.clone();
        let c = ctx.clone();
        std::thread::Builder::new()
            .name("omnihw-sampler".into())
            .spawn(move || run(s, c, stop_rx))
            .expect("spawn sampler thread");
        Sampler {
            shared,
            ctx,
            _stop: stop,
        }
    }

    /// Re-read power settings, devices and wake history on the next sample.
    pub fn refresh_power(&self) {
        self.shared.power_refresh.store(true, Ordering::Relaxed);
    }

    /// Run blocking power work (which may wait on a UAC prompt) on a short-lived worker thread,
    /// then report `msg(result)` and refresh the power info.
    fn power_job<T: Send + 'static>(
        &self,
        work: impl FnOnce() -> T + Send + 'static,
        done: impl FnOnce(T, &mut Snapshot) + Send + 'static,
    ) {
        {
            let mut snap = self.snapshot();
            if snap.power_busy {
                return;
            }
            snap.power_busy = true;
        }
        let (shared, ctx) = (self.shared.clone(), self.ctx.clone());
        let spawned = std::thread::Builder::new()
            .name("omnihw-power".into())
            .spawn(move || {
                let r = work();
                {
                    let mut snap = shared.snapshot.lock().unwrap_or_else(|e| e.into_inner());
                    snap.power_busy = false;
                    done(r, &mut snap);
                    snap.power_rev += 1;
                }
                shared.power_refresh.store(true, Ordering::Relaxed);
                ctx.request_repaint();
            });
        if spawned.is_err() {
            self.snapshot().power_busy = false;
        }
    }

    /// Apply a power change (asking for administrator rights if needed).
    pub fn power_action(&self, action: Action) {
        let a = action.clone();
        self.power_job(
            move || crate::power_ops::apply(&a),
            move |r, snap| {
                snap.power_msg = Some(match r {
                    Ok(()) => format!("Done: {}", action.label()),
                    Err(e) => format!("{} failed: {e}", action.label()),
                });
            },
        );
    }

    /// Read power requests and wake timers (asks for administrator rights if needed).
    pub fn power_query_admin(&self) {
        self.power_job(crate::power_ops::query_admin, |r, snap| match r {
            Ok((requests, timers)) => {
                if let Some(p) = &mut snap.power {
                    p.requests = Some(requests);
                    p.wake_timers = Some(timers);
                }
            }
            Err(e) => snap.power_msg = Some(format!("Couldn't read power requests: {e}")),
        });
    }

    /// Static info, once gathered.
    pub fn info(&self) -> Option<&SystemInfo> {
        self.shared.info.get()
    }

    pub fn snapshot(&self) -> std::sync::MutexGuard<'_, Snapshot> {
        self.shared
            .snapshot
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    pub fn interval_ms(&self) -> u64 {
        self.shared.interval_ms.load(Ordering::Relaxed)
    }

    pub fn set_interval_ms(&self, ms: u64) {
        self.shared.interval_ms.store(ms, Ordering::Relaxed);
    }

    pub fn reset_stats(&self) {
        self.shared.reset.store(true, Ordering::Relaxed);
    }

    /// Turn Windows notifications (and the taskbar flash) for fired alerts on or off.
    pub fn set_alert_notifications(&self, on: bool) {
        self.shared.alert_notifications.store(on, Ordering::Relaxed);
    }

    pub fn set_window_handle(&self, hwnd: isize) {
        self.shared.hwnd.store(hwnd, Ordering::Relaxed);
    }

    /// Show a sample notification on the next sample.
    pub fn test_notification(&self) {
        self.shared.test_notification.store(true, Ordering::Relaxed);
    }

    /// Replace the alert rules and save them. Returns an error message if saving failed.
    pub fn set_alert_rules(&self, rules: Vec<AlertRule>) -> Result<(), String> {
        let text = alerts::to_config(&rules);
        self.snapshot().alerts.set_rules(rules);
        let path = alerts_path().ok_or("APPDATA is not set")?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(&path, text).map_err(|e| format!("Could not save {}: {e}", path.display()))
    }

    /// Start recording every sensor to a new CSV file in `dir`. Requires at least one sample.
    pub fn start_recording(&self, dir: &std::path::Path, stamp: &str) -> Result<PathBuf, String> {
        let mut snap = self.snapshot();
        if snap.recording.is_some() {
            return Err("Already recording".into());
        }
        if snap.table.entries().is_empty() {
            return Err("No sensor data yet; try again in a moment".into());
        }
        let path = dir.join(format!("OmniHW-Recording-{stamp}.csv"));
        let file =
            File::create(&path).map_err(|e| format!("Could not create {}: {e}", path.display()))?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        let mut rec = Recording::new(now, &snap.table);
        let mut file = BufWriter::new(file);
        let write_error = file
            .write_all(rec.csv_header().as_bytes())
            .err()
            .map(|e| e.to_string());
        let i = rec.push(0.0, &snap.table);
        let mut r = Recorder {
            file,
            path: path.clone(),
            started: Instant::now(),
            write_error,
            rec,
        };
        r.write_row(i);
        snap.recording = Some(r);
        Ok(path)
    }

    /// Stop the active recording, keeping it for viewing. Returns the CSV path.
    pub fn stop_recording(&self) -> Option<PathBuf> {
        let mut snap = self.snapshot();
        let mut r = snap.recording.take()?;
        let _ = r.file.flush();
        snap.last_recording = Some((r.rec, r.path.clone()));
        Some(r.path)
    }
}

fn backends(info: &SystemInfo) -> (Vec<Box<dyn SensorBackend>>, Vec<String>) {
    #[allow(unused_mut)]
    let mut list: Vec<Box<dyn SensorBackend>> = Vec::new();
    #[allow(unused_mut)]
    let mut errors = Vec::new();
    #[cfg(windows)]
    {
        use omnihw_core::win::nvml::NvmlBackend;
        use omnihw_core::win::perf::{
            BatteryBackend, CpuPerfBackend, GpuPerfBackend, MemoryBackend,
        };
        use omnihw_core::win::storage::StorageBackend;
        match CpuPerfBackend::new(info.base_mhz.unwrap_or(0), info.topology.as_ref()) {
            Ok(b) => list.push(Box::new(b)),
            Err(e) => errors.push(format!("CPU counters: {e}")),
        }
        list.push(Box::new(MemoryBackend));
        if info.battery.is_some() {
            list.push(Box::new(BatteryBackend));
        }
        if !info.drives.is_empty() {
            list.push(Box::new(StorageBackend::new(&info.drives)));
        }
        // Vendor sensors first so they lead each GPU's group in the sensor list.
        if let Some(b) = NvmlBackend::new(&info.gpus) {
            list.push(Box::new(b));
        }
        if !info.gpus.is_empty() {
            match GpuPerfBackend::new(&info.gpus) {
                Ok(b) => list.push(Box::new(b)),
                Err(e) => errors.push(format!("GPU counters: {e}")),
            }
        }
    }
    #[cfg(not(windows))]
    let _ = info;
    (list, errors)
}

fn run(shared: Arc<Shared>, ctx: egui::Context, stop: mpsc::Receiver<()>) {
    let info = shared.info.get_or_init(SystemInfo::gather);
    ctx.request_repaint();

    let (mut backends, init_errors) = backends(info);
    let mut readings = Vec::new();
    let started = Instant::now();
    let mut notifier: Option<Notifier> = None;
    let mut power_read: Option<Instant> = None;
    loop {
        let interval = Duration::from_millis(shared.interval_ms.load(Ordering::Relaxed));
        match stop.recv_timeout(interval) {
            Err(RecvTimeoutError::Timeout) => {}
            _ => return,
        }

        readings.clear();
        let mut errors = init_errors.clone();
        for b in &mut backends {
            if let Err(e) = b.poll(&mut readings) {
                errors.push(format!("{}: {e}", b.name()));
            }
        }

        let mut snap = shared.snapshot.lock().unwrap_or_else(|e| e.into_inner());
        snap.table.update(readings.drain(..));
        if shared.reset.swap(false, Ordering::Relaxed) {
            snap.table.reset_stats();
        }
        snap.errors = errors;
        snap.samples += 1;
        let s = &mut *snap;
        if let Some(r) = &mut s.recording {
            let i = r.rec.push(r.started.elapsed().as_secs_f64(), &s.table);
            r.write_row(i);
        }
        let now_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        let fired = s
            .alerts
            .evaluate(started.elapsed().as_secs_f64(), now_unix, &s.table);
        s.alert_log.extend(fired.iter().cloned());
        let excess = s.alert_log.len().saturating_sub(ALERT_LOG_LEN);
        s.alert_log.drain(..excess);
        drop(snap);

        // Notify outside the lock.
        let hwnd = shared.hwnd.load(Ordering::Relaxed);
        if hwnd != 0 && notifier.is_none() {
            notifier = Some(Notifier::new(hwnd));
        }
        if let Some(n) = &mut notifier {
            let notify_alerts = shared.alert_notifications.load(Ordering::Relaxed);
            for e in fired.iter().filter(|_| notify_alerts) {
                n.show("Omni-HW alert", &e.message());
            }
            if shared.test_notification.swap(false, Ordering::Relaxed) {
                n.show(
                    "Omni-HW",
                    "Test notification: alerts will appear like this.",
                );
            }
        }
        update_power(&shared, &mut power_read);
        ctx.request_repaint();
    }
}

/// Read power info outside the snapshot lock: everything at startup and on request, plan
/// settings every `POWER_SETTINGS_EVERY`.
fn update_power(shared: &Shared, last: &mut Option<Instant>) {
    let full = shared.power_refresh.swap(false, Ordering::Relaxed) || last.is_none();
    if !full && last.is_some_and(|t| t.elapsed() < POWER_SETTINGS_EVERY) {
        return;
    }
    let fresh = if full {
        PowerInfo::gather()
    } else {
        PowerInfo::gather_settings()
    };
    *last = Some(Instant::now());
    let mut snap = shared.snapshot.lock().unwrap_or_else(|e| e.into_inner());
    snap.power_rev += 1;
    match (&mut snap.power, full) {
        (Some(old), false) => old.merge_settings(fresh),
        (old, _) => {
            let mut fresh = fresh;
            // Power requests read with a one-off elevation stay until the next elevated read.
            if fresh.requests.is_none()
                && let Some(o) = old
            {
                fresh.requests = o.requests.take();
                fresh.wake_timers = o.wake_timers.take();
            }
            *old = Some(fresh);
        }
    }
}

impl Recorder {
    /// Append sample `i` to the CSV, flushing so a crash loses at most one row.
    fn write_row(&mut self, i: usize) {
        if self.write_error.is_some() {
            return;
        }
        let res = self
            .file
            .write_all(self.rec.csv_row(i).as_bytes())
            .and_then(|_| self.file.flush());
        if let Err(e) = res {
            self.write_error = Some(e.to_string());
        }
    }
}
