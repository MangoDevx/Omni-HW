//! Report export: save HTML to Documents and open it, or produce text for the clipboard.

use crate::sampler::Snapshot;
use omnihw_core::{SystemInfo, report};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// Report sections, with the Health Check results first.
fn sections(info: &SystemInfo, snap: &Snapshot) -> Vec<report::Section> {
    let results = crate::tabs::health::run(info, snap);
    let mut rows: Vec<(String, String)> = omnihw_core::diagnostics::findings(&results)
        .iter()
        .flat_map(|f| report::finding_rows(f))
        .collect();
    if rows.is_empty() {
        rows.push(("Result".into(), "No problems found".into()));
    }
    let mut out = vec![report::Section {
        title: "Health Check".into(),
        rows,
    }];
    out.extend(report::sections(info, omnihw_core::platform::uptime_secs()));
    if let Some(p) = &snap.power {
        out.push(omnihw_core::power::report_section(p));
    }
    out
}

pub fn text(info: &SystemInfo, snap: &Snapshot) -> String {
    report::to_text(&sections(info, snap), snap.table.entries())
}

/// UTC timestamp "YYYYMMDD-HHMMSS" without a date library.
pub fn timestamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) as i64;
    let (days, rem) = (secs.div_euclid(86400), secs.rem_euclid(86400));
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}{m:02}{d:02}-{:02}{:02}{:02}",
        rem / 3600,
        rem / 60 % 60,
        rem % 60
    )
}

/// User-chosen folder for recordings and reports (Settings > Files); `None` = Documents.
static DATA_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

pub fn set_data_dir(dir: Option<PathBuf>) {
    *DATA_DIR.lock().unwrap_or_else(|e| e.into_inner()) = dir;
}

/// Where recordings and reports go: the chosen folder if it exists, else Documents, else temp.
pub fn report_dir() -> PathBuf {
    if let Some(d) = DATA_DIR.lock().unwrap_or_else(|e| e.into_inner()).clone()
        && d.is_dir()
    {
        return d;
    }
    let docs = std::env::var_os("HOME").map(|p| PathBuf::from(p).join("Documents"));
    match docs {
        Some(d) if d.is_dir() => d,
        _ => std::env::temp_dir(),
    }
}

/// Write the HTML report and open it in the default browser. Returns the path.
pub fn save_html(info: &SystemInfo, snap: &Snapshot) -> Result<PathBuf, String> {
    let html = report::to_html(&sections(info, snap), snap.table.entries());
    let path = report_dir().join(format!("OmniHW-Report-{}.html", timestamp()));
    std::fs::write(&path, html).map_err(|e| format!("Could not write {}: {e}", path.display()))?;
    open_in_explorer(&path, false);
    Ok(path)
}

/// Open `path` with `xdg-open` (a folder in the file manager, or a file in its default app).
/// With `select`, open the containing folder instead (xdg-open can't select a file). Doesn't
/// wait for it.
pub fn open_in_explorer(path: &std::path::Path, select: bool) {
    let target = match (select, path.parent()) {
        (true, Some(dir)) => dir,
        _ => path,
    };
    let _ = std::process::Command::new("xdg-open").arg(target).spawn();
}

#[cfg(test)]
mod tests {
    #[test]
    fn timestamp_shape() {
        let t = super::timestamp();
        assert_eq!(t.len(), 15);
        assert!(t.starts_with("20"));
        assert_eq!(&t[8..9], "-");
    }
}
