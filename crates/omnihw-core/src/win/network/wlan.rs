//! Wi-Fi connection details from the WLAN API (local queries to the WLAN service only).

use crate::network::{Band, Wifi, channel_from_mhz, phy_name};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::NetworkManagement::WiFi::{
    WLAN_BSS_LIST, WLAN_CONNECTION_ATTRIBUTES, WLAN_INTERFACE_INFO_LIST, WLAN_INTF_OPCODE,
    WlanCloseHandle, WlanEnumInterfaces, WlanFreeMemory, WlanGetNetworkBssList, WlanOpenHandle,
    WlanQueryInterface, dot11_BSS_type_infrastructure, wlan_interface_state_connected,
    wlan_intf_opcode_channel_number, wlan_intf_opcode_current_connection, wlan_intf_opcode_rssi,
};
use windows::core::GUID;

/// A WLAN client handle, closed on drop.
struct Client(HANDLE);

impl Drop for Client {
    fn drop(&mut self) {
        // SAFETY: a handle from WlanOpenHandle, closed exactly once.
        unsafe { WlanCloseHandle(self.0, None) };
    }
}

/// Memory returned by the WLAN API, freed with `WlanFreeMemory` on drop.
struct WlanMem<T>(*mut T);

impl<T> Drop for WlanMem<T> {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: allocated by the WLAN API and freed exactly once.
            unsafe { WlanFreeMemory(self.0 as *const _) };
        }
    }
}

/// "{GUID}" upper-case, as `GetAdaptersAddresses` names adapters.
fn guid_braced(g: &GUID) -> String {
    format!("{{{}}}", crate::win::wide::guid_text(g).to_uppercase())
}

fn query<T: Copy>(c: &Client, guid: &GUID, op: WLAN_INTF_OPCODE) -> Option<T> {
    let mut size = 0u32;
    let mut data: *mut core::ffi::c_void = std::ptr::null_mut();
    // SAFETY: valid client handle and interface GUID; on success `data` is WLAN-allocated and
    // owned by the guard below.
    let r = unsafe { WlanQueryInterface(c.0, guid, op, None, &mut size, &mut data, None) };
    let mem = WlanMem(data);
    if r != 0 || mem.0.is_null() || (size as usize) < size_of::<T>() {
        return None;
    }
    // SAFETY: the buffer holds at least one T (size checked above); read unaligned to be safe.
    Some(unsafe { std::ptr::read_unaligned(mem.0 as *const T) })
}

/// Centre frequency (MHz) of the access point with this BSSID.
fn bss_frequency(c: &Client, guid: &GUID, attrs: &WLAN_CONNECTION_ATTRIBUTES) -> Option<u32> {
    let a = &attrs.wlanAssociationAttributes;
    let mut list: *mut WLAN_BSS_LIST = std::ptr::null_mut();
    // SAFETY: valid handle, GUID and SSID; this reads the WLAN service's cached scan results
    // (no new scan). The list is owned by the guard below.
    let r = unsafe {
        WlanGetNetworkBssList(
            c.0,
            guid,
            Some(&a.dot11Ssid),
            dot11_BSS_type_infrastructure,
            attrs.wlanSecurityAttributes.bSecurityEnabled.as_bool(),
            None,
            &mut list,
        )
    };
    let mem = WlanMem(list);
    if r != 0 || mem.0.is_null() {
        return None;
    }
    // SAFETY: the list holds `dwNumberOfItems` entries starting at `wlanBssEntries`.
    let entries = unsafe {
        std::slice::from_raw_parts(
            (*mem.0).wlanBssEntries.as_ptr(),
            (*mem.0).dwNumberOfItems as usize,
        )
    };
    entries
        .iter()
        .find(|e| e.dot11Bssid == a.dot11Bssid)
        .map(|e| e.ulChCenterFrequency / 1000)
        .filter(|&f| f > 0)
}

/// Connected Wi-Fi interfaces: (adapter GUID "{...}", details).
pub fn connections() -> Vec<(String, Wifi)> {
    let mut version = 0u32;
    let mut handle = HANDLE::default();
    // SAFETY: valid out pointers; the handle is owned by `Client`.
    if unsafe { WlanOpenHandle(2, None, &mut version, &mut handle) } != 0 {
        return Vec::new();
    }
    let client = Client(handle);
    let mut list: *mut WLAN_INTERFACE_INFO_LIST = std::ptr::null_mut();
    // SAFETY: valid handle; the list is owned by the guard below.
    let r = unsafe { WlanEnumInterfaces(client.0, None, &mut list) };
    let mem = WlanMem(list);
    if r != 0 || mem.0.is_null() {
        return Vec::new();
    }
    // SAFETY: the list holds `dwNumberOfItems` entries starting at `InterfaceInfo`.
    let infos = unsafe {
        std::slice::from_raw_parts(
            (*mem.0).InterfaceInfo.as_ptr(),
            (*mem.0).dwNumberOfItems as usize,
        )
    };
    let mut out = Vec::new();
    for i in infos {
        if i.isState != wlan_interface_state_connected {
            continue;
        }
        let g = i.InterfaceGuid;
        // Fails with access denied when Windows' location permission is off (Windows 11 24H2+).
        let attrs: Option<WLAN_CONNECTION_ATTRIBUTES> =
            query(&client, &g, wlan_intf_opcode_current_connection);
        let channel: Option<u32> = query(&client, &g, wlan_intf_opcode_channel_number);
        let rssi: Option<i32> = query(&client, &g, wlan_intf_opcode_rssi);
        let mut w = Wifi {
            channel: channel.filter(|&c| c > 0),
            rssi_dbm: rssi.filter(|&r| r < 0),
            ..Default::default()
        };
        if let Some(attrs) = &attrs {
            let a = &attrs.wlanAssociationAttributes;
            let len = (a.dot11Ssid.uSSIDLength as usize).min(a.dot11Ssid.ucSSID.len());
            w.ssid = Some(String::from_utf8_lossy(&a.dot11Ssid.ucSSID[..len]).into_owned())
                .filter(|s| !s.is_empty());
            w.signal_pct = Some(a.wlanSignalQuality.min(100));
            w.phy = phy_name(a.dot11PhyType.0);
            w.rx_mbps = Some(a.ulRxRate / 1000).filter(|&r| r > 0);
            w.tx_mbps = Some(a.ulTxRate / 1000).filter(|&r| r > 0);
            w.freq_mhz = bss_frequency(&client, &g, attrs);
        }
        w.band = w
            .freq_mhz
            .and_then(Band::from_mhz)
            .or_else(|| w.channel.and_then(Band::from_channel));
        if w.channel.is_none() {
            w.channel = w.freq_mhz.and_then(channel_from_mhz);
        }
        out.push((guid_braced(&g), w));
    }
    out
}
