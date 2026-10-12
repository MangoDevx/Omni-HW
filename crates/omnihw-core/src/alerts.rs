//! User alert rules: "sensor above/below a threshold for N seconds" -> fire once, re-arm
//! when the condition clears. Pure logic; the app decides how to notify.

use crate::sensors::{SensorTable, Unit};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Condition {
    Above,
    Below,
}

impl Condition {
    pub fn symbol(self) -> &'static str {
        match self {
            Condition::Above => ">",
            Condition::Below => "<",
        }
    }

    fn met(self, value: f64, threshold: f64) -> bool {
        match self {
            Condition::Above => value > threshold,
            Condition::Below => value < threshold,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AlertRule {
    pub enabled: bool,
    pub sensor_id: String,
    /// Display name of the sensor, e.g. "GPU: NVIDIA GeForce RTX 3090 / GPU Temperature".
    pub sensor_name: String,
    pub unit: Unit,
    pub condition: Condition,
    pub threshold: f64,
    /// How long the condition must hold before firing.
    pub duration_secs: f64,
}

impl AlertRule {
    /// "GPU Temperature > 85 °C for 30 s"
    pub fn describe(&self) -> String {
        let name = self
            .sensor_name
            .rsplit(" / ")
            .next()
            .unwrap_or(&self.sensor_name);
        let dur = if self.duration_secs > 0.0 {
            format!(" for {:.0} s", self.duration_secs)
        } else {
            String::new()
        };
        format!(
            "{name} {} {}{dur}",
            self.condition.symbol(),
            self.unit.format(self.threshold)
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AlertEvent {
    /// Unix seconds when it fired.
    pub at_unix: f64,
    pub rule: String,
    pub value: f64,
    pub unit: Unit,
}

impl AlertEvent {
    pub fn message(&self) -> String {
        format!("{} (now {})", self.rule, self.unit.format(self.value))
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct RuleState {
    /// When the condition started holding (monotonic seconds), if it holds now.
    since: Option<f64>,
    /// Fired and not yet re-armed.
    fired: bool,
}

/// Rules plus their runtime state.
#[derive(Debug, Clone, Default)]
pub struct Alerts {
    rules: Vec<AlertRule>,
    state: Vec<RuleState>,
}

impl Alerts {
    pub fn new(rules: Vec<AlertRule>) -> Alerts {
        let state = vec![RuleState::default(); rules.len()];
        Alerts { rules, state }
    }

    pub fn rules(&self) -> &[AlertRule] {
        &self.rules
    }

    /// Replace the rules (e.g. after editing). Runtime state is kept for unchanged rules.
    pub fn set_rules(&mut self, rules: Vec<AlertRule>) {
        let state = rules
            .iter()
            .map(|r| {
                self.rules
                    .iter()
                    .position(|o| o == r)
                    .map(|i| self.state[i])
                    .unwrap_or_default()
            })
            .collect();
        self.rules = rules;
        self.state = state;
    }

    /// Whether rule `i` is currently firing (condition held long enough and not cleared).
    pub fn is_active(&self, i: usize) -> bool {
        self.state.get(i).is_some_and(|s| s.fired)
    }

    /// Evaluate against the latest readings. `now` is monotonic seconds; `now_unix` stamps events.
    /// Returns newly fired events.
    pub fn evaluate(&mut self, now: f64, now_unix: f64, table: &SensorTable) -> Vec<AlertEvent> {
        let mut out = Vec::new();
        for (rule, st) in self.rules.iter().zip(self.state.iter_mut()) {
            let value = table.get(&rule.sensor_id).map(|t| t.reading.value);
            let met = rule.enabled && value.is_some_and(|v| rule.condition.met(v, rule.threshold));
            if !met {
                *st = RuleState::default();
                continue;
            }
            let since = *st.since.get_or_insert(now);
            if !st.fired && now - since >= rule.duration_secs {
                st.fired = true;
                out.push(AlertEvent {
                    at_unix: now_unix,
                    rule: rule.describe(),
                    value: value.unwrap_or(0.0),
                    unit: rule.unit,
                });
            }
        }
        out
    }
}

fn unit_code(u: Unit) -> &'static str {
    match u {
        Unit::Mhz => "mhz",
        Unit::Percent => "pct",
        Unit::Celsius => "c",
        Unit::Watt => "w",
        Unit::Volt => "v",
        Unit::Rpm => "rpm",
        Unit::Megabytes => "mb",
        Unit::MegabytesPerSec => "mbps",
        Unit::None => "none",
    }
}

fn unit_from_code(s: &str) -> Option<Unit> {
    Some(match s {
        "mhz" => Unit::Mhz,
        "pct" => Unit::Percent,
        "c" => Unit::Celsius,
        "w" => Unit::Watt,
        "v" => Unit::Volt,
        "rpm" => Unit::Rpm,
        "mb" => Unit::Megabytes,
        "mbps" => Unit::MegabytesPerSec,
        "none" => Unit::None,
        _ => return None,
    })
}

/// Serialize rules, one per line: `enabled\tid\tabove|below\tthreshold\tduration\tunit\tname`.
pub fn to_config(rules: &[AlertRule]) -> String {
    let mut s = String::from("# Omni-HW alert rules\n");
    for r in rules {
        let cond = match r.condition {
            Condition::Above => "above",
            Condition::Below => "below",
        };
        s.push_str(&format!(
            "{}\t{}\t{cond}\t{}\t{}\t{}\t{}\n",
            u8::from(r.enabled),
            r.sensor_id,
            r.threshold,
            r.duration_secs,
            unit_code(r.unit),
            r.sensor_name.replace(['\t', '\n'], " ")
        ));
    }
    s
}

/// Parse rules written by [`to_config`]; malformed lines are skipped.
pub fn from_config(text: &str) -> Vec<AlertRule> {
    // Editors such as Notepad may prepend a UTF-8 byte-order mark.
    text.trim_start_matches('\u{feff}')
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            if f.len() != 7 {
                return None;
            }
            Some(AlertRule {
                enabled: f[0] == "1",
                sensor_id: f[1].to_string(),
                condition: match f[2] {
                    "above" => Condition::Above,
                    "below" => Condition::Below,
                    _ => return None,
                },
                threshold: f[3].parse().ok()?,
                duration_secs: f[4].parse().ok()?,
                unit: unit_from_code(f[5])?,
                sensor_name: f[6].to_string(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sensors::Reading;

    fn rule(cond: Condition, threshold: f64, dur: f64) -> AlertRule {
        AlertRule {
            enabled: true,
            sensor_id: "gpu/1/temp".into(),
            sensor_name: "GPU: RTX / GPU Temperature".into(),
            unit: Unit::Celsius,
            condition: cond,
            threshold,
            duration_secs: dur,
        }
    }

    fn table(v: f64) -> SensorTable {
        let mut t = SensorTable::default();
        t.update([Reading {
            id: "gpu/1/temp".into(),
            label: "GPU Temperature".into(),
            group: "GPU: RTX".into(),
            unit: Unit::Celsius,
            value: v,
        }]);
        t
    }

    #[test]
    fn fires_after_duration_once_and_rearms() {
        let mut a = Alerts::new(vec![rule(Condition::Above, 85.0, 10.0)]);
        assert!(a.evaluate(0.0, 0.0, &table(90.0)).is_empty());
        assert!(a.evaluate(5.0, 5.0, &table(90.0)).is_empty());
        let e = a.evaluate(10.0, 1000.0, &table(91.0));
        assert_eq!(e.len(), 1);
        assert_eq!(
            e[0].message(),
            "GPU Temperature > 85.0 °C for 10 s (now 91.0 °C)"
        );
        assert!(a.is_active(0));
        assert!(
            a.evaluate(20.0, 0.0, &table(92.0)).is_empty(),
            "fires once while it holds"
        );
        assert!(a.evaluate(21.0, 0.0, &table(70.0)).is_empty());
        assert!(!a.is_active(0), "re-armed when the condition clears");
        assert!(
            a.evaluate(22.0, 0.0, &table(90.0)).is_empty(),
            "duration restarts"
        );
        assert_eq!(a.evaluate(32.0, 0.0, &table(90.0)).len(), 1);
    }

    #[test]
    fn below_disabled_and_missing() {
        let mut a = Alerts::new(vec![rule(Condition::Below, 20.0, 0.0)]);
        assert_eq!(a.evaluate(0.0, 0.0, &table(10.0)).len(), 1);
        let mut r = rule(Condition::Above, 1.0, 0.0);
        r.enabled = false;
        let mut a = Alerts::new(vec![r]);
        assert!(a.evaluate(0.0, 0.0, &table(50.0)).is_empty());
        let mut a = Alerts::new(vec![rule(Condition::Above, 1.0, 0.0)]);
        assert!(a.evaluate(0.0, 0.0, &SensorTable::default()).is_empty());
    }

    #[test]
    fn set_rules_keeps_state_of_unchanged_rules() {
        let r1 = rule(Condition::Above, 85.0, 0.0);
        let mut a = Alerts::new(vec![r1.clone()]);
        a.evaluate(0.0, 0.0, &table(90.0));
        assert!(a.is_active(0));
        let r2 = rule(Condition::Below, 10.0, 0.0);
        a.set_rules(vec![r2, r1]);
        assert!(!a.is_active(0) && a.is_active(1));
    }

    #[test]
    fn config_roundtrip() {
        let mut r = rule(Condition::Below, 12.5, 30.0);
        r.enabled = false;
        let rules = vec![rule(Condition::Above, 85.0, 30.0), r];
        let text = to_config(&rules);
        assert_eq!(from_config(&text), rules);
        assert_eq!(from_config(&format!("\u{feff}{text}")), rules);
        assert!(from_config("garbage\n1\tx\tsideways\t1\t1\tc\tn\n").is_empty());
    }
}
