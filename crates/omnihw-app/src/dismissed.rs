//! Health Check findings the user dismissed. Shared by the Health tab, the Overview banner and
//! Settings; the app copies changes into [`crate::settings::Settings`] so they can persist.

use omnihw_core::diagnostics::{CheckResult, Finding, Outcome};
use std::collections::BTreeSet;
use std::sync::Mutex;

struct Store {
    keys: BTreeSet<String>,
    /// Changed since the app last copied the keys into its settings.
    dirty: bool,
}

static STORE: Mutex<Store> = Mutex::new(Store {
    keys: BTreeSet::new(),
    dirty: false,
});

fn with<R>(f: impl FnOnce(&mut Store) -> R) -> R {
    f(&mut STORE.lock().unwrap_or_else(|e| e.into_inner()))
}

/// Stable key for a finding: check name, level and title with digits removed, so a title like
/// "Windows has been running for 9 days" stays dismissed as the number changes, while a finding
/// that gets more severe shows up again.
pub fn key(check: &str, f: &Finding) -> String {
    let title: String = f.title.chars().filter(|c| !c.is_ascii_digit()).collect();
    format!("{check}|{}|{title}", f.level.name())
}

/// Seed the store at startup (from saved settings).
pub fn load(keys: &[String]) {
    with(|s| s.keys = keys.iter().cloned().collect());
}

pub fn dismiss(key: String) {
    with(|s| s.dirty |= s.keys.insert(key));
}

pub fn restore(key: &str) {
    with(|s| s.dirty |= s.keys.remove(key));
}

pub fn restore_all() {
    with(|s| {
        s.dirty |= !s.keys.is_empty();
        s.keys.clear();
    });
}

pub fn count() -> usize {
    with(|s| s.keys.len())
}

/// The current keys if they changed since the last call.
pub fn take_changes() -> Option<Vec<String>> {
    with(|s| std::mem::take(&mut s.dirty).then(|| s.keys.iter().cloned().collect()))
}

/// Split results into what to show and what was dismissed. Dismissed findings are removed from
/// their check (which keeps an empty `Findings` outcome, so it isn't counted as passed).
pub fn split(results: &[CheckResult]) -> (Vec<CheckResult>, Vec<(&'static str, Finding)>) {
    let mut hidden = Vec::new();
    let visible = with(|s| {
        results
            .iter()
            .map(|r| {
                let mut r = r.clone();
                let name = r.name;
                if let Outcome::Findings(f) = &mut r.outcome {
                    f.retain(|x| {
                        let gone = s.keys.contains(&key(name, x));
                        if gone {
                            hidden.push((name, x.clone()));
                        }
                        !gone
                    });
                }
                r
            })
            .collect()
    });
    (visible, hidden)
}

#[cfg(test)]
mod tests {
    use super::key;
    use omnihw_core::diagnostics::{Category, Finding, Level};

    fn finding(level: Level, title: &str) -> Finding {
        Finding {
            level,
            category: Category::System,
            title: title.into(),
            detail: String::new(),
            steps: Vec::new(),
            security: false,
            bios: false,
            fix: None,
        }
    }

    #[test]
    fn key_ignores_numbers_but_not_severity() {
        let a = key(
            "Uptime",
            &finding(Level::Info, "Windows has been running for 9 days"),
        );
        let b = key(
            "Uptime",
            &finding(Level::Info, "Windows has been running for 12 days"),
        );
        let c = key(
            "Uptime",
            &finding(Level::Warning, "Windows has been running for 40 days"),
        );
        assert_eq!(a, b);
        assert_ne!(a, c);
    }
}
