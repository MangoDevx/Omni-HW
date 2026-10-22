//! Update check: Omni-HW's only network request, made only when the user clicks
//! "Check for updates" or has turned on the check at start (off by default).
//!
//! Sends one HEAD request to `github.com/<repo>/releases/latest` through `curl`, without following
//! the redirect, and reads the release tag from its target, so nothing but the URL is sent and no
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

fn latest_tag() -> Result<String, String> {
    let url = format!("https://github.com/{REPO}/releases/latest");
    let out = std::process::Command::new("curl")
        .args([
            "--silent",
            "--show-error",
            "--head",
            // HTTPS only, no redirects followed: the tag is read from the redirect target.
            "--proto",
            "=https",
            "--max-time",
            "10",
            "--output",
            "/dev/null",
            "--write-out",
            "%{http_code} %{redirect_url}",
            "--user-agent",
            concat!("Omni-HW/", env!("CARGO_PKG_VERSION")),
            &url,
        ])
        .output()
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => {
                "The update check needs curl; install it with your package manager".to_string()
            }
            _ => format!("Could not run curl ({e})"),
        })?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let err = err.trim().trim_start_matches("curl: ");
        return Err(format!("Could not reach GitHub ({err})"));
    }
    parse_curl_output(&String::from_utf8_lossy(&out.stdout))
}

/// The tag from curl's `--write-out "%{http_code} %{redirect_url}"`.
fn parse_curl_output(out: &str) -> Result<String, String> {
    let (code, location) = out.trim().split_once(' ').unwrap_or((out.trim(), ""));
    let code: u32 = code.parse().unwrap_or(0);
    if !(300..400).contains(&code) {
        return Err(if code == 404 {
            "No releases found on GitHub".into()
        } else {
            format!("GitHub answered with HTTP {code}")
        });
    }
    tag_from_location(location).ok_or_else(|| "No releases found on GitHub".into())
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
    fn curl_output() {
        assert_eq!(
            parse_curl_output("302 https://github.com/MangoDevx/Omni-HW/releases/tag/v0.5.0"),
            Ok("v0.5.0".into())
        );
        assert_eq!(
            parse_curl_output("404 "),
            Err("No releases found on GitHub".into())
        );
        assert_eq!(
            parse_curl_output("200 "),
            Err("GitHub answered with HTTP 200".into())
        );
        assert!(parse_curl_output("").is_err());
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
