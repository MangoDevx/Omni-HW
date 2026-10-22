# Omni-HW for Linux

## Install from the release archive

Unpack `omnihw-<version>-linux-x86_64.tar.gz` and run `./omnihw` from the folder. To add it to
your application menu:

```sh
install -Dm755 omnihw ~/.local/bin/omnihw
install -Dm644 omnihw.desktop ~/.local/share/applications/omnihw.desktop
install -Dm644 omnihw.png ~/.local/share/icons/hicolor/256x256/apps/omnihw.png
```

`~/.local/bin` must be on your `PATH` for the menu entry to start it (it is on Ubuntu once the
folder exists and you've logged in again).

Settings live in `~/.config/omnihw`. "Launch at sign-in" adds `~/.config/autostart/omnihw.desktop`.

## Root access

Omni-HW runs as you. Memory module details (SMBIOS) and drive health (NVMe and SATA SMART) can
only be read by root: the "Read as root..." buttons (Memory, Storage, Settings) start a
short-lived copy of Omni-HW through `pkexec`, which asks for your password, prints that data and
exits. Don't run the whole app with `sudo`: GUI apps started as root often can't open a window
on Wayland.

## What it uses at runtime

- `pkexec` (polkit) for root access, `curl` for the update check, `xdg-open` to open reports and
  folders, `systemctl` for "Restart into UEFI". Each is optional: only that feature stops working
  without it.
- A notification daemon (every desktop has one) for alert notifications, and the XDG desktop
  portal for the folder chooser.
- NVIDIA's `libnvidia-ml.so.1` (comes with the driver) for NVIDIA GPU sensors.

## Building

```sh
cargo build --release
```

Needs the X11/xkb headers eframe links against (Debian/Ubuntu:
`libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev libxkbcommon-dev`).

## Files here

- `omnihw.desktop`: menu entry. `StartupWMClass` matches the app id the window sets, so the
  dock groups the window under this entry.
- `omnihw.png`: 256x256 icon, converted from `Assets/Brand/OmniHardwareIcon.ico`.
