//! Embeds the Windows resources into omnihw.exe: the app icon and version info (shown in
//! Explorer's Details tab and Task Manager). The version comes from Cargo.toml, so the resource
//! script is generated into OUT_DIR rather than kept by hand.
//!
//! The application manifest is not embedded here: gpui embeds its own (asInvoker, Windows 10
//! compatibility, per-monitor DPI awareness v2, Common Controls v6), and an executable can
//! hold only one.

use std::{env, fs, path::Path};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../Assets/Brand/OmniHardwareIcon.ico");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let out = env::var("OUT_DIR").expect("OUT_DIR");
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let num = |k: &str| {
        env::var(k)
            .ok()
            .and_then(|v| v.parse::<u16>().ok())
            .unwrap_or(0)
    };
    let (major, minor, patch) = (
        num("CARGO_PKG_VERSION_MAJOR"),
        num("CARGO_PKG_VERSION_MINOR"),
        num("CARGO_PKG_VERSION_PATCH"),
    );
    let version = env::var("CARGO_PKG_VERSION").unwrap_or_default();

    // Forward slashes keep the paths valid inside RC string literals (rc.exe and windres accept them).
    let rc_path = |p: &Path| p.display().to_string().replace('\\', "/");
    let icon = Path::new(&manifest_dir).join("../../Assets/Brand/OmniHardwareIcon.ico");

    let rc = format!(
        r#"#pragma code_page(65001)
// Application icon. Resource id 1 is what gpui gives the window and what the tray loads
// (tray.rs).
1 ICON "{icon}"

1 VERSIONINFO
FILEVERSION {major},{minor},{patch},0
PRODUCTVERSION {major},{minor},{patch},0
FILEFLAGSMASK 0x3fL
FILEFLAGS 0x0L
FILEOS 0x40004L
FILETYPE 0x1L
FILESUBTYPE 0x0L
BEGIN
    BLOCK "StringFileInfo"
    BEGIN
        BLOCK "040904b0"
        BEGIN
            VALUE "CompanyName", "Omni-HW"
            VALUE "FileDescription", "Omni-HW"
            VALUE "FileVersion", "{version}"
            VALUE "InternalName", "omnihw"
            VALUE "LegalCopyright", "Copyright (c) 2026 Omni-HW contributors. MIT License."
            VALUE "OriginalFilename", "omnihw.exe"
            VALUE "ProductName", "Omni-HW"
            VALUE "ProductVersion", "{version}"
        END
    END
    BLOCK "VarFileInfo"
    BEGIN
        VALUE "Translation", 0x409, 1200
    END
END
"#,
        icon = rc_path(&icon),
    );
    let rc_file = Path::new(&out).join("omnihw.rc");
    fs::write(&rc_file, rc).expect("write resource script");

    embed_resource::compile(&rc_file, embed_resource::NONE)
        .manifest_required()
        .expect("compile Windows resources");
}
