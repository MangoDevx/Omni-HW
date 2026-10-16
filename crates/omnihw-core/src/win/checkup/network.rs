//! Link speed of connected wired (Ethernet) adapters via `GetIfTable2`.

use crate::checkup::NicLink;
use crate::win::wide::from_wide;
use windows::Win32::Foundation::NO_ERROR;
use windows::Win32::NetworkManagement::IpHelper::{
    FreeMibTable, GetIfTable2, IF_TYPE_ETHERNET_CSMACD, MIB_IF_ROW2, MIB_IF_TABLE2,
};
use windows::Win32::NetworkManagement::Ndis::{IfOperStatusUp, MediaConnectStateConnected};

/// `InterfaceAndOperStatusFlags` bits.
const HARDWARE_INTERFACE: u8 = 1 << 0;
const CONNECTOR_PRESENT: u8 = 1 << 2;

/// The interface table from `GetIfTable2`, freed with `FreeMibTable` on drop.
struct IfTable(*mut MIB_IF_TABLE2);

impl IfTable {
    fn get() -> Option<IfTable> {
        let mut table: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
        // SAFETY: valid out pointer; on success the table is owned by the returned guard.
        if unsafe { GetIfTable2(&mut table) } != NO_ERROR || table.is_null() {
            return None;
        }
        Some(IfTable(table))
    }

    fn rows(&self) -> &[MIB_IF_ROW2] {
        // SAFETY: GetIfTable2 returned a non-null table whose `Table` array holds `NumEntries`
        // rows; the slice borrows `self`, so it cannot outlive the allocation.
        unsafe {
            std::slice::from_raw_parts((*self.0).Table.as_ptr(), (*self.0).NumEntries as usize)
        }
    }
}

impl Drop for IfTable {
    fn drop(&mut self) {
        // SAFETY: allocated by GetIfTable2 and freed exactly once here.
        unsafe { FreeMibTable(self.0.cast()) };
    }
}

pub fn wired_links() -> Vec<NicLink> {
    let Some(table) = IfTable::get() else {
        return Vec::new();
    };
    let mut out: Vec<NicLink> = Vec::new();
    for r in table.rows() {
        let flags = r.InterfaceAndOperStatusFlags._bitfield;
        if r.Type != IF_TYPE_ETHERNET_CSMACD
            || flags & HARDWARE_INTERFACE == 0
            || flags & CONNECTOR_PRESENT == 0
            || r.OperStatus != IfOperStatusUp
            || r.MediaConnectState != MediaConnectStateConnected
            || r.ReceiveLinkSpeed == 0
            || r.ReceiveLinkSpeed == u64::MAX
        {
            continue;
        }
        let name = from_wide(&r.Description);
        // Filter/sublayer interfaces repeat the same adapter.
        if out.iter().any(|n| n.name == name) {
            continue;
        }
        out.push(NicLink {
            name,
            speed_bps: r.ReceiveLinkSpeed.min(r.TransmitLinkSpeed.max(1)),
        });
    }
    out
}
