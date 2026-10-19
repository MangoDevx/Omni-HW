//! Extra sensors through PawnIO: setting them up or removing them when the user asks (Settings
//! page), plus the licence texts and source links that come with bundling the driver.
//!
//! Setup runs one elevated helper (`--setup-sensors`) that installs PawnIO silently from the
//! embedded, unmodified official installer if it isn't installed yet, copies this exe to
//! `Program Files\Omni-HW\omnihw-sensors.exe` (a folder only administrators can change, since it
//! runs as SYSTEM) and registers it as the sensor service. Omni-HW itself never opens PawnIO; it
//! reads the service's read-only pipe. Nothing here runs unless the user clicks.

use crate::power_ops::{helper_result_path, quote};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

pub const SITE_URL: &str = "https://pawnio.eu";
pub const DRIVER_SOURCE_URL: &str = "https://github.com/namazso/PawnIO/tree/2.2.0";
pub const SETUP_SOURCE_URL: &str = "https://github.com/namazso/PawnIO.Setup/tree/2.2.0";
pub const MODULES_SOURCE_URL: &str = "https://github.com/namazso/PawnIO.Modules/tree/0.2.11";
/// GPL 2 text covering the PawnIO driver.
pub const DRIVER_LICENSE: &str = include_str!("../pawnio/COPYING");
/// LGPL 2.1 text covering the PawnIO modules.
pub const MODULES_LICENSE: &str = include_str!("../../omnihw-core/pawnio/COPYING");

const SETUP_FLAG: &str = "--setup-sensors";
const REMOVE_FLAG: &str = "--remove-sensors";
/// Helper result files: `%TEMP%\omnihw-sensors-<pid>.txt`.
const FILE_PREFIX: &str = "omnihw-sensors-";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Job {
    #[default]
    Idle,
    /// "Setting up" / "Removing".
    Running(&'static str),
    Done(Result<String, String>),
}

#[derive(Default)]
struct State {
    job: Job,
    /// A job finished since the page last asked (it then re-reads the service status).
    finished: Option<bool>,
}

#[derive(Clone, Default)]
pub struct Installer {
    state: Arc<Mutex<State>>,
}

impl Installer {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn job(&self) -> Job {
        self.lock().job.clone()
    }

    /// `Some(true)` once after a successful setup, `Some(false)` after any other finished job.
    pub fn take_finished(&self) -> Option<bool> {
        self.lock().finished.take()
    }

    /// Run setup (or removal) in the background unless a job is already running.
    pub fn start(&self, ctx: &egui::Context, setup: bool) {
        {
            let mut s = self.lock();
            if matches!(s.job, Job::Running(_)) {
                return;
            }
            s.job = Job::Running(if setup { "Setting up" } else { "Removing" });
        }
        let me = self.clone();
        let ctx = ctx.clone();
        std::thread::Builder::new()
            .name("sensor-setup".into())
            .spawn(move || {
                let result = run_elevated(setup);
                let mut s = me.lock();
                s.finished = Some(setup && result.is_ok());
                s.job = Job::Done(result);
                drop(s);
                ctx.request_repaint();
            })
            .expect("spawn sensor-setup thread");
    }
}

fn temp_file() -> PathBuf {
    std::env::temp_dir().join(format!("{FILE_PREFIX}{}.txt", std::process::id()))
}

/// Run this exe elevated as the setup/remove helper and collect its message.
fn run_elevated(setup: bool) -> Result<String, String> {
    let out = temp_file();
    let flag = if setup { SETUP_FLAG } else { REMOVE_FLAG };
    let code = crate::elevate::run_as_admin_wait(&format!("{flag} {}", quote(&out)));
    let text = std::fs::read_to_string(&out).unwrap_or_default();
    let _ = std::fs::remove_file(&out);
    match code? {
        0 => Ok(text),
        _ if !text.is_empty() => Err(text),
        c => Err(format!("the elevated helper failed (code {c})")),
    }
}

/// If this process was started as the elevated setup/remove helper, do the job and return the
/// exit code.
pub fn run_helper() -> Option<i32> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [flag, out] = args.as_slice() else {
        return None;
    };
    let setup = match flag.as_str() {
        SETUP_FLAG => true,
        REMOVE_FLAG => false,
        _ => return None,
    };
    let Some(out) = helper_result_path(out, FILE_PREFIX) else {
        return Some(2);
    };
    let result = if setup { imp::setup() } else { imp::remove() };
    let (code, text) = match result {
        Ok(msg) => (0, msg),
        Err(e) => (1, e),
    };
    let _ = std::fs::write(out, text);
    Some(code)
}

/// Split a registry command line (`"C:\x\uninstall.exe" -uninstall`) into program and arguments.
#[cfg_attr(not(windows), allow(dead_code))]
fn split_command(cmd: &str) -> Option<(PathBuf, String)> {
    let cmd = cmd.trim();
    let (exe, rest) = match cmd.strip_prefix('"') {
        Some(quoted) => quoted.split_once('"')?,
        None => cmd.split_once(' ').unwrap_or((cmd, "")),
    };
    (!exe.is_empty()).then(|| (exe.into(), rest.trim().to_string()))
}

#[cfg(windows)]
mod imp {
    use crate::sensor_service::SERVICE_FLAG;
    use omnihw_core::sensor_service::{SERVICE_DISPLAY_NAME, SERVICE_NAME};
    use omnihw_core::win::pawnio;
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::time::{Duration, Instant};
    use windows_service::service::{
        Service, ServiceAccess, ServiceDependency, ServiceErrorControl, ServiceInfo,
        ServiceStartType, ServiceState, ServiceType,
    };
    use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};

    const SETUP: &[u8] = include_bytes!("../pawnio/PawnIO_setup.exe");
    const SERVICE_EXE: &str = "omnihw-sensors.exe";
    /// Present in the install folder when Omni-HW installed PawnIO (so removal uninstalls it
    /// again; a PawnIO another app installed is left alone).
    const PAWNIO_MARKER: &str = "installed-pawnio";
    /// `ERROR_SUCCESS_REBOOT_REQUIRED`.
    const REBOOT_REQUIRED: i32 = 3010;
    const ACCESS: ServiceAccess = ServiceAccess::QUERY_STATUS
        .union(ServiceAccess::START)
        .union(ServiceAccess::STOP)
        .union(ServiceAccess::CHANGE_CONFIG)
        .union(ServiceAccess::DELETE);

    /// `C:\Program Files\Omni-HW`, from the shell's known folder (not an environment variable,
    /// which the caller could have changed).
    fn install_dir() -> Result<PathBuf, String> {
        use windows::Win32::System::Com::CoTaskMemFree;
        use windows::Win32::UI::Shell::{
            FOLDERID_ProgramFiles, KF_FLAG_DEFAULT, SHGetKnownFolderPath,
        };
        // SAFETY: the returned string is CoTaskMemAlloc'd; it is copied, then freed once.
        let pf = unsafe {
            let p = SHGetKnownFolderPath(&FOLDERID_ProgramFiles, KF_FLAG_DEFAULT, None)
                .map_err(|e| format!("Program Files folder not found: {e}"))?;
            let s = p.to_string();
            CoTaskMemFree(Some(p.0 as *const _));
            s.map_err(|_| "Program Files path is not valid text".to_string())?
        };
        Ok(PathBuf::from(pf).join("Omni-HW"))
    }

    fn err(what: &str) -> impl Fn(windows_service::Error) -> String + '_ {
        move |e| format!("{what}: {e}")
    }

    fn manager() -> Result<ServiceManager, String> {
        ServiceManager::local_computer(
            None::<&str>,
            ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
        )
        .map_err(err("Couldn't open the service manager"))
    }

    fn stop_and_wait(s: &Service) {
        if s.query_status()
            .is_ok_and(|st| st.current_state != ServiceState::Stopped)
        {
            let _ = s.stop();
        }
        let until = Instant::now() + Duration::from_secs(10);
        while Instant::now() < until
            && s.query_status()
                .is_ok_and(|st| st.current_state != ServiceState::Stopped)
        {
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    /// Retry a file operation for a few seconds (the old service process may still be exiting).
    fn retry(mut f: impl FnMut() -> std::io::Result<()>) -> std::io::Result<()> {
        let until = Instant::now() + Duration::from_secs(5);
        loop {
            match f() {
                Err(_) if Instant::now() < until => std::thread::sleep(Duration::from_millis(250)),
                r => return r,
            }
        }
    }

    fn run_quiet(exe: &Path, args: &[&str]) -> Result<i32, String> {
        Command::new(exe)
            .args(args)
            .status()
            .map(|s| s.code().unwrap_or(1))
            .map_err(|e| format!("Couldn't run {}: {e}", exe.display()))
    }

    fn install_pawnio() -> Result<bool, String> {
        let dir = std::env::temp_dir().join("Omni-HW");
        let path = dir.join("PawnIO_setup.exe");
        std::fs::create_dir_all(&dir)
            .and_then(|_| std::fs::write(&path, SETUP))
            .map_err(|e| format!("Couldn't write the PawnIO installer: {e}"))?;
        let code = run_quiet(&path, &["-install", "-silent"]);
        let _ = std::fs::remove_file(&path);
        match code? {
            0 => Ok(false),
            REBOOT_REQUIRED => Ok(true),
            c => Err(format!("The PawnIO installer stopped with code {c}.")),
        }
    }

    pub(super) fn setup() -> Result<String, String> {
        let mut reboot = false;
        let mut installed_pawnio = false;
        if !pawnio::installed() {
            reboot = install_pawnio()?;
            installed_pawnio = true;
        }
        let manager = manager()?;
        let existing = manager.open_service(SERVICE_NAME, ACCESS).ok();
        if let Some(s) = &existing {
            stop_and_wait(s);
        }
        let dir = install_dir()?;
        let exe = dir.join(SERVICE_EXE);
        let me = std::env::current_exe().map_err(|e| e.to_string())?;
        std::fs::create_dir_all(&dir)
            .and_then(|_| retry(|| std::fs::copy(&me, &exe).map(drop)))
            .map_err(|e| format!("Couldn't copy Omni-HW to {}: {e}", dir.display()))?;
        if installed_pawnio {
            let _ = std::fs::write(dir.join(PAWNIO_MARKER), "");
        }
        let info = ServiceInfo {
            name: SERVICE_NAME.into(),
            display_name: SERVICE_DISPLAY_NAME.into(),
            service_type: ServiceType::OWN_PROCESS,
            start_type: ServiceStartType::AutoStart,
            error_control: ServiceErrorControl::Normal,
            executable_path: exe,
            launch_arguments: vec![OsString::from(SERVICE_FLAG)],
            dependencies: vec![ServiceDependency::Service("PawnIO".into())],
            account_name: None,
            account_password: None,
        };
        let service = match existing {
            Some(s) => {
                s.change_config(&info)
                    .map_err(err("Couldn't update the sensor service"))?;
                s
            }
            None => manager
                .create_service(&info, ACCESS)
                .map_err(err("Couldn't create the sensor service"))?,
        };
        let _ = service.set_description(
            "Reads CPU and motherboard sensors through the PawnIO driver for Omni-HW. Read-only.",
        );
        if reboot {
            return Ok("Set up. Restart Windows to finish installing PawnIO.".into());
        }
        service
            .start::<&str>(&[])
            .map_err(err("Couldn't start the sensor service"))?;
        Ok("Extra sensors are set up.".into())
    }

    pub(super) fn remove() -> Result<String, String> {
        let manager = manager()?;
        if let Ok(s) = manager.open_service(SERVICE_NAME, ACCESS) {
            stop_and_wait(&s);
            s.delete()
                .map_err(err("Couldn't remove the sensor service"))?;
        }
        let dir = install_dir()?;
        let ours = dir.join(PAWNIO_MARKER).exists();
        let _ = retry(|| std::fs::remove_file(dir.join(SERVICE_EXE)));
        let _ = std::fs::remove_file(dir.join(PAWNIO_MARKER));
        let _ = std::fs::remove_dir(&dir);
        if !ours || !pawnio::installed() {
            return Ok(if ours {
                "Extra sensors removed.".into()
            } else {
                "Extra sensors removed. PawnIO was left installed because Omni-HW didn't install it.".into()
            });
        }
        let (exe, args) = pawnio::uninstall_command()
            .and_then(|c| super::split_command(&c))
            .filter(|(exe, _)| exe.is_file())
            .ok_or("Removed the sensor service, but couldn't find PawnIO's uninstaller.")?;
        let mut args: Vec<&str> = args.split_whitespace().collect();
        args.push("-silent");
        match run_quiet(&exe, &args)? {
            0 => Ok("Extra sensors and PawnIO removed.".into()),
            REBOOT_REQUIRED => Ok("Removed. Restart Windows to finish removing PawnIO.".into()),
            c => Err(format!("The PawnIO uninstaller stopped with code {c}.")),
        }
    }
}

#[cfg(not(windows))]
mod imp {
    pub(super) fn setup() -> Result<String, String> {
        Err("Windows only".into())
    }
    pub(super) fn remove() -> Result<String, String> {
        Err("Windows only".into())
    }
}

#[cfg(test)]
mod tests {
    use super::split_command;

    #[test]
    fn uninstall_command_parsing() {
        assert_eq!(
            split_command(r#""C:\Program Files\PawnIO\uninstall.exe" -uninstall"#),
            Some((
                r"C:\Program Files\PawnIO\uninstall.exe".into(),
                "-uninstall".into()
            ))
        );
        assert_eq!(
            split_command(r"C:\PawnIO\uninstall.exe -uninstall -silent"),
            Some((
                r"C:\PawnIO\uninstall.exe".into(),
                "-uninstall -silent".into()
            ))
        );
        assert_eq!(split_command(r#""unterminated"#), None);
    }
}
