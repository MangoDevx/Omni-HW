//! The single background thread: gathers static info once, then polls sensor
//! backends on an interval and asks the UI to repaint after each sample.

use crate::notify::Notifier;
use omnihw_core::SystemInfo;
use omnihw_core::alerts::{self, AlertEvent, AlertRule, Alerts};
use omnihw_core::disktest::{self, DiskTestResult};
use omnihw_core::lansec::{self, LanScan};
use omnihw_core::memtest::{self, MemTestResult};
use omnihw_core::minidump::DumpAnalysis;
use omnihw_core::network::{self as net, NetAction, NetworkInfo, TestLine};
use omnihw_core::power::{Action, PowerInfo};
use omnihw_core::recording::Recording;
use omnihw_core::sensor_service::ServiceStatus;
use omnihw_core::sensors::{SensorBackend, SensorTable};
use omnihw_core::whea::WheaEvent;
use std::collections::HashMap;
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
/// Live WHEA events kept.
const WHEA_LIVE_LEN: usize = 200;
/// Event log times are whole seconds, so a run's start and end get this much slack.
const WHEA_SLACK: Duration = Duration::from_secs(1);
/// How often power plan settings are re-read (devices and wake history only on request).
const POWER_SETTINGS_EVERY: Duration = Duration::from_secs(30);
/// Network adapter re-reads (addresses, link, Wi-Fi) while the Network page is open / closed.
const NETWORK_EVERY_OPEN: Duration = Duration::from_secs(5);
const NETWORK_EVERY_CLOSED: Duration = Duration::from_secs(60);
/// Full re-reads including WMI details (driver, duplex, advanced properties).
const NETWORK_DETAILS_EVERY_OPEN: Duration = Duration::from_secs(30);
const NETWORK_DETAILS_EVERY_CLOSED: Duration = Duration::from_secs(300);
/// [`SensorBackend::name`] of the sensor-service backend.
const SERVICE_BACKEND: &str = "Sensor service";
/// How often to re-check a sensor service that is set up but not answering.
const SERVICE_RECHECK: Duration = Duration::from_secs(10);

/// Test results grouped under headings (adapter names; empty for the internet tests).
pub type TestGroups = Vec<(String, Vec<TestLine>)>;

/// A network test run: whether it's running and its last results, grouped by heading.
#[derive(Default, Clone)]
pub struct TestRun {
    pub running: bool,
    pub results: Option<(TestGroups, SystemTime)>,
}

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
    /// Network adapters (None until first read).
    pub network: Option<NetworkInfo>,
    /// A network change is in progress.
    pub net_busy: bool,
    /// Result of the last network change, for the status bar.
    pub net_msg: Option<String>,
    pub net_local: TestRun,
    pub net_internet: TestRun,
    /// LAN Security Report: scan progress and last results.
    pub lan: LanScanState,
    /// Disk speed test (Storage page): which drive is running and each drive's last result.
    pub disk_test: DiskTestState,
    /// Memory page quick test: progress and last result.
    pub mem_test: MemTestState,
    /// The optional sensor service (PawnIO): `None` until checked, or off Windows.
    pub sensor_service: Option<ServiceStatus>,
    /// WHEA hardware errors logged since the app started, newest last.
    pub whea_live: Vec<WheaEvent>,
    /// Bumped whenever `whea_live` gains events.
    pub whea_rev: u64,
    /// Analysed crash dumps (newest first), or why they couldn't be read; None until the user
    /// asks for them.
    pub dumps: Option<Result<Vec<DumpAnalysis>, String>>,
    /// Crash dumps are being read.
    pub dumps_busy: bool,
    /// Bumped whenever `dumps` or `dumps_busy` changes.
    pub dumps_rev: u64,
}

/// State of the LAN Security Report scan.
#[derive(Default, Clone)]
pub struct LanScanState {
    pub running: bool,
    /// Live progress of a running scan (phase and done/total).
    pub progress: Arc<lansec::Progress>,
    /// The last finished report, with the time it ran.
    pub results: Option<(LanScan, SystemTime)>,
}

/// State of the disk speed test. One drive is tested at a time.
#[derive(Default, Clone)]
pub struct DiskTestState {
    /// Index of the drive being tested.
    pub running: Option<u32>,
    /// Live progress of the running test.
    pub progress: Arc<disktest::Progress>,
    /// The last result per drive index, with the time it finished.
    pub results: HashMap<u32, (Result<DiskTestResult, String>, SystemTime)>,
    /// Per drive index: when its last run started and the WHEA errors logged during it.
    pub whea: HashMap<u32, (SystemTime, Vec<WheaEvent>)>,
}

/// State of the Memory page's quick bandwidth/latency test.
#[derive(Default, Clone)]
pub struct MemTestState {
    pub running: bool,
    /// Live phase of a running test.
    pub progress: Arc<memtest::Progress>,
    /// The last finished test (or why it failed), with the time it ran.
    pub result: Option<(Result<MemTestResult, String>, SystemTime)>,
    /// When the last run started.
    pub started: Option<SystemTime>,
    /// WHEA errors logged during the last (or running) test.
    pub whea_during: Vec<WheaEvent>,
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
    /// The Network page is open: read adapters more often.
    network_page: AtomicBool,
    /// Re-read network adapters with all details on the next sample.
    network_refresh: AtomicBool,
    /// A network read is running on its worker thread; the sampler schedules no second one.
    network_reading: AtomicBool,
    /// Read the sensor service's readings (the "Use extra sensors" setting).
    use_pawnio: AtomicBool,
    /// Re-check the sensor service (after setup or a settings change) on the next sample.
    service_refresh: AtomicBool,
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
    /// `use_pawnio`: read the extra sensors from the sensor service if it is set up.
    pub fn start(ctx: egui::Context, use_pawnio: bool) -> Self {
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
            network_page: AtomicBool::new(false),
            network_refresh: AtomicBool::new(false),
            network_reading: AtomicBool::new(false),
            use_pawnio: AtomicBool::new(use_pawnio),
            service_refresh: AtomicBool::new(false),
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

    /// Turn the sensor-service readings on or off (takes effect on the next sample).
    pub fn set_use_pawnio(&self, on: bool) {
        if self.shared.use_pawnio.swap(on, Ordering::Relaxed) != on {
            self.refresh_sensor_service();
        }
    }

    /// Re-check the sensor service on the next sample (e.g. after setting it up).
    pub fn refresh_sensor_service(&self) {
        self.shared.service_refresh.store(true, Ordering::Relaxed);
        self.ctx.request_repaint();
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

    /// Whether the Network page is open (it reads adapters more often then).
    pub fn set_network_page(&self, open: bool) {
        self.shared.network_page.store(open, Ordering::Relaxed);
    }

    /// Re-read network adapters, with all details, on the next sample.
    pub fn refresh_network(&self) {
        self.shared.network_refresh.store(true, Ordering::Relaxed);
    }

    /// Run `work` on a short-lived worker thread, then `done(result, snapshot)` and a repaint.
    /// Returns false if the thread couldn't be started.
    fn worker<T: Send + 'static>(
        &self,
        name: &str,
        work: impl FnOnce() -> T + Send + 'static,
        done: impl FnOnce(T, &mut Snapshot) + Send + 'static,
    ) -> bool {
        let (shared, ctx) = (self.shared.clone(), self.ctx.clone());
        std::thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                let r = work();
                done(
                    r,
                    &mut shared.snapshot.lock().unwrap_or_else(|e| e.into_inner()),
                );
                ctx.request_repaint();
            })
            .is_ok()
    }

    /// Apply a network change (asks for administrator rights when needed), then re-read.
    pub fn net_action(&self, action: NetAction) {
        {
            let mut snap = self.snapshot();
            if snap.net_busy {
                return;
            }
            snap.net_busy = true;
        }
        let shared = self.shared.clone();
        let previous = self.snapshot().network.clone();
        let a = action.clone();
        let ok = self.worker(
            "omnihw-net",
            move || {
                let r = crate::net_ops::apply(&a);
                // Re-read right away so the page never shows the state from before the change.
                // Wait (briefly) for a background read to finish so it can't overwrite this one.
                let mut owned = false;
                for _ in 0..100 {
                    if shared
                        .network_reading
                        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
                        .is_ok()
                    {
                        owned = true;
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                let fresh = NetworkInfo::gather(true, previous.as_ref());
                // Only release the flag if this thread took it (not another read's).
                if owned {
                    shared.network_reading.store(false, Ordering::Release);
                }
                (r, fresh)
            },
            move |(r, fresh), snap| {
                snap.network = Some(fresh);
                snap.net_busy = false;
                snap.net_msg = Some(match r {
                    Ok(()) if action.needs_restart() => {
                        format!("Done: {}. Restart Windows to finish.", action.label())
                    }
                    Ok(()) => format!("Done: {}", action.label()),
                    Err(e) => format!("{} failed: {e}", action.label()),
                });
            },
        );
        if !ok {
            self.snapshot().net_busy = false;
        }
    }

    /// Local network tests on every connected adapter: gateway ping and local DNS servers.
    pub fn run_local_tests(&self) {
        let adapters: Vec<_> = {
            let mut snap = self.snapshot();
            if snap.net_local.running {
                return;
            }
            let Some(n) = &snap.network else { return };
            let list: Vec<_> = n.connected().cloned().collect();
            snap.net_local.running = true;
            list
        };
        let ok = self.worker(
            "omnihw-net-local",
            move || {
                std::thread::scope(|s| {
                    let handles: Vec<_> = adapters
                        .iter()
                        .map(|a| s.spawn(move || net::local_results(a, &net::probe_local(a))))
                        .collect();
                    adapters
                        .iter()
                        .zip(handles)
                        .map(|(a, h)| (a.name.clone(), h.join().unwrap_or_default()))
                        .collect::<Vec<_>>()
                })
            },
            |r, snap| {
                snap.net_local = TestRun {
                    running: false,
                    results: Some((r, SystemTime::now())),
                };
            },
        );
        if !ok {
            self.snapshot().net_local.running = false;
        }
    }

    /// Internet tests (DNS, HTTP and pings to public servers). Only on the user's click.
    pub fn run_internet_tests(&self) {
        {
            let mut snap = self.snapshot();
            if snap.net_internet.running {
                return;
            }
            snap.net_internet.running = true;
        }
        let ok = self.worker(
            "omnihw-net-internet",
            || net::internet_results(&net::probe_internet()),
            |r, snap| {
                snap.net_internet = TestRun {
                    running: false,
                    results: Some((vec![(String::new(), r)], SystemTime::now())),
                };
            },
        );
        if !ok {
            self.snapshot().net_internet.running = false;
        }
    }

    /// Analyse the newest crash dumps (asks for administrator rights unless elevated). Only on
    /// the user's click.
    pub fn read_dumps(&self) {
        {
            let mut snap = self.snapshot();
            if snap.dumps_busy {
                return;
            }
            snap.dumps_busy = true;
            snap.dumps_rev += 1;
        }
        // Show the spinner now, not only after the (possibly long) UAC prompt.
        self.ctx.request_repaint();
        let ok = self.worker("omnihw-dumps", crate::dump_ops::read_dumps, |r, snap| {
            snap.dumps = Some(r);
            snap.dumps_busy = false;
            snap.dumps_rev += 1;
        });
        if !ok {
            let mut snap = self.snapshot();
            snap.dumps_busy = false;
            snap.dumps_rev += 1;
        }
    }

    /// Run the LAN Security Report: scan the connected adapters' private subnets. Only on the
    /// user's click. Progress is updated on the shared [`lansec::Progress`] as it runs.
    pub fn run_lan_scan(&self) {
        let (adapters, progress) = {
            let mut snap = self.snapshot();
            if snap.lan.running {
                return;
            }
            let Some(n) = &snap.network else { return };
            let adapters = n.adapters.clone();
            snap.lan = LanScanState {
                running: true,
                progress: Arc::new(lansec::Progress::default()),
                results: snap.lan.results.take(),
            };
            (adapters, snap.lan.progress.clone())
        };
        let ok = self.worker(
            "omnihw-lan-scan",
            move || lansec::scan(&adapters, &progress),
            |scan, snap| {
                snap.lan.running = false;
                snap.lan.results = Some((scan, SystemTime::now()));
            },
        );
        if !ok {
            self.snapshot().lan.running = false;
        }
    }

    /// Run the disk speed test on drive `drive_index` (Storage page). Only on the user's click
    /// and one drive at a time; progress is updated on the shared [`disktest::Progress`].
    pub fn run_disk_test(&self, drive_index: u32) {
        let Some(info) = self.info() else { return };
        let Some(drive) = info.drives.iter().find(|d| d.index == drive_index).cloned() else {
            return;
        };
        let link = disktest::nvme_link(&drive, &info.checkup.nvme_links);
        let expected = disktest::expectation(&drive, link);
        let progress = {
            let mut snap = self.snapshot();
            if snap.disk_test.running.is_some() {
                return;
            }
            snap.disk_test.running = Some(drive_index);
            snap.disk_test
                .whea
                .insert(drive_index, (SystemTime::now(), Vec::new()));
            snap.disk_test.progress = Arc::new(disktest::Progress::default());
            snap.disk_test.progress.clone()
        };
        let ok = self.worker(
            "omnihw-disk-test",
            move || disktest::run(&drive, expected, &progress),
            move |result, snap| {
                snap.disk_test.running = None;
                snap.disk_test
                    .results
                    .insert(drive_index, (result, SystemTime::now()));
            },
        );
        if !ok {
            self.snapshot().disk_test.running = None;
        }
    }

    /// Run the quick memory bandwidth/latency test. Only on the user's click; the test runs on
    /// its own worker thread and updates the shared [`memtest::Progress`].
    pub fn run_mem_test(&self) {
        let Some(info) = self.info() else { return };
        let progress = {
            let mut snap = self.snapshot();
            if snap.mem_test.running {
                return;
            }
            snap.mem_test = MemTestState {
                running: true,
                progress: Arc::new(memtest::Progress::default()),
                result: snap.mem_test.result.take(),
                started: Some(SystemTime::now()),
                whea_during: Vec::new(),
            };
            snap.mem_test.progress.clone()
        };
        let expected = info.smbios.as_ref().and_then(|s| {
            let dimms: Vec<_> = s.populated_memory().collect();
            memtest::expectation(&dimms, s.memory_slots)
        });
        // Size the buffer for what's free now, not at startup.
        let available = omnihw_core::win::memory_status()
            .or(info.memory)
            .map_or(0, |m| m.available_bytes);
        let ok = self.worker(
            "omnihw-mem-test",
            move || memtest::run(available, expected, &progress),
            |r, snap| {
                snap.mem_test.running = false;
                snap.mem_test.result = Some((r, SystemTime::now()));
            },
        );
        if !ok {
            self.snapshot().mem_test.running = false;
        }
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

/// Check the sensor service and add (or drop) its backend to match it and the setting. The
/// service backend sits right after the CPU counters so CPU temperatures lead with the CPU.
#[cfg(windows)]
fn refresh_sensor_service(shared: &Shared, backends: &mut Vec<Box<dyn SensorBackend>>) {
    use omnihw_core::sensor_service::ServiceState;
    use omnihw_core::win::pawnio::{ServiceBackend, service_status};
    backends.retain(|b| b.name() != SERVICE_BACKEND);
    let status = service_status();
    let on = shared.use_pawnio.load(Ordering::Relaxed)
        && matches!(status.state, ServiceState::Ready { .. });
    if on {
        backends.insert(backends.len().min(1), Box::new(ServiceBackend));
    }
    let mut snap = shared.snapshot.lock().unwrap_or_else(|e| e.into_inner());
    if !on {
        clear_service_readings(&mut snap);
    }
    snap.sensor_service = Some(status);
}

/// Drop the sensor service's readings so turned-off or lost sensors don't show frozen values.
fn clear_service_readings(snap: &mut Snapshot) {
    snap.table
        .retain(|t| !omnihw_core::sensor_service::is_service_reading(&t.reading.id));
}

/// The service stopped answering mid-session: drop its backend and readings and mark it not
/// running, so the periodic re-check brings it back when it returns.
#[cfg(windows)]
fn sensor_service_lost(shared: &Shared, backends: &mut Vec<Box<dyn SensorBackend>>, e: String) {
    use omnihw_core::sensor_service::ServiceState;
    backends.retain(|b| b.name() != SERVICE_BACKEND);
    let mut snap = shared.snapshot.lock().unwrap_or_else(|e| e.into_inner());
    clear_service_readings(&mut snap);
    if let Some(s) = &mut snap.sensor_service {
        s.state = ServiceState::NotRunning(e);
    }
}

#[cfg(not(windows))]
fn sensor_service_lost(_: &Shared, _: &mut Vec<Box<dyn SensorBackend>>, _: String) {}

#[cfg(not(windows))]
fn refresh_sensor_service(_: &Shared, _: &mut Vec<Box<dyn SensorBackend>>) {}

/// The sensor service is set up and wanted but wasn't answering at the last check.
fn service_pending(shared: &Shared) -> bool {
    use omnihw_core::sensor_service::ServiceState;
    shared.use_pawnio.load(Ordering::Relaxed)
        && matches!(
            shared
                .snapshot
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .sensor_service
                .as_ref()
                .map(|s| &s.state),
            Some(ServiceState::NotRunning(_))
        )
}

fn run(shared: Arc<Shared>, ctx: egui::Context, stop: mpsc::Receiver<()>) {
    let info = shared.info.get_or_init(SystemInfo::gather);
    ctx.request_repaint();

    let (mut backends, init_errors) = backends(info);
    // Past WHEA errors come from the startup query; this sees only new ones.
    #[cfg(windows)]
    let mut whea = omnihw_core::win::whea::WheaWatcher::new();
    refresh_sensor_service(&shared, &mut backends);
    let mut service_check = Instant::now();
    let mut readings = Vec::new();
    let started = Instant::now();
    let mut notifier: Option<Notifier> = None;
    let mut power_read: Option<Instant> = None;
    let mut network_read = NetworkReads::default();
    loop {
        let interval = Duration::from_millis(shared.interval_ms.load(Ordering::Relaxed));
        match stop.recv_timeout(interval) {
            Err(RecvTimeoutError::Timeout) => {}
            _ => return,
        }

        // Also re-check now and then while a set-up service isn't answering (e.g. still
        // starting after setup or at boot).
        if shared.service_refresh.swap(false, Ordering::Relaxed)
            || (service_check.elapsed() >= SERVICE_RECHECK && service_pending(&shared))
        {
            refresh_sensor_service(&shared, &mut backends);
            service_check = Instant::now();
        }
        readings.clear();
        let mut errors = init_errors.clone();
        let mut service_lost = None;
        for b in &mut backends {
            let before = readings.len();
            if let Err(e) = b.poll(&mut readings) {
                if b.name() == SERVICE_BACKEND && readings.len() == before {
                    service_lost = Some(e.clone());
                }
                errors.push(format!("{}: {e}", b.name()));
            }
        }
        if let Some(e) = service_lost {
            sensor_service_lost(&shared, &mut backends, e);
        }

        #[cfg(windows)]
        let new_whea = whea.as_mut().map(|w| w.poll()).unwrap_or_default();
        #[cfg(not(windows))]
        let new_whea: Vec<WheaEvent> = Vec::new();

        let mut snap = shared.snapshot.lock().unwrap_or_else(|e| e.into_inner());
        snap.table.update(readings.drain(..));
        if !new_whea.is_empty() {
            record_whea(&mut snap, new_whea);
        }
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
        update_network(&shared, &ctx, &mut network_read);
        ctx.request_repaint();
    }
}

/// Whether an event logged at `t` falls in a test run from `start` to `end` (None: running).
fn during(t: Option<SystemTime>, start: SystemTime, end: Option<SystemTime>) -> bool {
    // Without a time it can only have been seen now, so it counts while the test runs.
    let Some(t) = t else { return end.is_none() };
    t + WHEA_SLACK >= start && end.is_none_or(|e| t <= e + WHEA_SLACK)
}

/// Add newly logged WHEA events to the live list, and to a memory or disk test that was
/// running when they were logged (they can be read a moment after the test ends).
fn record_whea(snap: &mut Snapshot, events: Vec<WheaEvent>) {
    let mem = &mut snap.mem_test;
    if let Some(start) = mem.started {
        let end = if mem.running {
            None
        } else {
            mem.result.as_ref().map(|r| r.1)
        };
        if mem.running || end.is_some() {
            mem.whea_during.extend(
                events
                    .iter()
                    .filter(|e| during(e.time, start, end))
                    .cloned(),
            );
        }
    }
    let disk = &mut snap.disk_test;
    for (index, (start, list)) in &mut disk.whea {
        let end = if disk.running == Some(*index) {
            None
        } else {
            match disk.results.get(index) {
                Some((_, at)) => Some(*at),
                None => continue,
            }
        };
        list.extend(
            events
                .iter()
                .filter(|e| during(e.time, *start, end))
                .cloned(),
        );
    }
    snap.whea_live.extend(events);
    let excess = snap.whea_live.len().saturating_sub(WHEA_LIVE_LEN);
    snap.whea_live.drain(..excess);
    snap.whea_rev += 1;
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

/// When the network adapters were last read (quickly / with all details).
#[derive(Default)]
struct NetworkReads {
    fast: Option<Instant>,
    full: Option<Instant>,
}

/// Re-read network adapters outside the snapshot lock: addresses, link and Wi-Fi every few
/// seconds while the Network page is open (every minute otherwise), WMI details less often.
fn update_network(shared: &Arc<Shared>, ctx: &egui::Context, last: &mut NetworkReads) {
    if shared.network_reading.load(Ordering::Acquire) {
        return;
    }
    let open = shared.network_page.load(Ordering::Relaxed);
    let due = |t: Option<Instant>, every: Duration| t.is_none_or(|t| t.elapsed() >= every);
    let (fast_every, full_every) = if open {
        (NETWORK_EVERY_OPEN, NETWORK_DETAILS_EVERY_OPEN)
    } else {
        (NETWORK_EVERY_CLOSED, NETWORK_DETAILS_EVERY_CLOSED)
    };
    let full = shared.network_refresh.load(Ordering::Relaxed) || due(last.full, full_every);
    if !full && !due(last.fast, fast_every) {
        return;
    }
    if shared
        .network_reading
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
        .is_err()
    {
        return;
    }
    shared.network_refresh.store(false, Ordering::Relaxed);
    let now = Some(Instant::now());
    last.fast = now;
    if full {
        last.full = now;
    }
    let previous = shared
        .snapshot
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .network
        .clone();
    // The read can take hundreds of milliseconds (WMI), so it never runs on the sampler loop.
    let (s, c) = (shared.clone(), ctx.clone());
    let spawned = std::thread::Builder::new()
        .name("omnihw-net-read".into())
        .spawn(move || {
            let fresh = NetworkInfo::gather(full, previous.as_ref());
            s.snapshot.lock().unwrap_or_else(|e| e.into_inner()).network = Some(fresh);
            s.network_reading.store(false, Ordering::Release);
            c.request_repaint();
        });
    if spawned.is_err() {
        shared.network_reading.store(false, Ordering::Release);
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

#[cfg(all(test, windows))]
mod tests {
    use omnihw_core::sensors::SensorBackend;

    #[test]
    fn service_backend_name_matches() {
        assert_eq!(
            omnihw_core::win::pawnio::ServiceBackend.name(),
            super::SERVICE_BACKEND
        );
    }
}

#[cfg(test)]
mod whea_tests {
    use super::*;
    use omnihw_core::whea::WheaKind;

    fn event(time: SystemTime) -> WheaEvent {
        WheaEvent {
            record_id: 1,
            id: 19,
            time: Some(time),
            fatal: false,
            kind: WheaKind::Interconnect,
            source: String::new(),
            apic_id: Some(5),
            mca_bank: None,
            pcie: None,
            device: None,
            summary: String::new(),
        }
    }

    #[test]
    fn events_during_a_run() {
        let start = UNIX_EPOCH + Duration::from_secs(1_000);
        let end = start + Duration::from_secs(10);
        let at = |s: u64| Some(UNIX_EPOCH + Duration::from_secs(s));
        assert!(during(at(1_000), start, None));
        assert!(during(at(999), start, None));
        assert!(!during(at(990), start, None));
        assert!(during(at(1_010), start, Some(end)));
        assert!(!during(at(1_020), start, Some(end)));
        assert!(during(None, start, None));
        assert!(!during(None, start, Some(end)));
    }

    #[test]
    fn record_attributes_to_running_and_just_finished_tests() {
        let mut snap = Snapshot::default();
        let start = SystemTime::now() - Duration::from_secs(30);
        // Memory test running; drive 0 finished 10 s ago; drive 1 never finished.
        snap.mem_test.running = true;
        snap.mem_test.started = Some(start);
        snap.disk_test.whea.insert(0, (start, Vec::new()));
        snap.disk_test.results.insert(
            0,
            (Err("x".into()), SystemTime::now() - Duration::from_secs(10)),
        );
        snap.disk_test.whea.insert(1, (start, Vec::new()));
        // Logged 15 s ago (during both runs), then now (after the disk test ended).
        record_whea(
            &mut snap,
            vec![
                event(SystemTime::now() - Duration::from_secs(15)),
                event(SystemTime::now()),
            ],
        );
        assert_eq!(snap.whea_live.len(), 2);
        assert_eq!(snap.whea_rev, 1);
        assert_eq!(snap.mem_test.whea_during.len(), 2);
        assert_eq!(snap.disk_test.whea[&0].1.len(), 1);
        assert!(snap.disk_test.whea[&1].1.is_empty());

        // The live list is capped, oldest dropped.
        record_whea(&mut snap, vec![event(SystemTime::now()); WHEA_LIVE_LEN + 5]);
        assert_eq!(snap.whea_live.len(), WHEA_LIVE_LEN);
        assert_eq!(snap.whea_rev, 2);
    }
}
