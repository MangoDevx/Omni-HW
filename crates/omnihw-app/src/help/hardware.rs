//! Help pages: the hardware tabs, Power and Sensors.

use super::{bullets, details, h, p};
use egui::Ui;

pub(super) fn hardware(ui: &mut Ui) {
    p(
        ui,
        "These tabs show detailed, mostly fixed information about each part, read once when Omni-HW starts.",
    );
    h(ui, "CPU");
    bullets(
        ui,
        &[
            "Name, code name, microarchitecture, manufacturing process and socket.",
            "Family / model / stepping and the instruction sets the CPU supports (AVX2, AVX-512 and so on).",
            "Clocks: live average core speed, base clock, bus speed, multiplier and package power (plus temperature and core voltage with the extra sensors).",
            "Caches and topology: cores (performance and efficiency cores on hybrid CPUs), threads, NUMA nodes.",
        ],
    );
    details(
        ui,
        "cpu",
        &[
            "Read with the CPUID instruction and the kernel's CPU topology in /sys/devices/system/cpu. Live clocks come from cpufreq (scaling_cur_freq).",
            "Package power comes from RAPL (powercap), which recent kernels only let root read: use Read as root in Settings.",
            "VT-x / AMD-V may be missing from the instruction list inside a virtual machine.",
        ],
    );
    h(ui, "Caches");
    p(
        ui,
        "Every cache level with its size, how many copies exist (e.g. one per core), associativity and line size, plus the raw CPUID cache descriptors.",
    );
    h(ui, "Mainboard");
    p(
        ui,
        "Motherboard maker and model, system maker and product, BIOS vendor, version and date, SMBIOS version and CPU socket.",
    );
    h(ui, "Memory");
    p(
        ui,
        "Memory type, installed and usable size, slots used, configured speed and live load, plus a table of every module with its size, rated and configured speed, ranks, maker and part number (the module table needs Read as root, since only root can read the SMBIOS tables). The quick memory test measures bandwidth and latency; any hardware errors (machine checks, EDAC) the kernel logs while it runs are shown under the result, since they mean the memory or CPU is unstable.",
    );
    h(ui, "GPU");
    bullets(
        ui,
        &[
            "Name, chip, architecture, process, card maker, PCI ids and revision.",
            "PCIe link (current vs maximum), BIOS version, driver version and date (NVIDIA shows more detail).",
            "Resizable BAR: lets the CPU access all of the graphics memory at once, which helps many modern games. NVIDIA cards report it through NVML; on AMD and Intel it's On when the card's PCI memory window covers its whole VRAM.",
            "Displays: which monitors are connected to this card.",
            "Memory and limits (VRAM, bus width, CUDA cores, max clocks, power limit) and the card's live sensors.",
            "If you have more than one graphics adapter, pick it from the Adapter list.",
        ],
    );
    details(
        ui,
        "gpu",
        &[
            "Basic adapter information comes from /sys/class/drm and the PCI ID list, and works for every vendor. On NVIDIA cards with NVIDIA's driver, temperatures, clocks, power, fan, PCIe and ReBAR come from its NVML library (libnvidia-ml).",
            "AMD (amdgpu) and Intel (i915/xe) load, VRAM use, clocks and temperatures come from the driver's sysfs and hwmon files.",
            "Cards on the nouveau driver show basic information only.",
        ],
    );
    h(ui, "Storage");
    bullets(
        ui,
        &[
            "Each physical drive: model, type (NVMe SSD, SSD, HDD), interface, capacity, firmware and serial number.",
            "File systems: each mounted file system with its type and a free-space bar.",
            "NVMe health: health %, spare capacity, temperature, data written and read, power-on hours, power cycles, unsafe shutdowns, media errors and warnings.",
            "SATA health (SMART): reallocated, pending and uncorrectable sectors, SSD life left, temperature and the full attribute table. Needs Read as root.",
            "Live sensors: temperature, activity and read/write speed.",
            "Speed test: sequential and 4K random read/write speed; any hardware errors (e.g. PCIe errors) the kernel logs while it runs are shown under the result.",
        ],
    );
}

pub(super) fn network(ui: &mut Ui) {
    p(
        ui,
        "The Network page lists every network interface except loopback (and the per-container veth \
         interfaces Docker creates). It works fully offline: details are read from the kernel and \
         NetworkManager, and nothing is sent anywhere unless you click Run internet tests.",
    );
    h(ui, "What it shows");
    bullets(
        ui,
        &[
            "Status (connected, disconnected, down), adapter model, type (Ethernet, Wi-Fi, tunnel/VPN, virtual), MAC address and kernel driver.",
            "Link speed and duplex, plus the fastest speed the adapter supports when it's linked slower.",
            "IPv4 and IPv6 addresses (with DHCP lease time and duplicate addresses), gateway, DNS servers, DHCP on/off and server, MTU and DNS search domain.",
            "Wi-Fi: network name (SSID), band (2.4 / 5 / 6 GHz), channel, signal quality and strength, Wi-Fi standard and link rates.",
        ],
    );
    h(ui, "Changing settings");
    bullets(
        ui,
        &[
            "Per interface: Connect / Disconnect, IP address (automatic DHCP or a fixed IPv4 address, mask and gateway), DNS servers (automatic or your own IPv4 servers), MTU and Reconnect (renews the DHCP lease).",
            "Changes are saved in the interface's active NetworkManager connection, the same place the system's network settings use, so they last across restarts.",
            "Flush DNS cache clears systemd-resolved's cache.",
            "Every change asks for confirmation first. NetworkManager lets the signed-in user make them without a password; on some setups it asks for one.",
        ],
    );
    h(ui, "Tests");
    bullets(
        ui,
        &[
            "Local tests (run once when you open the page, and again with Run local tests): link up, a real IP address (169.254.x.x means no DHCP server answered), duplicate addresses, gateway reachable (4 pings), DNS servers on your local network answering, and link speed below what the adapter supports. They only contact your own router and local DNS servers.",
            &format!(
                "Internet tests run only when you click Run internet tests. They contact: {}. They report whether DNS and the web work (and spot captive portals) and the latency, jitter and packet loss of {} pings to each resolver.",
                omnihw_core::network::INTERNET_TARGETS,
                omnihw_core::network::INTERNET_PINGS
            ),
        ],
    );
    details(
        ui,
        "network",
        &[
            "Addresses and routes come from iproute2 (ip -j addr, ip -j route), link details from /sys/class/net, the fastest supported speed from the ethtool interface, DNS from resolvectl (or /etc/resolv.conf), Wi-Fi from iw and the DHCP server from nmcli. They refresh every 5 seconds while the page is open and every minute otherwise, on the background thread.",
            "Without the iw package installed, Wi-Fi shows the signal level only.",
            "Pings use unprivileged ICMP sockets, so no root is needed. Changes run nmcli; a page change to an interface NetworkManager doesn't manage shows NetworkManager's error.",
        ],
    );
}

pub(super) fn lan_security(ui: &mut Ui) {
    p(
        ui,
        "The LAN Security Report (Network > LAN Security Report) scans your own local network for \
         devices and common security problems. It runs only when you click Scan my network, stays \
         on your local network, and changes nothing: it identifies and explains issues, it does \
         not try to exploit them.",
    );
    h(ui, "What it does");
    bullets(
        ui,
        &[
            "Works out the private subnet of each connected adapter (for example 192.168.1.0/24) and refuses a range wider than a /22, so a scan stays bounded.",
            "Finds which addresses answer (a quick ping and the neighbour/ARP table), and labels each device with its MAC address, a best-effort maker, and its reverse-DNS name when it has one.",
            "Checks a short list of well-known TCP ports on each device and reads the greeting (banner) a service shows on connect. It never sends a password or anything beyond a bare request used to read the banner.",
            "Adds a read-only check of this PC itself: whether its Samba server still accepts the old, insecure SMBv1 protocol.",
        ],
    );
    h(ui, "Reading the report");
    bullets(
        ui,
        &[
            "Findings are ranked RISK (fix this), CHECK (worth tightening) and INFO (just so you know). Each one explains the problem in plain terms and links to fix steps you open in your browser.",
            "Typical RISK findings: Telnet or FTP (clear-text logins), VNC remote desktop, or an unauthenticated database (Redis, MongoDB) reachable on the network.",
            "Typical CHECK findings: a database or Remote Desktop exposed to the LAN, a router admin page over plain HTTP, or legacy NetBIOS sharing.",
            "The device list shows every device found; expand one to see its MAC address and open ports with their banners.",
        ],
    );
    details(
        ui,
        "lan_security",
        &[
            "Discovery uses one ICMP echo per address plus the IPv4 neighbour table (/proc/net/arp); reverse names use a PTR DNS query. Port probes are plain TCP connects from the standard library with short timeouts; banners are at most a couple of hundred printable characters.",
            "Nothing contacts the internet: every address is on your own private subnet. The scan runs on a background worker thread, so the app stays responsive, and a typical home network finishes in well under a minute.",
        ],
    );
}

pub(super) fn power(ui: &mut Ui) {
    p(
        ui,
        "The Power page (Hardware > Power) shows and changes the power settings Linux spreads over power-profiles-daemon, \
         cpufreq and sysfs, in one place.",
    );
    h(ui, "What it shows");
    bullets(
        ui,
        &[
            "Power profile: Power Saver, Balanced or Performance from power-profiles-daemon, and the firmware's platform profile on laptops that have one.",
            "CPU: the frequency driver (intel_pstate, amd-pstate, acpi-cpufreq) and its mode, the governor, the energy/performance preference and turbo boost.",
            "Power saving and sleep: the PCIe power saving policy (ASPM), USB autosuspend, the sleep mode (s2idle or deep) and the devices allowed to wake the PC.",
        ],
    );
    h(ui, "Changing settings");
    bullets(
        ui,
        &[
            "Pick a new value from a drop-down; the page refreshes when the change is done and the status bar says whether it worked.",
            "The power profile changes through power-profiles-daemon without a password. CPU settings need root: your desktop asks for your password (pkexec), and a short-lived copy of Omni-HW makes just that one change and exits.",
            "Changes last until the next restart. Tools such as TLP, auto-cpufreq or tuned may set their own values at startup; change them there to make a setting stick.",
        ],
    );
    details(
        ui,
        "power",
        &[
            "Settings are read when Omni-HW starts and when you open the page or click Refresh.",
            "The root helper only accepts the values the kernel lists as available for each setting.",
        ],
    );
}

pub(super) fn sensors(ui: &mut Ui) {
    h(ui, "What it shows");
    p(
        ui,
        "Every live reading Omni-HW collects, grouped by device (CPU clocks, CPU usage, memory, power, each GPU and drive). \
         For each sensor you see the current value and the minimum, maximum and average since Omni-HW started (or since \
         you last reset them).",
    );
    h(ui, "Using it");
    bullets(
        ui,
        &[
            "Reset min/max clears the statistics, e.g. right before starting a game or a stress test.",
            "Interval sets how often sensors are read: 250, 500, 1000 (default) or 2000 ms. Faster catches short spikes; slower uses less CPU.",
            "Click a group's header to collapse or expand it.",
        ],
    );
    details(
        ui,
        "sensors",
        &[
            "CPU clock per core is the fastest of its threads. CPU usage comes from /proc/stat, drive activity from /proc/diskstats.",
            "Temperatures, fans and voltages come from the kernel's hwmon drivers: coretemp or k10temp for the CPU, nvme and drivetemp for drives, and nct6775 or it87 for motherboard fans and voltages. A sensor chip without a loaded driver shows nothing; sensors-detect (lm-sensors) finds the right module.",
            "CPU package power comes from RAPL, which needs Read as root on recent kernels. Battery sensors only appear on laptops.",
            "Fan headers with nothing plugged in stay hidden.",
        ],
    );
}
