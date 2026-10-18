//! Run the disk speed test on every internal NVMe/SATA drive with a usable volume.
//! `cargo run --release -p omnihw-core --example disktest`

use omnihw_core::SystemInfo;
use omnihw_core::disktest::{self, Progress};
use omnihw_core::storage::BusType;

fn main() {
    let info = SystemInfo::gather();
    for d in &info.drives {
        if d.removable || !matches!(d.bus, BusType::Nvme | BusType::Sata | BusType::Ata) {
            println!("Disk {}: {} ({}) skipped", d.index, d.name(), d.bus.name());
            continue;
        }
        let link = disktest::nvme_link(d, &info.checkup.nvme_links);
        let expected = disktest::expectation(d, link);
        let Some(v) = disktest::pick_volume(d) else {
            println!("Disk {}: {} has no usable volume", d.index, d.name());
            continue;
        };
        println!("Disk {}: {} on {} ...", d.index, d.name(), v.mount);
        match disktest::run(d, expected, &Progress::default()) {
            Ok(r) => {
                println!("  file        {} MiB at {}", r.file_mb, r.path);
                println!("  seq read    {:8.1} MB/s", r.seq_read_mbs);
                println!("  seq write   {:8.1} MB/s", r.seq_write_mbs);
                println!("  4K Q32 read {:8.1} MB/s", r.rnd4k_read_q32_mbs);
                println!("  4K read     {:8.1} MB/s", r.rnd4k_read_mbs);
                println!("  4K write    {:8.1} MB/s", r.rnd4k_write_mbs);
                if let Some(e) = &r.expected {
                    println!(
                        "  typical     {:.0}-{:.0} MB/s ({})",
                        e.seq_read_mbs.0, e.seq_read_mbs.1, e.reason
                    );
                }
                println!("  verdict     {:?}", r.verdict());
            }
            Err(e) => println!("  error: {e}"),
        }
    }
}
