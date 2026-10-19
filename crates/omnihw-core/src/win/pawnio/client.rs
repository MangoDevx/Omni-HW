//! The app side of the sensor service: an unprivileged [`SensorBackend`] that reads the
//! service's pipe, and the service's status for the Settings page.

use super::pipe;
use crate::sensor_service::{
    PROTOCOL, Response, SERVICE_NAME, ServiceState, ServiceStatus, decode,
};
use crate::sensors::{Reading, SensorBackend};
use crate::win::registry::{self, Hive};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_millis(500);

fn service_key() -> String {
    format!(r"SYSTEM\CurrentControlSet\Services\{SERVICE_NAME}")
}

/// Whether the sensor service is registered with Windows (registry only).
pub fn service_installed() -> bool {
    registry::key_exists(Hive::LocalMachine, &service_key())
}

fn fetch() -> Result<Response, String> {
    decode(&pipe::fetch(TIMEOUT)?)
}

/// Driver and service state. Talks to the pipe once, so call it off the UI thread.
pub fn service_status() -> ServiceStatus {
    let state = if !service_installed() {
        ServiceState::NotSetUp
    } else {
        match fetch() {
            Ok(r) if r.protocol != PROTOCOL => ServiceState::Outdated { version: r.version },
            Ok(r) => ServiceState::Ready {
                version: r.version,
                sources: r.sources,
                errors: r.errors,
            },
            Err(e) => ServiceState::NotRunning(e),
        }
    };
    ServiceStatus {
        state,
        pawnio_version: super::version(),
    }
}

/// Readings from the sensor service, fetched on every poll.
pub struct ServiceBackend;

impl SensorBackend for ServiceBackend {
    fn name(&self) -> &str {
        "Sensor service"
    }

    fn poll(&mut self, out: &mut Vec<Reading>) -> Result<(), String> {
        let r = fetch()?;
        if r.protocol != PROTOCOL {
            return Err(format!(
                "the sensor service is from Omni-HW {}; update it in Settings",
                r.version
            ));
        }
        out.extend(r.readings);
        if r.errors.is_empty() {
            Ok(())
        } else {
            Err(r.errors.join("; "))
        }
    }
}
