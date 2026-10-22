//! Health Check findings the user dismissed. Shared by the Health tab, the Overview banner and
//! Settings; the app copies changes into [`crate::settings::Settings`] so they can persist.

use omnihw_core::diagnostics::{CheckResult, Finding, Outcome};
use std::collections::BTreeSet;
use std::sync::Mutex;

struct Store {
    keys: BTreeSet<String>,
    /// Changed since the app last copied the keys into its settings.
    dirty: bool,
    /// Bumped on every change, so cached Health Check results know to re-split.
    revision: u64,
}

static STORE: Mutex<Store> = Mutex::new(Store {
    keys: BTreeSet::new(),
    dirty: false,
    revision: 0,
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

impl Store {
    fn changed(&mut self, changed: bool) {
        if changed {
            self.dirty = true;
            self.revision += 1;
        }
    }
}

/// Seed the store at startup (from saved settings).
pub fn load(keys: &[String]) {
    with(|s| {
        s.keys = keys.iter().cloned().collect();
        s.revision += 1;
    });
}

pub fn dismiss(key: String) {
    with(|s| {
        let changed = s.keys.insert(key);
        s.changed(changed);
    });
}

pub fn restore(key: &str) {
    with(|s| {
        let changed = s.keys.remove(key);
        s.changed(changed);
    });
}

pub fn restore_all() {
    with(|s| {
        let changed = !s.keys.is_empty();
        s.keys.clear();
        s.changed(changed);
    });
}

/// Changes whenever the set of dismissed findings does.
pub fn revision() -> u64 {
    with(|s| s.revision)
}

pub fn count() -> usize {
    with(|s| s.keys.len())
}

/// The current keys if they changed since the last call.
pub fn take_changes() -> Option<Vec<String>> {
    with(|s| std::mem::take(&mut s.dirty).then(|| s.keys.iter().cloned().collect()))
}

/// Split results into what to show and what was dismissed. Dismissed findings are moved out of
/// their check (which keeps an empty `Findings` outcome, so it isn't counted as passed).
pub fn split(mut results: Vec<CheckResult>) -> (Vec<CheckResult>, Vec<(&'static str, Finding)>) {
    let mut hidden = Vec::new();
    with(|s| {
        if s.keys.is_empty() {
            return;
        }
        for r in &mut results {
            let name = r.name;
            if let Outcome::Findings(f) = &mut r.outcome {
                let (gone, kept): (Vec<Finding>, Vec<Finding>) = std::mem::take(f)
                    .into_iter()
                    .partition(|x| s.keys.contains(&key(name, x)));
                *f = kept;
                hidden.extend(gone.into_iter().map(|x| (name, x)));
            }
        }
    });
    (results, hidden)
}

#[cfg(test)]
mod tests {
    use super::*;
    use omnihw_core::diagnostics::{Category, Level};

    fn finding(level: Level, title: &str) -> Finding {
        Finding {
            level,
            category: Category::System,
            title: title.into(),
            detail: String::new(),
            steps: Vec::new(),
            security: false,
            bios: false,
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

    #[test]
    fn split_moves_dismissed_findings_out() {
        let results = vec![CheckResult {
            name: "Uptime",
            category: Category::System,
            outcome: Outcome::Findings(vec![
                finding(Level::Warning, "Windows has been running for 40 days"),
                finding(Level::Info, "Something else"),
            ]),
        }];
        let before = revision();
        load(&[key(
            "Uptime",
            &finding(Level::Warning, "Windows has been running for 9 days"),
        )]);
        assert_ne!(revision(), before);
        let (visible, hidden) = split(results);
        assert_eq!(hidden.len(), 1);
        assert_eq!(hidden[0].0, "Uptime");
        assert!(
            matches!(&visible[0].outcome, Outcome::Findings(f) if f.len() == 1 && f[0].title == "Something else")
        );
        restore_all();
        assert_eq!(count(), 0);
        assert_eq!(take_changes(), Some(Vec::new()));
    }
}
