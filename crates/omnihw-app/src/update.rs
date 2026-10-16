//! Update check: Omni-HW's only network request, made only when the user clicks
//! "Check for updates" or has turned on the check at start (off by default).
//!
//! Sends one HEAD request to `github.com/<repo>/releases/latest` with redirects disabled and
//! reads the release tag from the `Location` header, so nothing but the URL is sent and no
//! response body is downloaded. Runs on its own short-lived thread, never the UI thread.

use std::sync::{Arc, Mutex};

const REPO: &str = "MangoDevx/Omni-HW";
pub const RELEASES_URL: &str = "https://github.com/MangoDevx/Omni-HW/releases/latest";

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Status {
    #[default]
    Idle,
    Checking,
    UpToDate,
    /// A newer release is on GitHub (its version, without the leading `v`).
    Available(String),
    /// This build is newer than the latest release (a development build).
    AheadOfRelease(String),
    Failed(String),
}

#[derive(Clone, Default)]
pub struct UpdateChecker {
    status: Arc<Mutex<Status>>,
}

impl UpdateChecker {
    /// The status, even if a check thread panicked while holding the lock.
    fn lock(&self) -> std::sync::MutexGuard<'_, Status> {
        self.status.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn status(&self) -> Status {
        self.lock().clone()
    }

    /// Start a check in the background unless one is already running.
    pub fn start(&self, ctx: &egui::Context) {
        {
            let mut s = self.lock();
            if *s == Status::Checking {
                return;
            }
            *s = Status::Checking;
        }
        let checker = self.clone();
        let ctx = ctx.clone();
        std::thread::Builder::new()
            .name("update-check".into())
            .spawn(move || {
                let result = match latest_tag() {
                    Ok(tag) => compare(env!("CARGO_PKG_VERSION"), &tag),
                    Err(e) => Status::Failed(e),
                };
                *checker.lock() = result;
                ctx.request_repaint();
            })
            .expect("spawn update-check thread");
    }
}

/// Parse "v1.2.3" / "1.2.3-beta" into numeric parts; the pre-release suffix is ignored.
fn parse_version(v: &str) -> Option<Vec<u64>> {
    let v = v.trim().trim_start_matches(['v', 'V']);
    let core = v.split(['-', '+']).next()?;
    core.split('.').map(|p| p.parse().ok()).collect()
}

fn compare(current: &str, latest_tag: &str) -> Status {
    let latest = latest_tag.trim().trim_start_matches(['v', 'V']).to_string();
    let (Some(mut cur), Some(mut new)) = (parse_version(current), parse_version(&latest)) else {
        return Status::Failed(format!("Unrecognized release tag \"{latest_tag}\""));
    };
    let len = cur.len().max(new.len());
    cur.resize(len, 0);
    new.resize(len, 0);
    match new.cmp(&cur) {
        std::cmp::Ordering::Greater => Status::Available(latest),
        std::cmp::Ordering::Equal => Status::UpToDate,
        std::cmp::Ordering::Less => Status::AheadOfRelease(latest),
    }
}

/// The tag in a `.../releases/tag/<tag>` redirect target.
fn tag_from_location(location: &str) -> Option<String> {
    let (_, tag) = location.split_once("/releases/tag/")?;
    let tag = tag.split(['?', '#', '/']).next()?.trim();
    (!tag.is_empty()).then(|| tag.to_string())
}

#[cfg(windows)]
fn latest_tag() -> Result<String, String> {
    use windows::Win32::Networking::WinHttp::*;
    use windows::core::{HSTRING, PCWSTR, w};

    /// Closes a WinHTTP handle on drop.
    struct Handle(*mut core::ffi::c_void);
    impl Drop for Handle {
        fn drop(&mut self) {
            // SAFETY: a valid handle from WinHttpOpen/Connect/OpenRequest, closed once.
            unsafe {
                let _ = WinHttpCloseHandle(self.0);
            }
        }
    }
    fn handle(h: *mut core::ffi::c_void) -> Result<Handle, String> {
        if h.is_null() {
            Err(format!(
                "Could not reach GitHub ({})",
                windows::core::Error::from_thread()
            ))
        } else {
            Ok(Handle(h))
        }
    }
    let net = |e: windows::core::Error| format!("Could not reach GitHub ({e})");

    let agent = HSTRING::from(format!("Omni-HW/{}", env!("CARGO_PKG_VERSION")));
    let path = HSTRING::from(format!("/{REPO}/releases/latest"));
    // SAFETY: all strings are NUL-terminated and outlive the calls; handles are checked
    // for null and closed by `Handle` in reverse order of creation.
    unsafe {
        let session = handle(WinHttpOpen(
            &agent,
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            PCWSTR::null(),
            PCWSTR::null(),
            0,
        ))?;
        WinHttpSetTimeouts(session.0, 10_000, 10_000, 10_000, 10_000).map_err(net)?;
        let connect = handle(WinHttpConnect(
            session.0,
            w!("github.com"),
            INTERNET_DEFAULT_HTTPS_PORT,
            0,
        ))?;
        let request = handle(WinHttpOpenRequest(
            connect.0,
            w!("HEAD"),
            &path,
            PCWSTR::null(),
            PCWSTR::null(),
            std::ptr::null(),
            WINHTTP_FLAG_SECURE,
        ))?;
        // Read the redirect target ourselves instead of following it.
        WinHttpSetOption(
            Some(request.0),
            WINHTTP_OPTION_DISABLE_FEATURE,
            Some(&WINHTTP_DISABLE_REDIRECTS.to_ne_bytes()),
        )
        .map_err(net)?;
        WinHttpSendRequest(request.0, None, None, 0, 0, 0).map_err(net)?;
        WinHttpReceiveResponse(request.0, std::ptr::null_mut()).map_err(net)?;

        let mut code = 0u32;
        let mut len = size_of::<u32>() as u32;
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some((&raw mut code).cast()),
            &mut len,
            std::ptr::null_mut(),
        )
        .map_err(net)?;
        if !(300..400).contains(&code) {
            return Err(if code == 404 {
                "No releases found on GitHub".into()
            } else {
                format!("GitHub answered with HTTP {code}")
            });
        }

        let mut buf = [0u16; 1024];
        let mut len = size_of_val(&buf) as u32;
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_LOCATION,
            PCWSTR::null(),
            Some(buf.as_mut_ptr().cast()),
            &mut len,
            std::ptr::null_mut(),
        )
        .map_err(net)?;
        let location = String::from_utf16_lossy(&buf[..len as usize / 2]);
        tag_from_location(&location).ok_or_else(|| "No releases found on GitHub".into())
    }
}

#[cfg(not(windows))]
fn latest_tag() -> Result<String, String> {
    Err("Update checks are only supported on Windows".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert_eq!(compare("0.1.0", "v0.1.0"), Status::UpToDate);
        assert_eq!(
            compare("0.1.0", "v0.2.0"),
            Status::Available("0.2.0".into())
        );
        assert_eq!(
            compare("0.1.9", "v0.1.10"),
            Status::Available("0.1.10".into())
        );
        assert_eq!(compare("0.1", "0.1.0"), Status::UpToDate);
        assert_eq!(
            compare("0.2.0", "v0.1.0"),
            Status::AheadOfRelease("0.1.0".into())
        );
        assert_eq!(compare("0.1.0", "v0.1.0-beta"), Status::UpToDate);
        assert!(matches!(compare("0.1.0", "nightly"), Status::Failed(_)));
    }

    #[test]
    fn location_tag() {
        assert_eq!(
            tag_from_location("https://github.com/MangoDevx/Omni-HW/releases/tag/v0.1.0"),
            Some("v0.1.0".into())
        );
        assert_eq!(
            tag_from_location("https://github.com/MangoDevx/Omni-HW/releases"),
            None
        );
    }
}
