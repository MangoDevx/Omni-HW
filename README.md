# Omni-HW

All-in-one Windows hardware monitor written in Rust. Omni-HW combines what CPU-Z, GPU-Z and HWiNFO do in one fast app, then goes further: it explains what it finds, tells you how to fix common problems, alerts you, and records and analyzes sessions.

No kernel driver, no installer, a single executable.

## Features

- **Overview dashboard**: CPU, GPU, memory, storage and system on one page, with live numbers, history graphs, per-core tiles and heatmap colors (blue/orange/red) for load, temperatures, memory, VRAM and drive health.
- **Health Check**: 25 rule-based checks that flag common problems in plain language, with steps to fix them. Some examples:
  - Monitor plugged into the motherboard instead of the graphics card.
  - XMP/EXPO off, or RAM in single channel.
  - Outdated Intel 13th/14th Gen microcode.
  - GPU running at reduced PCIe width, or Resizable BAR off.
  - Drives nearly full, worn or running hot.
  - Pending restart, power-saver plan, BIOS age.
  - Gaming-only checks get their own section, and security-related settings are clearly flagged.
- **Hardware details** (CPU-Z / GPU-Z style):
  - CPU identification, codename and process, cores/threads (P/E cores), instruction sets and caches.
  - Mainboard and BIOS.
  - Memory modules (SMBIOS).
  - GPU: chip, VRAM, driver, PCIe link, Resizable BAR and connected displays.
  - Drives: NVMe and SATA SMART health, temperatures and volumes.
- **Sensors** (HWiNFO style): every live reading with min/max/average:
  - Per-core clocks and usage, CPU package power.
  - GPU load, temperature, clocks, power and fan (NVIDIA via NVML; load and VRAM for any vendor).
  - Memory, disk throughput and temperatures.
- **Recording**: record every sensor to CSV, chart a session, compare two sessions (before/after), and run **bottleneck analysis** to see whether a game was CPU-bound, GPU-bound or frame-capped.
- **Alerts**: "sensor above/below a limit for N seconds" rules. When one fires you get a Windows notification and a taskbar flash.
- **Reports**: save a self-contained HTML report or copy a plain-text one, with Health Check results first.
- **Quality of life**:
  - Grouped one-row navigation with Up/Down arrow-key paging.
  - Dark/light theme and UI scale.
  - Minimize to the system tray; launch at sign-in.
  - In-app feature guide (Info > Feature guide).

## Requirements

- Windows 10 or 11, x64.
- To build: Rust (stable, MSVC toolchain) and the Visual Studio C++ build tools with a Windows SDK.

## Build and run

```
cargo run -p omnihw-app --release
```

The executable is written to `target\release\omnihw.exe`.

Run the tests:

```
cargo test --workspace
```

### Command-line options

| Option | Effect |
|---|---|
| `--no-elevate` | Don't ask for administrator rights at start |
| `--autostart` | Used by the sign-in entry; never shows the administrator prompt |
| `--minimized` | Start minimized (to the tray if that setting is on) |

## Permissions and privacy

- Omni-HW uses only standard user-mode Windows APIs and the GPU driver's own monitoring library. It installs no driver or service.
- **Administrator rights** are only needed to read SATA drive health (SMART). If SATA drives are present, Omni-HW asks via UAC at start. Decline it and everything else still works, or turn the prompt off in Settings. NVMe health doesn't need it.
- Everything stays on your PC. Omni-HW makes no network connections.
- Files it writes:
  - Settings: `%APPDATA%\OmniHW\settings.ini`
  - Alert rules: `%APPDATA%\OmniHW\alerts.tsv`
  - Recordings and reports: Documents, or the folder you choose in Settings.
  - "Launch at sign-in" adds an `Omni-HW` value under `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`; turning it off removes it.

## Project layout

```
crates/
  omnihw-core/   hardware detection, parsers, sensors, diagnostics, recording and analysis
                 (no external dependencies; parsers are platform-agnostic and unit-tested)
  omnihw-app/    the eframe/egui desktop app
```

## Roadmap

Planned next: per-core CPU temperatures, fans and voltages through an optional signed-driver backend, and quick memory and disk self-tests.

## Trademarks

CPU-Z is a trademark of CPUID. GPU-Z is a trademark of TechPowerUp. HWiNFO is a trademark of Martin Malík (REALiX). Omni-HW is an independent project and is not affiliated with or endorsed by them.

## License

[MIT](LICENSE)
