//! Display adapters from `/sys/class/drm`, named via `pci.ids`.

use super::{Root, key_value, parse_int, pciids};
use crate::gpu::{Display, GpuAdapter, pci_vendor_name};

const DRM: &str = "/sys/class/drm";

/// PCI slot "0000:03:00.0" packed into one number (domain, bus, device, function); it stands in
/// for the Windows adapter LUID as the GPU's stable id.
pub fn slot_id(slot: &str) -> Option<u64> {
    let (domain, rest) = slot.split_once(':')?;
    let (bus, rest) = rest.split_once(':')?;
    let (dev, func) = rest.split_once('.')?;
    let h = |s: &str| u64::from_str_radix(s, 16).ok();
    Some((h(domain)? << 16) | (h(bus)? << 8) | (h(dev)? << 3) | h(func)?)
}

/// "1002:73BF" -> (0x1002, 0x73BF)
fn id_pair(s: &str) -> Option<(u32, u32)> {
    let (a, b) = s.split_once(':')?;
    Some((
        u32::from_str_radix(a, 16).ok()?,
        u32::from_str_radix(b, 16).ok()?,
    ))
}

/// The `card0`, `card1`, ... nodes (not their connectors like `card0-DP-1`).
pub fn cards(root: &Root) -> Vec<String> {
    root.list(DRM)
        .into_iter()
        .filter(|n| {
            n.strip_prefix("card")
                .is_some_and(|d| d.parse::<u32>().is_ok())
        })
        .collect()
}

/// Connected outputs of `card`, with their preferred (first listed) mode.
fn displays(root: &Root, card: &str) -> Vec<Display> {
    let prefix = format!("{card}-");
    root.list(DRM)
        .into_iter()
        .filter_map(|n| {
            let output = n.strip_prefix(&prefix)?.to_string();
            let p = format!("{DRM}/{n}");
            if root.read(&format!("{p}/status")).as_deref() != Some("connected") {
                return None;
            }
            let modes = root.read(&format!("{p}/modes")).unwrap_or_default();
            let (w, h) = modes.lines().next()?.split_once('x')?;
            // Interlaced modes end in "i".
            let h = h.trim_end_matches(|c: char| !c.is_ascii_digit());
            Some(Display {
                name: output,
                width: w.parse().ok()?,
                height: h.parse().ok()?,
            })
        })
        .collect()
}

/// One adapter from `card`'s PCI device; `None` for non-PCI cards (e.g. simpledrm).
pub fn adapter(root: &Root, card: &str) -> Option<GpuAdapter> {
    let dev = format!("{DRM}/{card}/device");
    let uevent = root.read(&format!("{dev}/uevent"))?;
    let (vendor_id, device_id) = id_pair(&key_value(&uevent, "PCI_ID")?)?;
    let subsys_id = key_value(&uevent, "PCI_SUBSYS_ID")
        .and_then(|s| id_pair(&s))
        .map(|(v, d)| (d << 16) | v)
        .unwrap_or(0);
    let slot = key_value(&uevent, "PCI_SLOT_NAME").unwrap_or_default();
    let driver = key_value(&uevent, "DRIVER").unwrap_or_default();
    let driver_name = driver.clone();
    let vendor = pci_vendor_name(vendor_id).unwrap_or("Unknown");
    let name = root
        .read_nonempty(&format!("{dev}/product_name"))
        .or_else(|| {
            let raw = pciids::lookup(root, vendor_id, device_id)?;
            Some(format!("{vendor} {}", pciids::marketing_name(&raw)))
        })
        .unwrap_or_else(|| format!("{vendor} GPU {device_id:04X}"));
    let driver_version = (!driver.is_empty())
        .then(|| root.read_nonempty(&format!("/sys/module/{driver}/version")))
        .flatten()
        .map(|v| format!("{driver} {v}"))
        .unwrap_or(driver);
    let mut g = GpuAdapter {
        name,
        vendor_id,
        device_id,
        subsys_id,
        revision: root
            .read(&format!("{dev}/revision"))
            .and_then(|r| parse_int(&r))
            .unwrap_or(0) as u32,
        // amdgpu reports its memory pools; other drivers leave these to the vendor library.
        dedicated_vram: root
            .read_u64(&format!("{dev}/mem_info_vram_total"))
            .unwrap_or(0),
        shared_memory: root
            .read_u64(&format!("{dev}/mem_info_gtt_total"))
            .unwrap_or(0),
        luid: slot_id(&slot).unwrap_or(0),
        driver_version,
        nvml: None,
        displays: displays(root, card),
        driver_date: None,
        vram_bar_bytes: None,
    };
    if is_discrete(&g, &driver_name) {
        g.vram_bar_bytes = root
            .read(&format!("{dev}/resource"))
            .and_then(|r| largest_prefetchable_bar(&r));
    }
    Some(g)
}

/// Discrete cards, whose VRAM aperture is what Resizable BAR resizes: amdgpu with dedicated
/// VRAM (APUs only have a small carve-out), and Intel Arc. NVIDIA comes from NVML.
fn is_discrete(g: &GpuAdapter, driver: &str) -> bool {
    match driver {
        "amdgpu" => g.dedicated_vram >= 2 << 30,
        "i915" | "xe" => g.supports_rebar() == Some(true),
        _ => false,
    }
}

/// Largest prefetchable memory BAR in a sysfs PCI `resource` file (one "start end flags" line
/// per region; the first six are the BARs).
pub fn largest_prefetchable_bar(resource: &str) -> Option<u64> {
    const MEM: u64 = 0x200;
    const PREFETCH: u64 = 0x2000;
    resource
        .lines()
        .take(6)
        .filter_map(|l| {
            let mut it = l.split_whitespace().map(parse_int);
            let (start, end, flags) = (it.next()??, it.next()??, it.next()??);
            (flags & MEM != 0 && flags & PREFETCH != 0 && end > start).then(|| end - start + 1)
        })
        .max()
}

pub fn gpu_adapters(root: &Root) -> Vec<GpuAdapter> {
    let mut out: Vec<GpuAdapter> = Vec::new();
    for card in cards(root) {
        if let Some(a) = adapter(root, &card) {
            // One PCI device can expose several cards; keep the first.
            if !out.iter().any(|o| o.luid == a.luid) {
                out.push(a);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::linux::Fixture;

    #[test]
    fn slots() {
        assert_eq!(slot_id("0000:03:00.0"), Some(0x0300));
        assert_eq!(slot_id("0001:65:00.1"), Some(0x1_6501));
        assert_ne!(slot_id("0000:00:02.0"), slot_id("0000:00:01.0"));
        assert_eq!(slot_id("junk"), None);
    }

    #[test]
    fn amd_card() {
        let f = Fixture::new();
        f.file(
            "usr/share/hwdata/pci.ids",
            "1002  AMD\n\t73bf  Navi 21 [Radeon RX 6800/6800 XT / 6900 XT]\n",
        )
        .file(
            "sys/class/drm/card1/device/uevent",
            "DRIVER=amdgpu\nPCI_CLASS=30000\nPCI_ID=1002:73BF\nPCI_SUBSYS_ID=1043:04F2\nPCI_SLOT_NAME=0000:03:00.0\n",
        )
        .file("sys/class/drm/card1/device/revision", "0xc1\n")
        .file(
            "sys/class/drm/card1/device/resource",
            "0x0000007800000000 0x0000007bffffffff 0x000000000014220c\n",
        )
        .file(
            "sys/class/drm/card1/device/mem_info_vram_total",
            "17163091968\n",
        )
        .file("sys/class/drm/card1-DP-1/status", "connected\n")
        .file("sys/class/drm/card1-DP-1/modes", "2560x1440\n1920x1080\n")
        .file("sys/class/drm/card1-HDMI-A-1/status", "disconnected\n")
        .file("sys/class/drm/card0/device/uevent", "DRIVER=simple-framebuffer\n")
        .dir("sys/class/drm/renderD128");
        let gpus = gpu_adapters(&f.root());
        assert_eq!(gpus.len(), 1);
        let g = &gpus[0];
        assert_eq!(g.name, "AMD Radeon RX 6800/6800 XT / 6900 XT");
        assert_eq!(
            (g.vendor_id, g.device_id, g.revision),
            (0x1002, 0x73BF, 0xC1)
        );
        assert_eq!(g.board_vendor(), Some("ASUS"));
        assert_eq!(g.subsys_id >> 16, 0x04F2);
        assert_eq!(g.dedicated_vram, 17163091968);
        assert_eq!(g.driver_version, "amdgpu");
        assert_eq!(g.luid, 0x0300);
        assert_eq!(g.displays.len(), 1);
        assert_eq!(g.displays[0].summary(), "DP-1 (2560x1440)");
        assert_eq!(g.vram_bar_bytes, Some(16 << 30));
        assert_eq!(g.rebar().label(), "On");
    }

    #[test]
    fn bars() {
        // BAR0 16 GiB prefetchable (VRAM), BAR2 256 MiB prefetchable, BAR4 I/O, BAR5 registers.
        let r = "0x0000007800000000 0x0000007bffffffff 0x000000000014220c\n\
                 0x0000000000000000 0x0000000000000000 0x0000000000000000\n\
                 0x0000007c00000000 0x0000007c0fffffff 0x000000000014220c\n\
                 0x0000000000000000 0x0000000000000000 0x0000000000000000\n\
                 0x000000000000e000 0x000000000000e0ff 0x0000000000040101\n\
                 0x00000000fcc00000 0x00000000fcc7ffff 0x0000000000040200\n\
                 0x00000000fcc80000 0x00000000fcc9ffff 0x000000000004e200\n";
        assert_eq!(largest_prefetchable_bar(r), Some(16 << 30));
        let legacy = "0x00000000e0000000 0x00000000efffffff 0x000000000014220c\n";
        assert_eq!(largest_prefetchable_bar(legacy), Some(256 << 20));
        assert_eq!(largest_prefetchable_bar(""), None);
    }

    #[test]
    fn unknown_device_name() {
        let f = Fixture::new();
        f.file(
            "sys/class/drm/card0/device/uevent",
            "DRIVER=nvidia\nPCI_ID=10DE:2684\nPCI_SLOT_NAME=0000:01:00.0\n",
        )
        .file("sys/module/nvidia/version", "560.35.03\n");
        let g = &gpu_adapters(&f.root())[0];
        assert_eq!(g.name, "NVIDIA GPU 2684");
        assert_eq!(g.driver_version, "nvidia 560.35.03");
    }
}
