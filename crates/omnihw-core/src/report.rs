//! System report export (plain text and self-contained HTML).

use crate::sensors::Tracked;
use crate::{SystemInfo, format_bytes, format_uptime};

/// A titled list of key/value rows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Section {
    pub title: String,
    pub rows: Vec<(String, String)>,
}

impl Section {
    fn new(title: impl Into<String>) -> Self {
        Section {
            title: title.into(),
            rows: Vec::new(),
        }
    }

    fn row(&mut self, k: impl Into<String>, v: impl Into<String>) {
        let v = v.into();
        if !v.trim().is_empty() {
            self.rows.push((k.into(), v));
        }
    }
}

/// Collect the static information into report sections.
pub fn sections(info: &SystemInfo, uptime_secs: Option<u64>) -> Vec<Section> {
    let mut out = Vec::new();

    let mut s = Section::new("System");
    if let Some(o) = &info.os {
        s.row("Operating System", o.summary());
        s.row("Computer Name", &o.computer_name);
    }
    if let Some(u) = uptime_secs {
        s.row("Uptime", format_uptime(u));
    }
    if let Some(sys) = info.smbios.as_ref().and_then(|x| x.system.as_ref()) {
        s.row("System", format!("{} {}", sys.manufacturer, sys.product));
    }
    if let Some(b) = &info.battery {
        s.row(
            "Battery",
            format!("{}%{}", b.percent, if b.on_ac { " (AC)" } else { "" }),
        );
    }
    if let Some(b) = &info.battery_health {
        s.row("Battery Health", b.summary());
    }
    out.push(s);

    let c = &info.cpu;
    let mut s = Section::new("Processor");
    s.row("Name", &c.brand);
    if let Some(n) = info.codename {
        s.row("Code Name", n.codename);
        s.row("Microarchitecture", n.microarch);
        s.row("Technology", n.process);
    }
    if let Some(p) = info.smbios.as_ref().and_then(|x| x.processors.first()) {
        s.row("Package", &p.socket);
    }
    s.row(
        "Family / Model / Stepping",
        format!("{:X} / {:X} / {:X}", c.family, c.model, c.stepping),
    );
    s.row(
        "Ext. Family / Ext. Model",
        format!("{:X} / {:X}", c.ext_family, c.ext_model),
    );
    if let Some(m) = info.base_mhz {
        s.row("Base Clock", format!("{m} MHz"));
    }
    if let Some(t) = &info.topology {
        let (p, e) = t.p_e_counts();
        let cores = if e > 0 {
            format!("{} ({p}P + {e}E)", t.cores.len())
        } else {
            t.cores.len().to_string()
        };
        s.row("Cores", cores);
        s.row("Threads", t.logical_count().to_string());
        for cs in t.cache_summary() {
            s.row(
                cs.name(),
                format!(
                    "{} x {} ({})",
                    cs.count,
                    format_bytes(cs.size_bytes),
                    cs.ways_text()
                ),
            );
        }
    }
    s.row("Instructions", c.instructions_line());
    out.push(s);

    if let Some(sm) = &info.smbios {
        let mut s = Section::new("Mainboard");
        if let Some(b) = &sm.board {
            s.row("Manufacturer", &b.manufacturer);
            s.row("Model", &b.product);
            s.row("Version", &b.version);
        }
        if let Some(b) = &sm.bios {
            s.row(
                "BIOS",
                format!("{} {} ({})", b.vendor, b.version, b.release_date),
            );
        }
        s.row(
            "SMBIOS Version",
            format!("{}.{}", sm.version.0, sm.version.1),
        );
        out.push(s);

        let mut s = Section::new("Memory");
        s.row("Installed", format!("{} GB", sm.total_memory_mb() / 1024));
        if let Some(m) = info.memory {
            s.row("Usable", format_bytes(m.total_bytes));
        }
        if sm.memory_slots > 0 {
            s.row(
                "Slots",
                format!(
                    "{} of {} used",
                    sm.populated_memory().count(),
                    sm.memory_slots
                ),
            );
        }
        for d in sm.populated_memory() {
            s.row(
                &d.locator,
                format!(
                    "{} GB {} {} {} MT/s (configured {}) {} {}",
                    d.size_mb / 1024,
                    d.memory_type,
                    d.form_factor,
                    d.speed_mts,
                    d.configured_speed_mts,
                    d.manufacturer,
                    d.part_number
                ),
            );
        }
        out.push(s);
    }

    for g in &info.gpus {
        let mut s = Section::new(format!("Graphics: {}", g.name));
        if let Some(c) = g.chip() {
            s.row(
                "GPU",
                format!("{} ({}, {})", c.chip, c.architecture, c.process),
            );
        }
        s.row(
            "Vendor",
            g.board_vendor().map_or_else(
                || g.vendor_name().to_string(),
                |b| format!("{} ({b})", g.vendor_name()),
            ),
        );
        s.row("Device ID", g.pci_ids());
        s.row("Dedicated Memory", format_bytes(g.dedicated_vram));
        s.row("Driver", &g.driver_version);
        if let Some((y, m, d)) = g.driver_date {
            s.row("Driver Date", format!("{y}-{m:02}-{d:02}"));
        }
        s.row("Resizable BAR", g.rebar().label());
        if let Some(n) = &g.nvml {
            s.row("NVIDIA Driver", &n.driver_version);
            s.row("BIOS Version", &n.vbios);
            if let Some((gen_, w)) = n.pcie_max {
                s.row("Bus Interface", format!("PCIe x{w} {gen_}.0"));
            }
            if let Some(c) = n.cuda_cores {
                s.row("CUDA Cores", c.to_string());
            }
            if let Some(w) = n.memory_bus_width {
                s.row("Memory Bus", format!("{w} bit"));
            }
            if let Some(p) = n.power_limit_w {
                s.row("Power Limit", format!("{p:.0} W"));
            }
        }
        out.push(s);
    }

    for d in &info.drives {
        let mut s = Section::new(format!("Drive {}: {}", d.index, d.name()));
        s.row("Type", format!("{} ({})", d.kind(), d.bus.name()));
        s.row("Capacity", format_bytes(d.size_bytes));
        s.row("Firmware", &d.firmware);
        s.row("Serial", &d.serial);
        for v in &d.volumes {
            let label = if v.label.is_empty() {
                String::new()
            } else {
                format!(" \"{}\"", v.label)
            };
            s.row(
                format!("Volume {}", v.mount),
                format!(
                    "{}{label}, {} free of {}",
                    v.file_system,
                    format_bytes(v.free_bytes),
                    format_bytes(v.total_bytes)
                ),
            );
        }
        if let Some(h) = &d.nvme_health {
            s.row(
                "Health",
                format!("{}% (life used {}%)", h.health_pct(), h.percentage_used),
            );
            s.row("Temperature", format!("{} °C", h.temperature_c));
            s.row(
                "Data Written",
                format_bytes(h.data_written_bytes.min(u64::MAX as u128) as u64),
            );
            s.row(
                "Data Read",
                format_bytes(h.data_read_bytes.min(u64::MAX as u128) as u64),
            );
            s.row("Power-On Hours", h.power_on_hours.to_string());
            s.row("Power Cycles", h.power_cycles.to_string());
            s.row("Unsafe Shutdowns", h.unsafe_shutdowns.to_string());
            s.row("Media Errors", h.media_errors.to_string());
            let w = h.warnings();
            s.row(
                "Critical Warnings",
                if w.is_empty() {
                    "none".to_string()
                } else {
                    w.join(", ")
                },
            );
        }
        if let Some(a) = &d.ata_smart {
            let opt = |v: Option<u64>| v.map(|v| v.to_string()).unwrap_or_else(|| "-".into());
            if let Some(life) = a.life_remaining_pct() {
                s.row("Life Remaining", format!("{life} %"));
            }
            if let Some(t) = a.temperature_c() {
                s.row("Temperature", format!("{t} °C"));
            }
            s.row("Power-On Hours", opt(a.power_on_hours()));
            s.row("Power Cycles", opt(a.power_cycles()));
            s.row(
                "Bad Sectors",
                format!(
                    "{} reallocated, {} pending, {} uncorrectable",
                    opt(a.reallocated_sectors()),
                    opt(a.pending_sectors()),
                    opt(a.uncorrectable())
                ),
            );
            s.row("CRC (Cable) Errors", opt(a.crc_errors()));
        }
        out.push(s);
    }
    out
}

/// Plain-text report.
pub fn to_text(sections: &[Section], sensors: &[Tracked]) -> String {
    let mut out = format!("Omni-HW report (v{})\n", env!("CARGO_PKG_VERSION"));
    for s in sections {
        out.push_str(&format!("\n== {} ==\n", s.title));
        let w = s
            .rows
            .iter()
            .map(|(k, _)| k.chars().count())
            .max()
            .unwrap_or(0);
        for (k, v) in &s.rows {
            out.push_str(&format!("{k:<w$}  {v}\n"));
        }
    }
    if !sensors.is_empty() {
        out.push_str("\n== Sensors ==\n");
        let mut group = "";
        for t in sensors {
            if t.reading.group != group {
                group = &t.reading.group;
                out.push_str(&format!("\n[{group}]\n"));
            }
            let u = t.reading.unit;
            out.push_str(&format!(
                "{:<36} {:>12}  min {:>12}  max {:>12}  avg {:>12}\n",
                t.reading.label,
                u.format(t.reading.value),
                u.format(t.min),
                u.format(t.max),
                u.format(t.avg())
            ));
        }
    }
    out
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Self-contained HTML report (inline CSS, light and dark).
pub fn to_html(sections: &[Section], sensors: &[Tracked]) -> String {
    let mut body = String::new();
    for s in sections {
        body.push_str(&format!("<section><h2>{}</h2><table>", esc(&s.title)));
        for (k, v) in &s.rows {
            body.push_str(&format!("<tr><th>{}</th><td>{}</td></tr>", esc(k), esc(v)));
        }
        body.push_str("</table></section>\n");
    }
    if !sensors.is_empty() {
        body.push_str("<section class=wide><h2>Sensors</h2><table class=sensors>");
        body.push_str(
            "<tr><th>Sensor</th><th>Current</th><th>Min</th><th>Max</th><th>Average</th></tr>",
        );
        let mut group = "";
        for t in sensors {
            if t.reading.group != group {
                group = &t.reading.group;
                body.push_str(&format!(
                    "<tr class=group><td colspan=5>{}</td></tr>",
                    esc(group)
                ));
            }
            let u = t.reading.unit;
            body.push_str(&format!(
                "<tr><th>{}</th><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                esc(&t.reading.label),
                esc(&u.format(t.reading.value)),
                esc(&u.format(t.min)),
                esc(&u.format(t.max)),
                esc(&u.format(t.avg()))
            ));
        }
        body.push_str("</table></section>\n");
    }
    format!(
        r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>Omni-HW Report</title>
<style>
:root {{ --bg:#f6f7f9; --card:#fff; --fg:#1d2330; --muted:#677084; --line:#e3e6eb; --accent:#4285f4; }}
@media (prefers-color-scheme: dark) {{ :root {{ --bg:#15171c; --card:#1e2128; --fg:#e7e9ee; --muted:#9aa3b5; --line:#2c313a; }} }}
* {{ box-sizing:border-box; }}
body {{ margin:0; padding:24px 16px; background:var(--bg); color:var(--fg); font:14px/1.45 "Segoe UI",system-ui,sans-serif; }}
h1 {{ margin:0 0 4px; font-size:22px; }}
.sub {{ color:var(--muted); margin-bottom:20px; }}
main {{ display:grid; grid-template-columns:repeat(auto-fit,minmax(min(440px,100%),1fr)); gap:16px; max-width:1200px; }}
section {{ background:var(--card); border:1px solid var(--line); border-radius:10px; padding:14px 16px; overflow-x:auto; }}
section.wide {{ grid-column:1/-1; }}
h2 {{ margin:0 0 8px; font-size:15px; border-left:4px solid var(--accent); padding-left:8px; }}
table {{ border-collapse:collapse; width:100%; }}
th,td {{ text-align:left; padding:3px 8px 3px 0; vertical-align:top; border-bottom:1px solid var(--line); }}
th {{ color:var(--muted); font-weight:500; white-space:nowrap; }}
.sensors td {{ font-variant-numeric:tabular-nums; white-space:nowrap; }}
tr.group td {{ font-weight:600; padding-top:10px; color:var(--fg); }}
</style></head>
<body><h1>Omni-HW Report</h1><div class="sub">Generated by Omni-HW v{}</div><main>
{}</main></body></html>
"#,
        env!("CARGO_PKG_VERSION"),
        body
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sensors::{Reading, SensorTable, Unit};

    fn sample() -> Vec<Section> {
        vec![Section {
            title: "Processor".into(),
            rows: vec![
                ("Name".into(), "Test <CPU> & Co".into()),
                ("Cores".into(), "8".into()),
            ],
        }]
    }

    fn table() -> SensorTable {
        let mut t = SensorTable::default();
        t.update([Reading {
            id: "a".into(),
            label: "Core Clock".into(),
            group: "CPU".into(),
            unit: Unit::Mhz,
            value: 4200.0,
        }]);
        t
    }

    #[test]
    fn text_report() {
        let t = table();
        let s = to_text(&sample(), t.entries());
        assert!(s.contains("== Processor =="));
        assert!(s.contains("Name   Test <CPU> & Co"));
        assert!(s.contains("[CPU]"));
        assert!(s.contains("4200 MHz"));
    }

    #[test]
    fn html_report_escapes() {
        let t = table();
        let h = to_html(&sample(), t.entries());
        assert!(h.starts_with("<!doctype html>"));
        assert!(h.contains("Test &lt;CPU&gt; &amp; Co"));
        assert!(!h.contains("<CPU>"));
        assert!(h.contains("<tr class=group><td colspan=5>CPU</td></tr>"));
    }

    #[test]
    fn empty_values_are_skipped() {
        let mut s = Section::new("x");
        s.row("a", "  ");
        s.row("b", "1");
        assert_eq!(s.rows, vec![("b".to_string(), "1".to_string())]);
    }
}
