//! The single background thread: gathers static info once, then polls sensor
//! backends on an interval and asks the UI to repaint after each sample.

use crate::notify::Notifier;
use omnihw_core::SystemInfo;
use omnihw_core::alerts::{self, AlertEvent, AlertRule, Alerts};
use omnihw_core::disktest::{self, DiskTestResult};
use omnihw_core::eventlog::{EventLog, LogEntry};
use omnihw_core::lansec::{self, LanScan};
use omnihw_core::memtest::{self, MemTestResult};
use omnihw_core::network::{self as net, NetAction, NetworkInfo, TestLine};
use omnihw_core::recording::Recording;
use omnihw_core::sensors::{SensorBackend, SensorTable};
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const DEFAULT_INTERVAL_MS: u64 = 1000;
/// Fired alerts kept for the log.
const ALERT_LOG_LEN: usize = 200;
/// Journal times are read in whole seconds, so a run's start and end get this much slack.
const HW_SLACK: Duration = Duration::from_secs(1);
/// How often the kernel log is read for hardware errors while a memory or disk test runs.
const HW_POLL_EVERY: Duration = Duration::from_secs(5);
/// Network adapter re-reads (addresses, link, Wi-Fi) while the Network page is open / closed.
const NETWORK_EVERY_OPEN: Duration = Duration::from_secs(5);
const NETWORK_EVERY_CLOSED: Duration = Duration::from_secs(60);
/// Full re-reads including WMI details (driver, duplex, advanced properties).
const NETWORK_DETAILS_EVERY_OPEN: Duration = Duration::from_secs(30);
const NETWORK_DETAILS_EVERY_CLOSED: Duration = Duration::from_secs(300);

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
    /// Event Log page: the last read with the time it finished (`None` until first read).
    pub event_log: Option<(Arc<EventLog>, SystemTime)>,
    /// The event log is being read.
    pub event_log_busy: bool,
    /// Reading the root-only data (memory modules, drive health) through pkexec.
    pub root_read: RootRead,
}

/// State of the root-only read (see [`Sampler::read_as_root`]).
#[derive(Default, Clone)]
pub struct RootRead {
    pub busy: bool,
    /// The last read's outcome: how many items it added, or why it failed.
    pub result: Option<Result<usize, String>>,
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
    /// Per drive index: when its last run started and the hardware errors logged during it.
    pub hw_errors: HashMap<u32, (SystemTime, Vec<LogEntry>)>,
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
    /// Hardware errors logged during the last (or running) test.
    pub hw_errors: Vec<LogEntry>,
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
    /// Static info once gathered; replaced when the root-only data is merged in.
    info: RwLock<Option<Arc<SystemInfo>>>,
    snapshot: Mutex<Snapshot>,
    interval_ms: AtomicU64,
    reset: AtomicBool,
    test_notification: AtomicBool,
    /// Desktop notifications for fired alerts (they always show in-app).
    alert_notifications: AtomicBool,
    /// The Network page is open: read adapters more often.
    network_page: AtomicBool,
    /// Re-read network adapters with all details on the next sample.
    network_refresh: AtomicBool,
    /// A network read is running on its worker thread; the sampler schedules no second one.
    network_reading: AtomicBool,
}

/// Where alert rules are saved: `alerts.tsv` in the config folder.
fn alerts_path() -> Option<PathBuf> {
    Some(crate::settings::config_dir()?.join("alerts.tsv"))
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
    /// `root_at_start`: read the root-only data through pkexec once the info is gathered.
    pub fn start(ctx: egui::Context, root_at_start: bool) -> Self {
        let shared = Arc::new(Shared {
            info: RwLock::new(None),
            snapshot: Mutex::new(Snapshot {
                alerts: Alerts::new(load_alert_rules()),
                ..Default::default()
            }),
            interval_ms: AtomicU64::new(DEFAULT_INTERVAL_MS),
            reset: AtomicBool::new(false),
            test_notification: AtomicBool::new(false),
            alert_notifications: AtomicBool::new(true),
            network_page: AtomicBool::new(false),
            network_refresh: AtomicBool::new(false),
            network_reading: AtomicBool::new(false),
        });
        let (stop, stop_rx) = mpsc::channel::<()>();
        let s = shared.clone();
        let c = ctx.clone();
        std::thread::Builder::new()
            .name("omnihw-sampler".into())
            .spawn(move || run(s, c, stop_rx, root_at_start))
            .expect("spawn sampler thread");
        Sampler {
            shared,
            ctx,
            _stop: stop,
        }
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
                let r = net::apply(&a);
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
                .hw_errors
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
                hw_errors: Vec::new(),
            };
            snap.mem_test.progress.clone()
        };
        let expected = info.smbios.as_ref().and_then(|s| {
            let dimms: Vec<_> = s.populated_memory().collect();
            memtest::expectation(&dimms, s.memory_slots)
        });
        // Size the buffer for what's free now, not at startup.
        let available = omnihw_core::platform::memory_status()
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

    /// Read the journal for the Event Log page on a worker thread
    /// (when the page is first opened and on Refresh).
    pub fn read_event_log(&self) {
        {
            let mut snap = self.snapshot();
            if snap.event_log_busy {
                return;
            }
            snap.event_log_busy = true;
        }
        let ok = self.worker(
            "omnihw-event-log",
            omnihw_core::platform::events_viewer::read,
            |log, snap| {
                snap.event_log_busy = false;
                snap.event_log = Some((Arc::new(log), SystemTime::now()));
            },
        );
        if !ok {
            self.snapshot().event_log_busy = false;
        }
    }

    /// Static info, once gathered.
    pub fn info(&self) -> Option<Arc<SystemInfo>> {
        self.shared
            .info
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Read the root-only data (memory modules, drive health) through pkexec and merge it into
    /// the static info. Only on the user's click (or the "read at start" setting).
    pub fn read_as_root(&self) {
        {
            let mut snap = self.snapshot();
            if snap.root_read.busy || self.info().is_none() {
                return;
            }
            snap.root_read.busy = true;
        }
        let shared = self.shared.clone();
        let ok = self.worker(
            "omnihw-root",
            move || read_root_data(&shared),
            |r, snap| {
                snap.root_read = RootRead {
                    busy: false,
                    result: Some(r),
                };
            },
        );
        if !ok {
            self.snapshot().root_read.busy = false;
        }
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

    /// Turn desktop notifications (and the attention request) for fired alerts on or off.
    pub fn set_alert_notifications(&self, on: bool) {
        self.shared.alert_notifications.store(on, Ordering::Relaxed);
    }

    /// Show a sample notification on the next sample.
    pub fn test_notification(&self) {
        self.shared.test_notification.store(true, Ordering::Relaxed);
    }

    /// Replace the alert rules and save them. Returns an error message if saving failed.
    pub fn set_alert_rules(&self, rules: Vec<AlertRule>) -> Result<(), String> {
        let text = alerts::to_config(&rules);
        self.snapshot().alerts.set_rules(rules);
        let path = alerts_path().ok_or("No settings folder (APPDATA / HOME is not set)")?;
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

fn backends(info: &SystemInfo) -> Vec<Box<dyn SensorBackend>> {
    use omnihw_core::linux::Root;
    use omnihw_core::linux::sensors::{
        BatteryBackend, CpuBackend, DrmGpuBackend, HwmonBackend, MemoryBackend, StorageBackend,
    };
    let mut list: Vec<Box<dyn SensorBackend>> = Vec::new();
    list.push(Box::new(CpuBackend::new(
        Root::system(),
        info.topology.as_ref(),
    )));
    let hwmon = HwmonBackend::new(Root::system(), info.topology.as_ref());
    if !hwmon.is_empty() {
        list.push(Box::new(hwmon));
    }
    list.push(Box::new(MemoryBackend {
        root: Root::system(),
    }));
    if info.battery.is_some() {
        list.push(Box::new(BatteryBackend {
            root: Root::system(),
        }));
    }
    if !info.drives.is_empty() {
        list.push(Box::new(StorageBackend::new(Root::system(), &info.drives)));
    }
    // Vendor sensors first so they lead each GPU's group in the sensor list.
    if let Some(b) = omnihw_core::platform::nvml::NvmlBackend::new(&info.gpus) {
        list.push(Box::new(b));
    }
    let drm = DrmGpuBackend::new(Root::system(), &info.gpus);
    if !drm.is_empty() {
        list.push(Box::new(drm));
    }
    list
}

/// Run the root helper and merge its data into a copy of the info, which then replaces it.
/// Returns how many items were added.
fn read_root_data(shared: &Shared) -> Result<usize, String> {
    let text = crate::elevate::read_as_root()?;
    let names = omnihw_core::linux::block::drive_names(&omnihw_core::linux::Root::system());
    let mut slot = shared.info.write().unwrap_or_else(|e| e.into_inner());
    let Some(current) = slot.as_ref() else {
        return Err("Hardware info isn't ready yet".into());
    };
    let mut info = SystemInfo::clone(current);
    let n = omnihw_core::linux::rootdata::apply(&mut info, &names, &text);
    *slot = Some(Arc::new(info));
    Ok(n)
}

fn run(shared: Arc<Shared>, ctx: egui::Context, stop: mpsc::Receiver<()>, root_at_start: bool) {
    let info = Arc::new(SystemInfo::gather());
    *shared.info.write().unwrap_or_else(|e| e.into_inner()) = Some(info.clone());
    ctx.request_repaint();
    if root_at_start
        && !crate::elevate::elevated()
        && omnihw_core::linux::rootdata::would_add(&info)
    {
        let r = read_root_data(&shared);
        shared
            .snapshot
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .root_read
            .result = Some(r);
        ctx.request_repaint();
    }

    let mut backends = backends(&info);
    let mut readings = Vec::new();
    let started = Instant::now();
    let mut notifier = Notifier::default();
    let mut hw_watch = HwWatch::default();
    let mut network_read = NetworkReads::default();
    loop {
        let interval = Duration::from_millis(shared.interval_ms.load(Ordering::Relaxed));
        match stop.recv_timeout(interval) {
            Err(RecvTimeoutError::Timeout) => {}
            _ => return,
        }

        readings.clear();
        let mut errors = Vec::new();
        for b in &mut backends {
            if let Err(e) = b.poll(&mut readings) {
                errors.push(format!("{}: {e}", b.name()));
            }
        }

        let new_hw = hw_watch.poll(&shared);

        let mut snap = shared.snapshot.lock().unwrap_or_else(|e| e.into_inner());
        snap.table.update(readings.drain(..));
        if !new_hw.is_empty() {
            record_hw_errors(&mut snap, &new_hw);
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
        let notify_alerts = shared.alert_notifications.load(Ordering::Relaxed);
        for e in fired.iter().filter(|_| notify_alerts) {
            notifier.show(&ctx, "Omni-HW alert", &e.message());
        }
        if shared.test_notification.swap(false, Ordering::Relaxed) {
            notifier.show(
                &ctx,
                "Omni-HW",
                "Test notification: alerts will appear like this.",
            );
        }
        update_network(&shared, &ctx, &mut network_read);
        ctx.request_repaint();
    }
}

/// Whether an event logged at `t` falls in a test run from `start` to `end` (None: running).
fn during(t: SystemTime, start: SystemTime, end: Option<SystemTime>) -> bool {
    t + HW_SLACK >= start && end.is_none_or(|e| t <= e + HW_SLACK)
}

fn logged_at(e: &LogEntry) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(e.unix.max(0) as u64)
}

/// Add newly logged hardware errors to a memory or disk test that was running when they were
/// logged (they can be read a moment after the test ends).
fn record_hw_errors(snap: &mut Snapshot, events: &[LogEntry]) {
    let mem = &mut snap.mem_test;
    if let Some(start) = mem.started {
        let end = if mem.running {
            None
        } else {
            mem.result.as_ref().map(|r| r.1)
        };
        if mem.running || end.is_some() {
            mem.hw_errors.extend(
                events
                    .iter()
                    .filter(|e| during(logged_at(e), start, end))
                    .cloned(),
            );
        }
    }
    let disk = &mut snap.disk_test;
    for (index, (start, list)) in &mut disk.hw_errors {
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
                .filter(|e| during(logged_at(e), *start, end))
                .cloned(),
        );
    }
}

/// Reads the kernel's hardware errors while a memory or disk test runs (and briefly after, as
/// errors can be read a moment after they're logged).
#[derive(Default)]
struct HwWatch {
    last: Option<Instant>,
    /// Read errors logged at or after this Unix second.
    since: i64,
    /// Errors already seen that were logged in second `since`.
    seen: Vec<LogEntry>,
}

impl HwWatch {
    fn poll(&mut self, shared: &Shared) -> Vec<LogEntry> {
        let active = {
            let snap = shared.snapshot.lock().unwrap_or_else(|e| e.into_inner());
            let recent = |t: SystemTime| t.elapsed().is_ok_and(|d| d < HW_POLL_EVERY * 2);
            snap.mem_test.running
                || snap.disk_test.running.is_some()
                || snap.mem_test.result.as_ref().is_some_and(|r| recent(r.1))
                || snap.disk_test.results.values().any(|r| recent(r.1))
        };
        if !active {
            self.last = None;
            return Vec::new();
        }
        if self.last.is_some_and(|t| t.elapsed() < HW_POLL_EVERY) {
            return Vec::new();
        }
        if self.last.is_none() {
            // A test just started: look back far enough to cover its first moments.
            self.since = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_secs() as i64)
                - HW_POLL_EVERY.as_secs() as i64;
            self.seen.clear();
        }
        self.last = Some(Instant::now());
        let fresh: Vec<LogEntry> =
            omnihw_core::platform::events_viewer::hardware_errors_since(self.since)
                .into_iter()
                .filter(|e| !self.seen.contains(e))
                .collect();
        if let Some(newest) = fresh.iter().map(|e| e.unix).max() {
            if newest > self.since {
                self.since = newest;
                self.seen.clear();
            }
            let since = self.since;
            self.seen
                .extend(fresh.iter().filter(|e| e.unix == since).cloned());
        }
        fresh
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

#[cfg(test)]
mod hw_error_tests {
    use super::*;

    fn event(time: SystemTime) -> LogEntry {
        LogEntry {
            unix: time.duration_since(UNIX_EPOCH).unwrap().as_secs() as i64,
            utc_offset: None,
            source: "kernel".into(),
            level: omnihw_core::eventlog::EventLevel::Warning,
            category: omnihw_core::eventlog::EventCategory::HardwareError,
            title: "Corrected memory error",
            detail: String::new(),
        }
    }

    #[test]
    fn events_during_a_run() {
        let start = UNIX_EPOCH + Duration::from_secs(1_000);
        let end = start + Duration::from_secs(10);
        let at = |s: u64| UNIX_EPOCH + Duration::from_secs(s);
        assert!(during(at(1_000), start, None));
        assert!(during(at(999), start, None));
        assert!(!during(at(990), start, None));
        assert!(during(at(1_010), start, Some(end)));
        assert!(!during(at(1_020), start, Some(end)));
    }

    #[test]
    fn record_attributes_to_running_and_just_finished_tests() {
        let mut snap = Snapshot::default();
        let start = SystemTime::now() - Duration::from_secs(30);
        // Memory test running; drive 0 finished 10 s ago; drive 1 never finished.
        snap.mem_test.running = true;
        snap.mem_test.started = Some(start);
        snap.disk_test.hw_errors.insert(0, (start, Vec::new()));
        snap.disk_test.results.insert(
            0,
            (Err("x".into()), SystemTime::now() - Duration::from_secs(10)),
        );
        snap.disk_test.hw_errors.insert(1, (start, Vec::new()));
        // Logged 15 s ago (during both runs), then now (after the disk test ended).
        record_hw_errors(
            &mut snap,
            &[
                event(SystemTime::now() - Duration::from_secs(15)),
                event(SystemTime::now()),
            ],
        );
        assert_eq!(snap.mem_test.hw_errors.len(), 2);
        assert_eq!(snap.disk_test.hw_errors[&0].1.len(), 1);
        assert!(snap.disk_test.hw_errors[&1].1.is_empty());
    }
}
