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
            "Clocks: live average core speed, base clock, bus speed, multiplier and package power.",
            "Caches and topology: cores (performance and efficiency cores on hybrid CPUs), threads, NUMA nodes.",
        ],
    );
    details(
        ui,
        "cpu",
        &[
            "Read with the CPUID instruction and Windows' processor topology. Live clocks are Windows performance counters (% Processor Performance x base clock).",
            "VT-x may be missing from the instruction list when Hyper-V or Memory Integrity is on; Windows hides it from programs.",
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
        "Memory type, installed and usable size, slots used, configured speed and live load, plus a table of every module with its size, rated and configured speed, ranks, maker and part number.",
    );
    h(ui, "GPU");
    bullets(
        ui,
        &[
            "Name, chip, architecture, process, card maker, PCI ids and revision.",
            "PCIe link (current vs maximum), BIOS version, driver version and date (NVIDIA shows more detail).",
            "Resizable BAR: lets the CPU access all of the graphics memory at once, which helps many modern games. Shown as On/Off on NVIDIA cards; Unknown on AMD and Intel for now.",
            "Displays: which monitors are connected to this card.",
            "Memory and limits (VRAM, bus width, CUDA cores, max clocks, power limit) and the card's live sensors.",
            "If you have more than one graphics adapter, pick it from the Adapter list.",
        ],
    );
    details(
        ui,
        "gpu",
        &[
            "Basic adapter information comes from DXGI and works for every vendor. Temperatures, clocks, power, fan, PCIe and ReBAR come from NVIDIA's NVML library, which ships with the NVIDIA driver.",
            "GPU load and VRAM use for any vendor come from the same Windows counters Task Manager uses.",
            "Virtual display adapters (used by streaming and virtual monitor tools) are hidden so your real card isn't listed twice.",
        ],
    );
    h(ui, "Storage");
    bullets(
        ui,
        &[
            "Each physical drive: model, type (NVMe SSD, SSD, HDD), interface, capacity, firmware and serial number.",
            "Volumes: each drive letter with a free-space bar.",
            "NVMe health: health %, spare capacity, temperature, data written and read, power-on hours, power cycles, unsafe shutdowns, media errors and warnings.",
            "SATA health (SMART): reallocated, pending and uncorrectable sectors, SSD life left, temperature and the full attribute table. Needs administrator rights.",
            "Live sensors: temperature, activity and read/write speed.",
        ],
    );
}

pub(super) fn network(ui: &mut Ui) {
    p(
        ui,
        "The Network page (Hardware > Network) lists every adapter Windows shows in Network Connections. It works fully offline: \
         details are read from Windows, and nothing is sent anywhere unless you click Run internet tests.",
    );
    h(ui, "What it shows");
    bullets(
        ui,
        &[
            "Status (connected, disconnected, disabled), adapter model, type (Ethernet, Wi-Fi, virtual), MAC address and driver version and date.",
            "Link speed and duplex, plus the fastest speed the adapter supports when it's linked slower.",
            "IPv4 and IPv6 addresses (with DHCP lease time and address conflicts), gateway, DNS servers, DHCP on/off and server, MTU and DNS suffix.",
            "Wi-Fi: network name (SSID), band (2.4 / 5 / 6 GHz), channel, signal quality and strength, Wi-Fi standard and link rates.",
            "Advanced driver properties (Speed & Duplex, Energy-Efficient Ethernet, power saving, jumbo frames, interrupt moderation and so on), as in Device Manager.",
        ],
    );
    h(ui, "Changing settings");
    bullets(
        ui,
        &[
            "Per adapter: Enable / Disable, Rename, IP address (automatic DHCP or a fixed IPv4 address, mask and gateway), DNS servers (automatic or your own IPv4 servers), MTU, Release and Renew.",
            "Advanced driver properties with a list of values can be changed from their drop-down; the adapter restarts, so the connection drops for a few seconds. Free-form values are shown read-only (change them in Device Manager).",
            "Repair tools: Flush DNS cache, Reset Winsock and Reset TCP/IP. The two resets need a restart of Windows.",
            "Every change asks for confirmation first. Except flushing the DNS cache, they need administrator rights: Windows asks each time, and a short-lived copy of Omni-HW makes just that one change and exits.",
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
            "Addresses and link state come from IP Helper (GetAdaptersAddresses, GetIfEntry2); Wi-Fi from the WLAN API; driver details and advanced properties from WMI (MSFT_NetAdapter). They refresh every 5 seconds while the page is open and every minute otherwise, on the background thread.",
            "On Windows 11 24H2 and later the Wi-Fi network name is hidden unless location access for apps is on (Settings > Privacy & security > Location).",
            "Changes use WMI (MSFT_NetAdapter, Win32_NetworkAdapterConfiguration, MSFT_NetIPInterface); the resets run netsh and the DNS flush runs ipconfig, without a console window.",
        ],
    );
}

pub(super) fn power(ui: &mut Ui) {
    p(
        ui,
        "The Power page (Hardware > Power) shows and changes Windows power settings in one place. Changes apply to the \
         active power plan right away.",
    );
    h(ui, "What it shows");
    bullets(
        ui,
        &[
            "Power plan: every plan on the PC (Balanced, High performance, Ultimate Performance, vendor plans). Click one to switch.",
            "Sleep and display: when the screen turns off, the PC sleeps or hibernates and hard disks turn off, plugged in and (on laptops) on battery.",
            "Power buttons and lid, minimum and maximum processor state, USB selective suspend, PCI Express power saving, wake timers, Wi-Fi power saving and the battery saver level.",
            "Startup and standby: Fast Startup, hibernation and whether the PC uses Modern Standby.",
            "Network and Bluetooth adapters: whether Windows may turn them off to save power, and Energy-Efficient Ethernet.",
            "Waking: the last wake source, the wake history of the last 2 weeks, devices allowed to wake the PC (with Disarm) and active wake timers.",
            "What's keeping the PC awake: apps and drivers with active power requests (like powercfg /requests).",
        ],
    );
    h(ui, "Changing settings");
    bullets(
        ui,
        &[
            "Pick a new value from a drop-down or click a button; the page refreshes when the change is done and the status bar says whether it worked.",
            "Power plan values and plan switching don't need administrator rights. Fast Startup, adapter power saving, Energy-Efficient Ethernet and wake devices do: Windows shows its permission prompt.",
            "Changes that briefly disconnect a network adapter or change how Windows shuts down ask for confirmation first.",
            "Power requests and wake timers need administrator rights to read: click Check now.",
        ],
    );
    details(
        ui,
        "power",
        &[
            "Settings are read with Windows' power APIs when Omni-HW starts, when you open the page or click Refresh, and plan settings every 30 seconds in the background.",
            "Wake history comes from the System event log (Power-Troubleshooter). Adapter power saving is Device Manager's \"Allow the computer to turn off this device\" setting. Power requests and wake timers come from powercfg.",
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
            "CPU clock per core is the fastest of its threads. GPU 'Load (busiest engine)' is how Task Manager measures GPU usage; 'Core Load' (NVIDIA) is what GPU-Z shows.",
            "Some sensors only exist on some systems: package power needs a CPU that reports it to Windows; battery sensors only appear on laptops.",
        ],
    );
}
