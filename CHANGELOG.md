# Changelog

User-visible changes in each Omni-HW release. The release workflow publishes the section for the tagged version as the GitHub release notes, which the in-app **What's new** button opens.

## Unreleased

### New
- **LAN Security Report** (Network → LAN Security Report): scans your own local network for devices and common security problems — clear-text logins (Telnet, FTP), exposed admin pages, unauthenticated databases, remote-desktop ports and more — and lists ranked findings with plain explanations and links to fix steps. It stays on your local network, runs only when you click, and changes nothing.
- **Memory test** (Memory page): measures memory bandwidth and latency in a few seconds (AIDA64-style streaming reads, on all cores and on a single core) and compares them with what your RAM speed and channel count should deliver.
- **Network is its own category** in the grouped navigation bar, holding the Network page and the LAN Security Report.
- **Disk speed test** (Storage page): measures sequential and 4K random read/write speed with a temporary file and compares it with what the drive and its link should deliver.
- **Extra sensors** (optional, Settings → Extra sensors): per-core CPU temperatures, CPU voltage, per-core power on Ryzen, and motherboard fan speeds, temperatures and voltages, read through the signed PawnIO driver by a small read-only Omni-HW sensor service. Omni-HW itself doesn't need administrator rights for them, and works fully without them.
- **CPU temperature** on the Overview and CPU pages, and a matching Health Check, when extra sensors are set up.

### Improved
- Health Check redesigned as a list of issues with a details pane, plus category tiles to filter it.
- Up and Down arrow keys move through the Health Check list.

## 0.3.0

### New
- **Network page**: every adapter's driver, link speed, addresses, gateway, DNS, DHCP lease, MTU, Wi-Fi details and advanced driver settings.
- **Network changes**: enable, disable or rename an adapter, set its IP address, DNS servers, MTU or driver settings, release or renew DHCP, flush DNS and reset Winsock or TCP/IP, each after a plain-language confirmation.
- **Connection tests**: local tests of your gateway and DNS servers, plus internet tests that run only when you click their button.
- **Power page**: power plan, sleep, hibernation and Fast Startup, wake timers, the devices that can wake the PC and what is keeping it awake, with matching Health Check findings.
- **Guided fixes**: Health Check findings come with numbered steps, search, Settings and website links, BIOS menu paths and a Restart into UEFI button.
- **New Health Check findings**: refresh rate, missing or leftover GPU drivers, Device Manager problems, hardware errors and blue screens, NVMe/SATA/Ethernet link speed, Secure Boot and TPM, activation, startup apps and network problems.
- **Dismiss findings** you don't care about, and restore them from the Dismissed list.
- **Retro theme** with an early-2000s look.
- **Show gaming suggestions** setting to hide the Gaming category.
- **What's new** button next to the update check.

### Improved
- Health Check is grouped by category, with Expand all, Collapse all and Expand issues only.
- Arrow keys move between pages in both navigation bar styles.
- Browse button for choosing the recordings folder.

## 0.2.0

### New
- **Check for updates** (Settings → Updates): compares your version with the latest release on GitHub and links to the download page when a newer one exists.
- **Check for updates when Omni-HW starts**: an optional toggle, off by default.

## 0.1.0

### New
- First release: Overview dashboard, Health Check, hardware details, live sensors, recording and analysis, alerts and reports.
