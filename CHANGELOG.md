# Changelog

User-visible changes in each Omni-HW for Linux release. The release workflow publishes the section for the tagged version (`linux-v<version>`) as the GitHub release notes, which the in-app **What's new** button opens.

## Unreleased

### New
- **Omni-HW for Linux**: the first Linux version, shipped as a `.tar.gz` for x86-64 (built on Ubuntu 22.04).
- Hardware pages: CPU, caches, mainboard, memory, GPU and storage details from sysfs, procfs, SMBIOS and the PCI database, including NVMe drive health and PCIe links.
- Live sensors: CPU clocks, load, temperature and power (RAPL), motherboard fans and voltages (hwmon), memory, battery, drive temperature and I/O, AMD and Intel GPU sensors, and NVIDIA GPUs through NVML.
- **Read as root**: one click (with your password, through pkexec) adds memory module details and SATA/NVMe drive health that only root can read.
- **Event Log** page: kernel crashes and hangs, hardware errors, GPU hangs, disk problems, out-of-memory kills, app crashes and overheating from the system journal, in plain English.
- **Power** page: power profile (power-profiles-daemon), CPU governor, energy/performance preference, boost and platform profile, plus PCIe and USB power saving, the sleep mode and the devices that can wake the PC. Kernel settings ask for your password.
- Memory and disk tests warn when the kernel logged hardware errors while they ran.
- Health Check with Linux checks, including kernel crashes, hardware errors, GPU hangs, disk errors, out-of-memory kills and overheating from the system journal, pending restarts, kernel problems since boot, swap, the CPU frequency driver and governor, power profile, PCIe power saving, a missing NVIDIA driver, UEFI, Secure Boot and TPM, Resizable BAR on AMD, Intel and NVIDIA GPUs, wired link speed, DHCP/DNS problems and weak Wi-Fi.
- **Network** page: every interface with its link, addresses, gateway, DNS, DHCP and Wi-Fi details; connect/disconnect, IP address, DNS, MTU and reconnect through NetworkManager; DNS cache flush; and the local and internet tests.
- **LAN Security Report**: scan your own network for devices and risky open services, plus a check for SMBv1 in this PC's Samba server.
- Alert notifications on the desktop, launch at sign-in, reports in your Documents folder, and the update check.
