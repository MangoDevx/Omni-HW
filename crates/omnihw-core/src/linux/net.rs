//! Network interfaces from `/sys/class/net` (and udev's database for adapter names).

use super::Root;
use super::block::udev_props;
use crate::checkup::NicLink;

const NET: &str = "/sys/class/net";
/// `ARPHRD_ETHER`.
const TYPE_ETHER: u64 = 1;

/// The adapter's name from the PCI/USB id database ("Realtek RTL8125 2.5GbE Controller"), if
/// udev has it.
pub fn adapter_model(root: &Root, iface: &str) -> Option<String> {
    let index = root.read(&format!("{NET}/{iface}/ifindex"))?;
    let props = udev_props(&root.read(&format!("/run/udev/data/n{index}"))?);
    let get = |k: &str| {
        props
            .iter()
            .find(|(key, v)| key == k && !v.is_empty())
            .map(|(_, v)| v.clone())
    };
    let model = get("ID_MODEL_FROM_DATABASE")?;
    Some(match get("ID_VENDOR_FROM_DATABASE") {
        Some(vendor) if !model.starts_with(&vendor) => format!("{vendor} {model}"),
        _ => model,
    })
}

/// A physical Ethernet interface: backed by a device, Ethernet type, not Wi-Fi.
fn is_wired(root: &Root, iface: &str) -> bool {
    let p = format!("{NET}/{iface}");
    root.exists(&format!("{p}/device"))
        && root.read_u64(&format!("{p}/type")) == Some(TYPE_ETHER)
        && !root.exists(&format!("{p}/wireless"))
        && !root.exists(&format!("{p}/phy80211"))
}

/// Connected wired adapters with their negotiated speed.
pub fn wired_links(root: &Root) -> Vec<NicLink> {
    root.list(NET)
        .into_iter()
        .filter(|i| is_wired(root, i))
        .filter(|i| root.read(&format!("{NET}/{i}/operstate")).as_deref() == Some("up"))
        .filter_map(|i| {
            // Mbps; -1 (or unreadable) when the link speed isn't known.
            let mbps = root.read_i64(&format!("{NET}/{i}/speed"))?;
            let name = match adapter_model(root, &i) {
                Some(m) => format!("{i} ({m})"),
                None => i.clone(),
            };
            (mbps > 0).then(|| NicLink {
                name,
                speed_bps: mbps as u64 * 1_000_000,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::linux::Fixture;

    #[test]
    fn wired() {
        let f = Fixture::new();
        f.file("sys/class/net/enp5s0/type", "1\n")
            .file("sys/class/net/enp5s0/operstate", "up\n")
            .file("sys/class/net/enp5s0/speed", "1000\n")
            .file("sys/class/net/enp5s0/ifindex", "2\n")
            .dir("sys/class/net/enp5s0/device")
            .file(
                "run/udev/data/n2",
                "E:ID_VENDOR_FROM_DATABASE=Realtek Semiconductor Co., Ltd.\nE:ID_MODEL_FROM_DATABASE=RTL8125 2.5GbE Controller\n",
            )
            // Wi-Fi, a virtual bridge, and a cable that's unplugged.
            .file("sys/class/net/wlp6s0/type", "1\n")
            .file("sys/class/net/wlp6s0/operstate", "up\n")
            .dir("sys/class/net/wlp6s0/device")
            .dir("sys/class/net/wlp6s0/wireless")
            .file("sys/class/net/docker0/type", "1\n")
            .file("sys/class/net/docker0/operstate", "up\n")
            .file("sys/class/net/eno1/type", "1\n")
            .file("sys/class/net/eno1/operstate", "down\n")
            .file("sys/class/net/eno1/speed", "-1\n")
            .dir("sys/class/net/eno1/device");
        let links = wired_links(&f.root());
        assert_eq!(
            links,
            vec![NicLink {
                name: "enp5s0 (Realtek Semiconductor Co., Ltd. RTL8125 2.5GbE Controller)".into(),
                speed_bps: 1_000_000_000,
            }]
        );
    }
}
