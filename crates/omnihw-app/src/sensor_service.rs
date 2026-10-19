//! `omnihw-sensors.exe --sensor-service`: the Omni-HW sensor service, started by the Windows
//! service manager (see `omnihw_core::sensor_service` for the design).
//!
//! It loads the PawnIO modules, and for every client that connects to its pipe it writes the
//! current readings and closes the connection. It never reads from clients, and polls the hardware at most
//! every [`MIN_POLL`] however often it is asked, so callers can't steer or hammer it.

use omnihw_core::sensor_service::SERVICE_NAME;

pub const SERVICE_FLAG: &str = "--sensor-service";

/// If this process was started by the service manager, run the service and return its exit
/// code.
pub fn run_if_service() -> Option<i32> {
    let mut args = std::env::args().skip(1);
    (args.next().as_deref() == Some(SERVICE_FLAG) && args.next().is_none()).then(imp::run)
}

#[cfg(windows)]
mod imp {
    use super::SERVICE_NAME;
    use omnihw_core::lowlevel::PawnioState;
    use omnihw_core::sensor_service::{PROTOCOL, Response, encode};
    use omnihw_core::sensors::SensorBackend;
    use omnihw_core::topology::Topology;
    use omnihw_core::win::pawnio::{self, CpuMsrBackend, SuperIoBackend};
    use std::ffi::OsString;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};
    use windows_service::service::{
        ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus,
        ServiceType,
    };
    use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
    use windows_service::{define_windows_service, service_dispatcher};

    /// Minimum time between hardware polls; requests in between get the cached response.
    const MIN_POLL: Duration = Duration::from_millis(250);
    /// How long to wait before retrying when PawnIO wasn't usable (e.g. still starting at boot).
    const RETRY_SETUP: Duration = Duration::from_secs(30);

    define_windows_service!(ffi_service_main, service_main);

    pub(super) fn run() -> i32 {
        match service_dispatcher::start(SERVICE_NAME, ffi_service_main) {
            Ok(()) => 0,
            Err(_) => 1,
        }
    }

    fn status(state: ServiceState, code: u32) -> ServiceStatus {
        ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state: state,
            controls_accepted: if state == ServiceState::Running {
                ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN
            } else {
                ServiceControlAccept::empty()
            },
            exit_code: ServiceExitCode::Win32(code),
            checkpoint: 0,
            wait_hint: Duration::from_secs(5),
            process_id: None,
        }
    }

    fn service_main(_args: Vec<OsString>) {
        let stop = Arc::new(AtomicBool::new(false));
        let on_stop = stop.clone();
        let handler = move |control| match control {
            ServiceControl::Stop | ServiceControl::Shutdown => {
                on_stop.store(true, Ordering::Relaxed);
                pawnio::wake();
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        };
        let Ok(handle) = service_control_handler::register(SERVICE_NAME, handler) else {
            return;
        };
        let _ = handle.set_service_status(status(ServiceState::Running, 0));
        let mut sensors = Sensors::default();
        let code = match pawnio::serve(&stop, || sensors.respond()) {
            Ok(()) => 0,
            Err(_) => 1,
        };
        let _ = handle.set_service_status(status(ServiceState::Stopped, code));
    }

    #[derive(Default)]
    struct Sensors {
        backends: Vec<Box<dyn SensorBackend>>,
        sources: Vec<String>,
        setup_errors: Vec<String>,
        set_up_at: Option<Instant>,
        cache: Option<(Instant, String)>,
    }

    impl Sensors {
        /// Load the PawnIO backends, retrying now and then while none work.
        fn set_up(&mut self) {
            if !self.backends.is_empty()
                || self.set_up_at.is_some_and(|t| t.elapsed() < RETRY_SETUP)
            {
                return;
            }
            self.set_up_at = Some(Instant::now());
            self.sources.clear();
            self.setup_errors.clear();
            let state = pawnio::status().state;
            if state != PawnioState::Ready {
                self.setup_errors.push(match state {
                    PawnioState::NotInstalled => "PawnIO isn't installed".into(),
                    PawnioState::NotRunning(e) => format!("PawnIO isn't running ({e})"),
                    _ => "PawnIO can't be opened".into(),
                });
                return;
            }
            let cpu = omnihw_core::cpuid::decode(&omnihw_core::cpuid::NativeCpuid);
            let topology =
                omnihw_core::win::topology_buffer().and_then(|b| Topology::parse(&b).ok());
            match CpuMsrBackend::new(&cpu, topology.as_ref()) {
                Ok(b) => {
                    self.backends.push(Box::new(b));
                    self.sources.push("CPU temperatures and voltage".into());
                }
                Err(e) => self.setup_errors.push(format!("CPU sensors: {e}")),
            }
            match SuperIoBackend::new() {
                Ok(b) => {
                    self.sources.extend(b.chips());
                    self.backends.push(Box::new(b));
                }
                Err(e) => self.setup_errors.push(format!("Motherboard sensors: {e}")),
            }
        }

        fn respond(&mut self) -> String {
            if let Some((at, text)) = &self.cache
                && at.elapsed() < MIN_POLL
            {
                return text.clone();
            }
            self.set_up();
            let mut r = Response {
                protocol: PROTOCOL,
                version: env!("CARGO_PKG_VERSION").into(),
                sources: self.sources.clone(),
                errors: self.setup_errors.clone(),
                readings: Vec::new(),
            };
            for b in &mut self.backends {
                if let Err(e) = b.poll(&mut r.readings) {
                    r.errors.push(format!("{}: {e}", b.name()));
                }
            }
            let text = encode(&r);
            self.cache = Some((Instant::now(), text.clone()));
            text
        }
    }
}

#[cfg(not(windows))]
mod imp {
    pub(super) fn run() -> i32 {
        1
    }
}
