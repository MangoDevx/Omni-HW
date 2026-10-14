//! Link speed of connected wired (Ethernet) adapters via `GetIfTable2`.

use crate::checkup::NicLink;
use windows::Win32::Foundation::NO_ERROR;
use windows::Win32::NetworkManagement::IpHelper::{
    FreeMibTable, GetIfTable2, IF_TYPE_ETHERNET_CSMACD, MIB_IF_ROW2, MIB_IF_TABLE2,
};
use windows::Win32::NetworkManagement::Ndis::{IfOperStatusUp, MediaConnectStateConnected};

/// `InterfaceAndOperStatusFlags` bits.
const HARDWARE_INTERFACE: u8 = 1 << 0;
const CONNECTOR_PRESENT: u8 = 1 << 2;

pub fn wired_links() -> Vec<NicLink> {
    let mut table: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
    // SAFETY: out pointer; freed with FreeMibTable below.
    if unsafe { GetIfTable2(&mut table) } != NO_ERROR || table.is_null() {
        return Vec::new();
    }
    // SAFETY: GetIfTable2 returned a table with NumEntries rows.
    let rows: &[MIB_IF_ROW2] = unsafe {
        std::slice::from_raw_parts((*table).Table.as_ptr(), (*table).NumEntries as usize)
    };
    let mut out: Vec<NicLink> = Vec::new();
    for r in rows {
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
        let name = super::utf16_z(&r.Description);
        // Filter/sublayer interfaces repeat the same adapter.
        if out.iter().any(|n| n.name == name) {
            continue;
        }
        out.push(NicLink {
            name,
            speed_bps: r.ReceiveLinkSpeed.min(r.TransmitLinkSpeed.max(1)),
        });
    }
    // SAFETY: allocated by GetIfTable2.
    unsafe { FreeMibTable(table.cast()) };
    out
}
