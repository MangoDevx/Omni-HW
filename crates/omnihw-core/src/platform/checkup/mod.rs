//! Collectors for [`crate::checkup::Checkup`]. Called once from `SystemInfo::gather` on the
//! background thread; nothing here runs on the UI thread or makes network requests.

use crate::checkup::Checkup;

/// Only the NVMe PCIe links so far.
pub fn gather() -> Checkup {
    Checkup {
        nvme_links: crate::linux::block::nvme_links(&crate::linux::Root::system()),
        ..Default::default()
    }
}
