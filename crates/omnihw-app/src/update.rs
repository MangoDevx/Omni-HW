//! Update check: Omni-HW's only network request, made only when the user clicks
//! "Check for updates" or has turned on the check at start (off by default).
//!
//! Asks GitHub's API for the repository's release list through `curl` and picks the newest
//! `linux-v*` tag (the Windows releases share the repository). Nothing but the URL is sent.
//! Runs on its own short-lived thread, never the UI thread.

use std::sync::{Arc, Mutex};

const REPO: &str = "MangoDevx/Omni-HW";
/// The repository's releases, filtered to the Linux ones.
pub const RELEASES_URL: &str =
    "https://github.com/MangoDevx/Omni-HW/releases?q=linux-v&expanded=true";

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

/// The newest `linux-v*` release tag, as "v<version>".
fn latest_tag() -> Result<String, String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases?per_page=100");
    let out = std::process::Command::new("curl")
        .args([
            "--silent",
            "--show-error",
            "--fail",
            "--proto",
            "=https",
            "--max-time",
            "10",
            "--header",
            "Accept: application/vnd.github+json",
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
    newest_linux_tag(&String::from_utf8_lossy(&out.stdout))
        .ok_or_else(|| "No Linux releases found on GitHub".into())
}

/// The highest-versioned `linux-v*` `tag_name` in a GitHub releases list (JSON), returned as
/// "v<version>". Only the tag names are needed, so they're scanned for directly.
fn newest_linux_tag(json: &str) -> Option<String> {
    let mut best: Option<(Vec<u64>, String)> = None;
    let mut rest = json;
    while let Some(i) = rest.find("\"tag_name\"") {
        rest = &rest[i + "\"tag_name\"".len()..];
        let value = rest.trim_start().strip_prefix(':')?.trim_start();
        let Some(value) = value.strip_prefix('"') else {
            continue;
        };
        let Some(end) = value.find('"') else {
            break;
        };
        let tag = &value[..end];
        if let Some(v) = tag.strip_prefix("linux-")
            && v.starts_with(['v', 'V'])
            && let Some(parts) = parse_version(v)
            && best.as_ref().is_none_or(|(b, _)| parts > *b)
        {
            best = Some((parts, v.to_string()));
        }
    }
    best.map(|(_, tag)| tag)
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
    fn linux_tags() {
        let json = r#"[{"tag_name": "v0.9.0", "name": "Omni-HW 0.9.0"},
            {"tag_name":"linux-v0.5.10","draft":false},
            {"tag_name": "linux-v0.5.2"}, {"tag_name": "linux-nightly"}]"#;
        assert_eq!(newest_linux_tag(json), Some("v0.5.10".into()));
        assert_eq!(newest_linux_tag(r#"[{"tag_name": "v0.9.0"}]"#), None);
        assert_eq!(newest_linux_tag("[]"), None);
        assert_eq!(newest_linux_tag(r#"{"tag_name": "#), None);
    }
}
