# Linux packaging

- `omnihw.desktop`: menu entry. Install to `~/.local/share/applications/` (or
  `/usr/share/applications/`). `StartupWMClass` matches the app id the window sets, so the
  dock groups the window under this entry.
- `omnihw.png`: 256x256 icon, converted from `Assets/Brand/OmniHardwareIcon.ico`. Install to
  `~/.local/share/icons/hicolor/256x256/apps/omnihw.png`.

Manual install of a release build:

```sh
cargo build --release
install -Dm755 target/release/omnihw ~/.local/bin/omnihw
install -Dm644 packaging/linux/omnihw.desktop ~/.local/share/applications/omnihw.desktop
install -Dm644 packaging/linux/omnihw.png ~/.local/share/icons/hicolor/256x256/apps/omnihw.png
```

Build needs the X11/xkb headers eframe links against (Debian/Ubuntu:
`libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev libxkbcommon-dev`).

Running as root (e.g. `sudo -E omnihw`) adds the full SMBIOS tables (memory modules, CPU
socket) and the RAPL CPU power counters; everything else works as a normal user.
