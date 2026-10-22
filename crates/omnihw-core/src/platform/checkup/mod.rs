//! Collectors for [`crate::checkup::Checkup`]. Called once from `SystemInfo::gather` on the
//! background thread; nothing here runs on the UI thread or makes network requests.

use crate::checkup::Checkup;
use crate::linux::{Root, block, firmware, net, os};

pub fn gather() -> Checkup {
    let root = Root::system();
    Checkup {
        firmware: firmware::firmware(&root),
        nics: net::wired_links(&root),
        nvme_links: block::nvme_links(&root),
        reboot_required: os::reboot_required(&root),
        swap: os::swap_space(&root),
        kernel_taint: os::kernel_taint(&root),
    }
}
