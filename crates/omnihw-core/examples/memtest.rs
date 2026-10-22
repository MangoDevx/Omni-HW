//! Run the quick memory test and print the result.
//! `cargo run --release -p omnihw-core --example memtest`

use omnihw_core::memtest::{self, Progress};
use omnihw_core::platform;
use omnihw_core::smbios::Smbios;
use std::time::Instant;

fn main() {
    let smbios = platform::smbios_buffer().and_then(|b| Smbios::parse_raw(&b).ok());
    let expected = smbios.as_ref().and_then(|s| {
        let dimms: Vec<_> = s.populated_memory().collect();
        memtest::expectation(&dimms, s.memory_slots)
    });
    let available = platform::memory_status().map_or(0, |m| m.available_bytes);
    let started = Instant::now();
    match memtest::run(available, expected, &Progress::default()) {
        Ok(r) => {
            println!("Buffer      {} MB, {} threads", r.buffer_mb, r.threads);
            println!("Read        {:.1} GB/s", r.read_gbs);
            println!(
                "Read 1 core {:.1} GB/s on {:?}",
                r.read_single_gbs, r.read_cpu
            );
            println!("Write       {:.1} GB/s", r.write_gbs);
            println!("Copy        {:.1} GB/s", r.copy_gbs);
            println!("Latency     {:.1} ns", r.latency_ns);
            if let Some(e) = &r.expected {
                println!(
                    "Expected    {:.1} GB/s ({}-{}, {} channels, estimated)",
                    e.theoretical_gbs, e.memory_type, e.mts, e.channels
                );
            }
            if let Some(p) = r.efficiency_pct() {
                println!("Efficiency  {p:.0} %");
            }
            println!(
                "Verdicts    bandwidth {:?}, latency {:?}",
                r.bandwidth_verdict(),
                r.latency_verdict()
            );
        }
        Err(e) => println!("Error: {e}"),
    }
    println!("Took        {:.2} s", started.elapsed().as_secs_f64());
}
