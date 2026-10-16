//! Applying power changes, with administrator rights when needed. Changes that need them run
//! in a short-lived elevated copy of Omni-HW (`--power-action`), so the app itself never has
//! to restart elevated; reading power requests works the same way (`--power-query`). The helper
//! does its one job and exits without opening a window. All of this blocks, so it runs on a
//! worker thread (see `Sampler::power_action`).

use omnihw_core::power::{self, Action, ApplyError, PowerRequest};
use std::path::{Path, PathBuf};

const ACTION_FLAG: &str = "--power-action";
const QUERY_FLAG: &str = "--power-query";
/// Result files live in %TEMP% with this prefix; the helper refuses any other path.
const FILE_PREFIX: &str = "omnihw-power-";
/// Separates the two powercfg outputs in a query result file.
const SEPARATOR: &str = "\n<<<omnihw-waketimers>>>\n";

/// If this process was started as the elevated helper, do the job and return the exit code.
pub fn run_helper() -> Option<i32> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [flag, arg, out] if flag == ACTION_FLAG => {
            let Some(out) = result_path(out) else {
                return Some(2);
            };
            let (code, text) = match Action::from_arg(arg) {
                Some(a) => match power::apply(&a) {
                    Ok(()) => (0, String::new()),
                    Err(e) => (1, e.to_string()),
                },
                None => (1, "Unknown power action".into()),
            };
            let _ = std::fs::write(out, text);
            Some(code)
        }
        [flag, out] if flag == QUERY_FLAG => {
            let Some(out) = result_path(out) else {
                return Some(2);
            };
            match power::query_admin_raw() {
                Some((r, t)) => {
                    let _ = std::fs::write(out, format!("{r}{SEPARATOR}{t}"));
                    Some(0)
                }
                None => Some(1),
            }
        }
        _ => None,
    }
}

/// Accept only `omnihw-power-*.txt` in a temp folder, so the elevated helper can't be used to
/// write anywhere else. (With over-the-shoulder elevation the helper runs as another account, so
/// its own %TEMP% may differ from the caller's.)
fn result_path(p: &str) -> Option<PathBuf> {
    let p = PathBuf::from(p);
    let in_temp = p.parent().is_some_and(|d| {
        d == std::env::temp_dir().as_path()
            || d.file_name()
                .is_some_and(|n| n.eq_ignore_ascii_case("Temp"))
    });
    let ok = in_temp
        && p.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with(FILE_PREFIX) && n.ends_with(".txt"));
    ok.then_some(p)
}

fn temp_file(kind: &str) -> PathBuf {
    std::env::temp_dir().join(format!("{FILE_PREFIX}{kind}-{}.txt", std::process::id()))
}

fn quote(p: &Path) -> String {
    format!("\"{}\"", p.display())
}

/// Apply a change, asking for administrator rights only when needed. Blocks.
pub fn apply(action: &Action) -> Result<(), String> {
    let elevated = crate::elevate::elevated();
    if !action.needs_admin() || elevated {
        match power::apply(action) {
            Ok(()) => return Ok(()),
            Err(ApplyError::AccessDenied) if !elevated => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    let out = temp_file("action");
    let code = crate::elevate::run_as_admin_wait(&format!(
        "{ACTION_FLAG} {} {}",
        action.to_arg(),
        quote(&out)
    ));
    let text = std::fs::read_to_string(&out).unwrap_or_default();
    let _ = std::fs::remove_file(&out);
    match code? {
        0 => Ok(()),
        _ if !text.is_empty() => Err(text),
        c => Err(format!("the elevated helper failed (code {c})")),
    }
}

/// Read active power requests and wake timers, elevating if needed. Blocks.
pub fn query_admin() -> Result<(Vec<PowerRequest>, Vec<String>), String> {
    let raw = if crate::elevate::elevated() {
        power::query_admin_raw().ok_or("powercfg failed")?
    } else {
        let out = temp_file("query");
        let code = crate::elevate::run_as_admin_wait(&format!("{QUERY_FLAG} {}", quote(&out)));
        let text = std::fs::read_to_string(&out).unwrap_or_default();
        let _ = std::fs::remove_file(&out);
        if code? != 0 {
            return Err("powercfg failed".into());
        }
        let (r, t) = text.split_once(SEPARATOR).unwrap_or((&text, ""));
        (r.to_string(), t.to_string())
    };
    Ok((
        power::parse_requests(&raw.0),
        power::parse_wake_timers(&raw.1),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helper_writes_only_to_temp() {
        assert!(result_path(&temp_file("action").to_string_lossy()).is_some());
        assert!(result_path(r"C:\Windows\System32\omnihw-power-x.txt").is_none());
        let other = std::env::temp_dir().join("evil.txt");
        assert!(result_path(&other.to_string_lossy()).is_none());
        let other = std::env::temp_dir().join("omnihw-power-x.dll");
        assert!(result_path(&other.to_string_lossy()).is_none());
    }
}
