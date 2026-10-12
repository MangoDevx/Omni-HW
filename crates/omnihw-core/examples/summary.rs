//! Print a CPU-Z style summary, then a few seconds of live sensor samples.
//! `cargo run -p omnihw-core --example summary`

use omnihw_core::{SystemInfo, format_bytes};

fn main() {
    let info = SystemInfo::gather();
    let c = &info.cpu;
    println!("Name        {}", c.brand);
    if let Some(n) = info.codename {
        println!(
            "Codename    {} ({}, {})",
            n.codename, n.microarch, n.process
        );
    }
    println!(
        "Family      {:X}  Model {:X}  Stepping {:X}",
        c.family, c.model, c.stepping
    );
    println!(
        "Ext. Family {:X}  Ext. Model {:X}",
        c.ext_family, c.ext_model
    );
    println!("Instructions {}", c.instructions_line());
    if let Some(mhz) = info.base_mhz {
        println!("Base clock  {mhz} MHz");
    }
    if let Some(t) = &info.topology {
        let (p, e) = t.p_e_counts();
        println!(
            "Cores       {} ({p}P + {e}E)  Threads {}  Packages {}",
            t.cores.len(),
            t.logical_count(),
            t.packages
        );
        for s in t.cache_summary() {
            println!(
                "  {:<4} {} x {:>8}  {}",
                s.name(),
                s.count,
                format_bytes(s.size_bytes),
                s.ways_text()
            );
        }
    }
    if let Some(s) = &info.smbios {
        println!("SMBIOS      {}.{}", s.version.0, s.version.1);
        if let Some(b) = &s.board {
            println!("Board       {} {} {}", b.manufacturer, b.product, b.version);
        }
        if let Some(b) = &s.bios {
            println!(
                "BIOS        {} {} ({})",
                b.vendor, b.version, b.release_date
            );
        }
        for p in &s.processors {
            println!(
                "Socket      {}  ext. clock {} MHz",
                p.socket, p.external_clock_mhz
            );
        }
        println!(
            "Memory      {} slots, {} MB",
            s.memory_slots,
            s.total_memory_mb()
        );
        for m in s.populated_memory() {
            println!(
                "  {:<12} {:>6} MB {} {} {}/{} MT/s  {} {}",
                m.locator,
                m.size_mb,
                m.memory_type,
                m.form_factor,
                m.speed_mts,
                m.configured_speed_mts,
                m.manufacturer,
                m.part_number
            );
        }
    }
    for g in &info.gpus {
        let chip = g
            .chip()
            .map(|c| format!("{} ({}, {})", c.chip, c.architecture, c.process))
            .unwrap_or_default();
        println!("GPU         {} [{}]  {}", g.name, g.pci_ids(), chip);
        println!(
            "  VRAM {}  driver {}  board {}",
            format_bytes(g.dedicated_vram),
            g.driver_version,
            g.board_vendor().unwrap_or("-")
        );
        if let Some(n) = &g.nvml {
            println!(
                "  NVML: driver {} vbios {} bus {} cores {:?} bus width {:?} max {:?}/{:?} MHz limit {:?} W PCIe {:?} / {:?}",
                n.driver_version,
                n.vbios,
                n.bus_id,
                n.cuda_cores,
                n.memory_bus_width,
                n.max_core_clock_mhz,
                n.max_mem_clock_mhz,
                n.power_limit_w,
                n.pcie_current,
                n.pcie_max
            );
        }
    }
    if let Some(o) = &info.os {
        println!("OS          {} on {}", o.summary(), o.computer_name);
    }
    println!("Battery     {:?}", info.battery);
    for d in &info.drives {
        println!(
            "Drive {}     {} [{} {}] {} fw {} sn {}",
            d.index,
            d.name(),
            d.kind(),
            d.bus.name(),
            format_bytes(d.size_bytes),
            d.firmware,
            d.serial
        );
        for v in &d.volumes {
            println!(
                "  {} {:?} {} {} free of {}",
                v.mount,
                v.label,
                v.file_system,
                format_bytes(v.free_bytes),
                format_bytes(v.total_bytes)
            );
        }
        if let Some(h) = &d.nvme_health {
            println!(
                "  NVMe health {}% temp {}C written {} POH {} warnings {:?}",
                h.health_pct(),
                h.temperature_c,
                format_bytes(h.data_written_bytes as u64),
                h.power_on_hours,
                h.warnings()
            );
        }
    }
    for w in &info.warnings {
        println!("warning: {w}");
    }

    #[cfg(windows)]
    {
        use omnihw_core::sensors::{SensorBackend, SensorTable};
        use omnihw_core::win::nvml::NvmlBackend;
        use omnihw_core::win::perf::{CpuPerfBackend, GpuPerfBackend, MemoryBackend};
        let mut backends: Vec<Box<dyn SensorBackend>> = vec![Box::new(MemoryBackend)];
        match GpuPerfBackend::new(&info.gpus) {
            Ok(b) => backends.push(Box::new(b)),
            Err(e) => println!("warning: {e}"),
        }
        if let Some(b) = NvmlBackend::new(&info.gpus) {
            backends.push(Box::new(b));
        }
        backends.push(Box::new(omnihw_core::win::storage::StorageBackend::new(
            &info.drives,
        )));
        match CpuPerfBackend::new(info.base_mhz.unwrap_or(0), info.topology.as_ref()) {
            Ok(b) => backends.push(Box::new(b)),
            Err(e) => println!("warning: {e}"),
        }
        let mut table = SensorTable::default();
        for _ in 0..3 {
            std::thread::sleep(std::time::Duration::from_millis(1000));
            let mut out = Vec::new();
            for b in &mut backends {
                if let Err(e) = b.poll(&mut out) {
                    println!("warning: {}: {e}", b.name());
                }
            }
            table.update(out);
        }
        println!("\nSensors (3 samples):");
        for t in table.entries() {
            let u = t.reading.unit;
            println!(
                "  {:<14} {:<28} {:>10}  min {:>10}  max {:>10}",
                t.reading.group,
                t.reading.label,
                u.format(t.reading.value),
                u.format(t.min),
                u.format(t.max)
            );
        }
    }
}
