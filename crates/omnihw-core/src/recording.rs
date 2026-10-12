//! Sensor recording: every sample of every sensor over time, for later diagnosis.
//!
//! Columns are fixed when the recording starts (the sensor set is stable after the
//! first sample); sensors that appear later are ignored and missing values are NaN.

use crate::sensors::{SensorTable, Unit};

#[derive(Debug, Clone, PartialEq)]
pub struct Column {
    pub id: String,
    pub label: String,
    pub group: String,
    pub unit: Unit,
}

impl Column {
    /// Key for matching the same sensor across recordings: "group / label".
    /// Recordings loaded from CSV use this as their column id (sensor ids aren't stored).
    pub fn key(&self) -> String {
        format!("{} / {}", self.group, self.label)
    }

    /// CSV header text, e.g. "CPU Usage / Total CPU Usage [%]".
    pub fn header(&self) -> String {
        let unit = self.unit.symbol();
        if unit.is_empty() {
            format!("{} / {}", self.group, self.label)
        } else {
            format!("{} / {} [{unit}]", self.group, self.label)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stats {
    pub min: f64,
    pub max: f64,
    pub avg: f64,
    pub first: f64,
    pub last: f64,
}

#[derive(Debug, Clone, Default)]
pub struct Recording {
    /// Wall-clock start, seconds since the Unix epoch.
    pub started_unix: f64,
    pub columns: Vec<Column>,
    /// Seconds since start, one per sample.
    pub times: Vec<f64>,
    /// `values[column][sample]`; NaN where a sensor had no reading.
    pub values: Vec<Vec<f32>>,
}

impl Recording {
    /// Start a recording with one column per sensor currently in `table`.
    pub fn new(started_unix: f64, table: &SensorTable) -> Recording {
        let columns: Vec<Column> = table
            .entries()
            .iter()
            .map(|t| Column {
                id: t.reading.id.clone(),
                label: t.reading.label.clone(),
                group: t.reading.group.clone(),
                unit: t.reading.unit,
            })
            .collect();
        let values = vec![Vec::new(); columns.len()];
        Recording {
            started_unix,
            columns,
            times: Vec::new(),
            values,
        }
    }

    pub fn len(&self) -> usize {
        self.times.len()
    }

    pub fn is_empty(&self) -> bool {
        self.times.is_empty()
    }

    pub fn duration_secs(&self) -> f64 {
        self.times.last().copied().unwrap_or(0.0)
    }

    /// Record the table's current values at `elapsed` seconds. Returns the new sample's index.
    pub fn push(&mut self, elapsed: f64, table: &SensorTable) -> usize {
        self.times.push(elapsed);
        for (c, col) in self.columns.iter().enumerate() {
            let v = table
                .get(&col.id)
                .map_or(f32::NAN, |t| t.reading.value as f32);
            self.values[c].push(v);
        }
        self.times.len() - 1
    }

    pub fn column_index(&self, id: &str) -> Option<usize> {
        self.columns.iter().position(|c| c.id == id)
    }

    /// Column with the given [`Column::key`] ("group / label").
    pub fn column_by_key(&self, key: &str) -> Option<usize> {
        self.columns.iter().position(|c| c.key() == key)
    }

    /// Parse a recording written by [`Recording::csv_header`] / [`Recording::csv_row`].
    ///
    /// Column ids become [`Column::key`] ("group / label"), since sensor ids aren't stored in
    /// the file; comparisons therefore match sensors by group and label. Rows with the wrong
    /// number of fields (e.g. a final row cut short by a crash) are skipped.
    pub fn from_csv(text: &str) -> Result<Recording, String> {
        let mut lines = text
            .trim_start_matches('\u{feff}')
            .lines()
            .filter(|l| !l.trim().is_empty());
        let header = split_csv_line(lines.next().ok_or("Empty file")?);
        if header.len() < 2 || !header[0].starts_with("Time") || !header[1].starts_with("Elapsed") {
            return Err("Not an Omni-HW recording (unexpected header)".into());
        }
        let columns: Vec<Column> = header[2..].iter().map(|h| parse_column(h)).collect();
        let mut rec = Recording {
            started_unix: 0.0,
            values: vec![Vec::new(); columns.len()],
            columns,
            times: Vec::new(),
        };
        for line in lines {
            let fields = split_csv_line(line);
            if fields.len() != header.len() {
                continue;
            }
            let Ok(elapsed) = fields[1].trim().parse::<f64>() else {
                continue;
            };
            if rec.times.is_empty() {
                rec.started_unix = parse_iso_utc(&fields[0]).map_or(0.0, |t| t - elapsed);
            }
            rec.times.push(elapsed);
            for (col, f) in rec.values.iter_mut().zip(&fields[2..]) {
                col.push(f.trim().parse::<f32>().unwrap_or(f32::NAN));
            }
        }
        if rec.times.is_empty() {
            return Err("Recording has no samples".into());
        }
        Ok(rec)
    }

    /// Statistics over the whole recording, ignoring missing samples.
    pub fn stats(&self, column: usize) -> Option<Stats> {
        let vals = self.values.get(column)?;
        let mut it = vals
            .iter()
            .copied()
            .filter(|v| v.is_finite())
            .map(f64::from);
        let first = it.next()?;
        let (mut min, mut max, mut sum, mut n, mut last) = (first, first, first, 1usize, first);
        for v in it {
            min = min.min(v);
            max = max.max(v);
            sum += v;
            n += 1;
            last = v;
        }
        Some(Stats {
            min,
            max,
            avg: sum / n as f64,
            first,
            last,
        })
    }

    pub fn csv_header(&self) -> String {
        let mut s = String::from("Time (UTC),Elapsed (s)");
        for c in &self.columns {
            s.push(',');
            s.push_str(&csv_field(&c.header()));
        }
        s.push('\n');
        s
    }

    pub fn csv_row(&self, i: usize) -> String {
        let t = self.times[i];
        let mut s = format!("{},{t:.3}", iso_utc(self.started_unix + t));
        for col in &self.values {
            s.push(',');
            let v = col[i];
            if v.is_finite() {
                // At most 3 decimals; f32 values otherwise print noise digits.
                let t = format!("{v:.3}");
                s.push_str(t.trim_end_matches('0').trim_end_matches('.'));
            }
        }
        s.push('\n');
        s
    }
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// Split one CSV line into fields, handling quotes and doubled quotes.
fn split_csv_line(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => out.push(std::mem::take(&mut field)),
            _ => field.push(c),
        }
    }
    out.push(field);
    out
}

fn unit_from_symbol(s: &str) -> Option<Unit> {
    [
        Unit::Mhz,
        Unit::Percent,
        Unit::Celsius,
        Unit::Watt,
        Unit::Volt,
        Unit::Rpm,
        Unit::Megabytes,
        Unit::MegabytesPerSec,
    ]
    .into_iter()
    .find(|u| u.symbol() == s)
}

/// Inverse of [`Column::header`].
fn parse_column(header: &str) -> Column {
    let mut name = header;
    let mut unit = Unit::None;
    if let Some(i) = header.rfind(" [")
        && header.ends_with(']')
        && let Some(u) = unit_from_symbol(&header[i + 2..header.len() - 1])
    {
        name = &header[..i];
        unit = u;
    }
    let (group, label) = name.split_once(" / ").unwrap_or(("", name));
    let mut c = Column {
        id: String::new(),
        label: label.to_string(),
        group: group.to_string(),
        unit,
    };
    c.id = c.key();
    c
}

/// Days since 1970-01-01 for a Gregorian date (inverse of `civil_from_days`).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Parse "2026-10-12T01:13:10.250Z" (as written by [`iso_utc`]) to Unix seconds.
pub fn parse_iso_utc(s: &str) -> Option<f64> {
    let s = s.trim().strip_suffix('Z')?;
    let (date, time) = s.split_once('T')?;
    let mut d = date.split('-');
    let (y, m, day) = (
        d.next()?.parse().ok()?,
        d.next()?.parse().ok()?,
        d.next()?.parse().ok()?,
    );
    let mut t = time.split(':');
    let (h, min, sec): (f64, f64, f64) = (
        t.next()?.parse().ok()?,
        t.next()?.parse().ok()?,
        t.next()?.parse().ok()?,
    );
    Some(days_from_civil(y, m, day) as f64 * 86400.0 + h * 3600.0 + min * 60.0 + sec)
}

/// "2026-10-12T01:13:10.250Z" from Unix seconds.
pub fn iso_utc(unix: f64) -> String {
    let ms_total = (unix * 1000.0).round() as i64;
    let (secs, ms) = (ms_total.div_euclid(1000), ms_total.rem_euclid(1000));
    let (days, rem) = (secs.div_euclid(86400), secs.rem_euclid(86400));
    let (y, m, d) = crate::civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{ms:03}Z",
        rem / 3600,
        rem / 60 % 60,
        rem % 60
    )
}

/// "1:02:03" / "4:05" elapsed time.
pub fn format_elapsed(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sensors::Reading;

    fn r(id: &str, unit: Unit, v: f64) -> Reading {
        Reading {
            id: id.into(),
            label: format!("{id} label"),
            group: "G, 1".into(),
            unit,
            value: v,
        }
    }

    #[test]
    fn records_and_summarizes() {
        let mut t = SensorTable::default();
        t.update([r("a", Unit::Percent, 10.0), r("b", Unit::Mhz, 4000.0)]);
        let mut rec = Recording::new(1_760_231_590.0, &t);
        rec.push(0.0, &t);
        t.update([r("a", Unit::Percent, 30.0), r("new", Unit::Watt, 1.0)]);
        rec.push(1.0, &t);
        t.update([r("a", Unit::Percent, 20.0), r("b", Unit::Mhz, 4200.0)]);
        rec.push(2.0, &t);

        assert_eq!(rec.len(), 3);
        assert_eq!(
            rec.columns.len(),
            2,
            "sensors appearing later are not added"
        );
        let a = rec.stats(rec.column_index("a").unwrap()).unwrap();
        assert_eq!(
            (a.min, a.max, a.avg, a.first, a.last),
            (10.0, 30.0, 20.0, 10.0, 20.0)
        );
        let b = rec.stats(1).unwrap();
        // "b" kept its last value on the second sample (the table holds the latest reading).
        assert_eq!((b.first, b.last, b.max), (4000.0, 4200.0, 4200.0));
        assert_eq!(rec.duration_secs(), 2.0);
    }

    #[test]
    fn csv_output() {
        let mut t = SensorTable::default();
        t.update([r("a", Unit::Percent, 12.5), r("p", Unit::None, 3.0)]);
        let mut rec = Recording::new(1_760_231_590.0, &t);
        rec.push(1.5, &t);
        t.update([r("a", Unit::Percent, 4935.0156), r("p", Unit::None, 0.0)]);
        rec.push(2.0, &t);
        assert_eq!(
            rec.csv_row(1),
            "2025-10-12T01:13:12.000Z,2.000,4935.016,0
"
        );
        assert_eq!(
            rec.csv_header(),
            "Time (UTC),Elapsed (s),\"G, 1 / a label [%]\",\"G, 1 / p label\"\n"
        );
        assert_eq!(rec.csv_row(0), "2025-10-12T01:13:11.500Z,1.500,12.5,3\n");
    }

    #[test]
    fn missing_values_are_empty_cells() {
        let t = SensorTable::default();
        let mut rec = Recording::new(0.0, &t);
        rec.columns.push(Column {
            id: "x".into(),
            label: "x".into(),
            group: "g".into(),
            unit: Unit::Mhz,
        });
        rec.values.push(Vec::new());
        rec.push(0.0, &t);
        assert_eq!(rec.csv_row(0), "1970-01-01T00:00:00.000Z,0.000,\n");
        assert!(rec.stats(0).is_none());
    }

    #[test]
    fn elapsed_format() {
        assert_eq!(format_elapsed(65.0), "1:05");
        assert_eq!(format_elapsed(3723.0), "1:02:03");
    }

    fn to_csv(rec: &Recording) -> String {
        let mut s = rec.csv_header();
        for i in 0..rec.len() {
            s.push_str(&rec.csv_row(i));
        }
        s
    }

    #[test]
    fn csv_round_trip() {
        let mut t = SensorTable::default();
        let reading = |id: &str, label: &str, group: &str, unit, value| Reading {
            id: id.into(),
            label: label.into(),
            group: group.into(),
            unit,
            value,
        };
        t.update([
            reading("a", "P-core 0 / T0 Usage", "CPU Usage", Unit::Percent, 12.5),
            reading("b", "Drive \"C\" Temp", "Drive: X, Y", Unit::Celsius, 41.0),
            reading("c", "Performance State (P)", "GPU: RTX", Unit::None, 2.0),
            reading("d", "Read Rate", "Drive: X, Y", Unit::MegabytesPerSec, 0.25),
        ]);
        let mut rec = Recording::new(1_760_231_590.0, &t);
        rec.push(0.0, &t);
        // "d" drops out for one sample -> empty cell -> NaN after parsing.
        rec.push(1.0, &t);
        rec.values[3][1] = f32::NAN;
        let text = to_csv(&rec);

        for input in [text.clone(), format!("\u{feff}{text}")] {
            let back = Recording::from_csv(&input).unwrap();
            assert_eq!(back.len(), 2);
            assert_eq!(back.times, vec![0.0, 1.0]);
            assert!((back.started_unix - 1_760_231_590.0).abs() < 1e-3);
            let cols: Vec<_> = back
                .columns
                .iter()
                .map(|c| (c.group.as_str(), c.label.as_str(), c.unit))
                .collect();
            assert_eq!(
                cols,
                vec![
                    ("CPU Usage", "P-core 0 / T0 Usage", Unit::Percent),
                    ("Drive: X, Y", "Drive \"C\" Temp", Unit::Celsius),
                    ("GPU: RTX", "Performance State (P)", Unit::None),
                    ("Drive: X, Y", "Read Rate", Unit::MegabytesPerSec),
                ]
            );
            assert_eq!(back.columns[0].id, "CPU Usage / P-core 0 / T0 Usage");
            assert_eq!(back.column_by_key("Drive: X, Y / Read Rate"), Some(3));
            assert_eq!(back.values[0], vec![12.5, 12.5]);
            assert_eq!(back.values[1][1], 41.0);
            assert_eq!(back.values[3][0], 0.25);
            assert!(back.values[3][1].is_nan());
        }
    }

    #[test]
    fn csv_rejects_and_skips() {
        assert!(Recording::from_csv("").is_err());
        assert!(Recording::from_csv("a,b,c\n1,2,3\n").is_err());
        assert!(Recording::from_csv("Time (UTC),Elapsed (s),G / x [W]\n").is_err());
        // A truncated final row is skipped.
        let r = Recording::from_csv(
            "Time (UTC),Elapsed (s),G / x [W],G / y [W]\n2026-10-12T00:00:00.000Z,0.000,1,2\n2026-10-12T00:00:01.000Z,1.0",
        )
        .unwrap();
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn iso_parse_round_trip() {
        for t in [0.0, 1_760_231_591.5, 951_782_400.25] {
            assert!(
                (parse_iso_utc(&iso_utc(t)).unwrap() - t).abs() < 1e-3,
                "{t}"
            );
        }
        assert!(parse_iso_utc("garbage").is_none());
    }
}
