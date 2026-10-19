use super::section;
use crate::sampler::Snapshot;
use omnihw_core::sensors::{CPU_PACKAGE_POWER, CPU_TEMP};
use omnihw_core::{SystemInfo, format_bytes};

pub fn show(ui: &mut egui::Ui, info: &SystemInfo, snap: &Snapshot) {
    let c = &info.cpu;
    let socket = info.smbios.as_ref().and_then(|s| s.processors.first());
    let (codename, microarch, process) = match info.codename {
        Some(n) => (
            n.codename.to_string(),
            n.microarch.to_string(),
            n.process.to_string(),
        ),
        None => ("Unknown".into(), String::new(), String::new()),
    };
    section(
        ui,
        "Processor",
        &[
            ("Name", c.brand.clone()),
            ("Code Name", codename),
            ("Microarchitecture", microarch),
            ("Technology", process),
            (
                "Package",
                socket.map(|p| p.socket.clone()).unwrap_or_default(),
            ),
            ("Vendor", c.vendor_string.clone()),
            (
                "Family / Model / Stepping",
                format!("{:X} / {:X} / {:X}", c.family, c.model, c.stepping),
            ),
            (
                "Ext. Family / Ext. Model",
                format!("{:X} / {:X}", c.ext_family, c.ext_model),
            ),
            ("Instructions", c.instructions_line()),
        ],
    );

    let avg = snap.table.get("cpu/clock/avg").map(|t| t.reading.value);
    let bus = socket.map(|p| p.external_clock_mhz).filter(|&b| b > 0);
    let mut clocks = vec![
        (
            "Core Speed (avg)",
            avg.map(|v| format!("{v:.0} MHz"))
                .unwrap_or_else(|| "sampling...".into()),
        ),
        (
            "Base Clock",
            info.base_mhz
                .map(|m| format!("{m} MHz"))
                .unwrap_or_default(),
        ),
    ];
    if let Some(b) = bus {
        clocks.push(("Bus Speed", format!("{b} MHz")));
        if let Some(v) = avg {
            clocks.push(("Multiplier (avg)", format!("x {:.1}", v / b as f64)));
        }
    }
    if let Some(p) = snap.table.first_of(&CPU_PACKAGE_POWER) {
        clocks.push(("Package Power", format!("{:.1} W", p.reading.value)));
    }
    if let Some(t) = snap.table.get(CPU_TEMP) {
        clocks.push(("Temperature", format!("{:.0} °C", t.reading.value)));
    }
    if let Some(v) = snap.table.get("cpu/volt/vid") {
        clocks.push(("Core Voltage (VID)", format!("{:.3} V", v.reading.value)));
    }
    section(ui, "Clocks", &clocks);

    // One line per cache name; hybrid parts list both core types: "6 x 48 KB (12-way) + 8 x 32 KB (8-way)".
    let mut lines: Vec<(String, Vec<String>)> = Vec::new();
    let mut add = |name: String, part: String| match lines.iter_mut().find(|(n, _)| *n == name) {
        Some((_, parts)) => parts.push(part),
        None => lines.push((name, vec![part])),
    };
    match &info.topology {
        Some(t) => {
            for s in t.cache_summary() {
                add(
                    s.name(),
                    format!(
                        "{} x {} ({})",
                        s.count,
                        format_bytes(s.size_bytes),
                        s.ways_text()
                    ),
                );
            }
        }
        None => {
            for d in &c.caches {
                add(
                    d.name(),
                    format!("{} ({}-way)", format_bytes(d.size_bytes), d.ways),
                );
            }
        }
    }
    let cache_rows: Vec<(&str, String)> = lines
        .into_iter()
        .map(|(n, p)| (cache_label(&n), p.join(" + ")))
        .collect();
    section(ui, "Cache", &cache_rows);

    if let Some(t) = &info.topology {
        let (p, e) = t.p_e_counts();
        let cores = if e > 0 {
            format!("{} ({p}P + {e}E)", t.cores.len())
        } else {
            t.cores.len().to_string()
        };
        section(
            ui,
            "Topology",
            &[
                ("Sockets", t.packages.to_string()),
                ("Cores", cores),
                ("Threads", t.logical_count().to_string()),
                ("NUMA Nodes", t.numa_nodes.to_string()),
                ("Processor Groups", t.groups.to_string()),
            ],
        );
    }
}

fn cache_label(name: &str) -> &'static str {
    match name {
        "L1D" => "L1 Data",
        "L1I" => "L1 Inst.",
        "L2" => "Level 2",
        "L3" => "Level 3",
        "L4" => "Level 4",
        _ => "Other",
    }
}
