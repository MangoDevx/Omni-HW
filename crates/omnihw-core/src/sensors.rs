//! Sensor abstraction: backends produce [`Reading`]s; [`SensorTable`] tracks min/max/avg.

use std::collections::{HashMap, VecDeque};

/// Samples kept per sensor for sparklines.
pub const HISTORY_LEN: usize = 120;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Mhz,
    Percent,
    Celsius,
    Watt,
    Volt,
    Rpm,
    Megabytes,
    MegabytesPerSec,
    /// Dimensionless value, e.g. a P-state number.
    None,
}

impl Unit {
    pub fn symbol(self) -> &'static str {
        match self {
            Unit::Mhz => "MHz",
            Unit::Percent => "%",
            Unit::Celsius => "°C",
            Unit::Watt => "W",
            Unit::Volt => "V",
            Unit::Rpm => "RPM",
            Unit::Megabytes => "MB",
            Unit::MegabytesPerSec => "MB/s",
            Unit::None => "",
        }
    }

    pub fn format(self, v: f64) -> String {
        match self {
            Unit::Mhz | Unit::Rpm | Unit::Megabytes => format!("{v:.0} {}", self.symbol()),
            Unit::None => format!("{v:.0}"),
            Unit::MegabytesPerSec if v < 10.0 => format!("{v:.2} {}", self.symbol()),
            Unit::Volt => format!("{v:.3} {}", self.symbol()),
            _ => format!("{v:.1} {}", self.symbol()),
        }
    }
}

/// One instantaneous sensor value.
#[derive(Debug, Clone, PartialEq)]
pub struct Reading {
    /// Stable identifier, unique across all backends (e.g. "cpu/clock/3").
    pub id: String,
    pub label: String,
    /// Display group, e.g. "CPU Clocks".
    pub group: String,
    pub unit: Unit,
    pub value: f64,
}

/// A source of sensor readings, polled from the sampler thread.
pub trait SensorBackend: Send {
    fn name(&self) -> &str;
    /// Append current readings to `out`. Errors are reported but must not be fatal.
    fn poll(&mut self, out: &mut Vec<Reading>) -> Result<(), String>;
}

/// A reading plus its running statistics.
#[derive(Debug, Clone, PartialEq)]
pub struct Tracked {
    pub reading: Reading,
    pub min: f64,
    pub max: f64,
    sum: f64,
    pub samples: u64,
    /// Most recent values, oldest first, at most [`HISTORY_LEN`].
    pub history: VecDeque<f32>,
}

impl Tracked {
    pub fn avg(&self) -> f64 {
        if self.samples == 0 {
            0.0
        } else {
            self.sum / self.samples as f64
        }
    }
}

/// All readings seen so far, in first-seen order, with min/max/avg.
#[derive(Debug, Clone, Default)]
pub struct SensorTable {
    entries: Vec<Tracked>,
    index: HashMap<String, usize>,
}

impl SensorTable {
    pub fn update(&mut self, readings: impl IntoIterator<Item = Reading>) {
        for r in readings {
            if !r.value.is_finite() {
                continue;
            }
            match self.index.get(&r.id) {
                Some(&i) => {
                    let t = &mut self.entries[i];
                    t.min = t.min.min(r.value);
                    t.max = t.max.max(r.value);
                    t.sum += r.value;
                    t.samples += 1;
                    if t.history.len() == HISTORY_LEN {
                        t.history.pop_front();
                    }
                    t.history.push_back(r.value as f32);
                    t.reading = r;
                }
                None => {
                    self.index.insert(r.id.clone(), self.entries.len());
                    self.entries.push(Tracked {
                        min: r.value,
                        max: r.value,
                        sum: r.value,
                        samples: 1,
                        history: VecDeque::from([r.value as f32]),
                        reading: r,
                    });
                }
            }
        }
    }

    /// Reset min/max/avg to the current values.
    pub fn reset_stats(&mut self) {
        for t in &mut self.entries {
            t.min = t.reading.value;
            t.max = t.reading.value;
            t.sum = t.reading.value;
            t.samples = 1;
        }
    }

    pub fn entries(&self) -> &[Tracked] {
        &self.entries
    }

    pub fn get(&self, id: &str) -> Option<&Tracked> {
        self.index.get(id).map(|&i| &self.entries[i])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(id: &str, v: f64) -> Reading {
        Reading {
            id: id.into(),
            label: id.into(),
            group: "g".into(),
            unit: Unit::Mhz,
            value: v,
        }
    }

    #[test]
    fn tracks_min_max_avg() {
        let mut t = SensorTable::default();
        t.update([r("a", 100.0), r("b", 1.0)]);
        t.update([r("a", 300.0)]);
        t.update([r("a", 200.0), r("b", f64::NAN)]);
        let a = t.get("a").unwrap();
        assert_eq!(
            (a.reading.value, a.min, a.max, a.avg()),
            (200.0, 100.0, 300.0, 200.0)
        );
        assert_eq!(t.get("b").unwrap().samples, 1);
        assert_eq!(a.history, [100.0, 300.0, 200.0]);
        assert_eq!(t.entries()[0].reading.id, "a");
        t.reset_stats();
        let a = t.get("a").unwrap();
        assert_eq!((a.min, a.max, a.avg()), (200.0, 200.0, 200.0));
    }

    #[test]
    fn history_is_bounded() {
        let mut t = SensorTable::default();
        for i in 0..HISTORY_LEN + 5 {
            t.update([r("a", i as f64)]);
        }
        let h = &t.get("a").unwrap().history;
        assert_eq!(h.len(), HISTORY_LEN);
        assert_eq!(h[0], 5.0);
    }

    #[test]
    fn unit_format() {
        assert_eq!(Unit::Mhz.format(4987.6), "4988 MHz");
        assert_eq!(Unit::Percent.format(12.34), "12.3 %");
        assert_eq!(Unit::Volt.format(1.25), "1.250 V");
    }
}
