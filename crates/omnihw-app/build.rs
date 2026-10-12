//! Embeds the Windows resources into omnihw.exe: the app icon, version info (shown in Explorer's
//! Details tab and Task Manager) and an application manifest. The version comes from Cargo.toml,
//! so the resource script and manifest are generated into OUT_DIR rather than kept by hand.

use std::{env, fs, path::Path};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../Assets/Brand/OmniHardwareIcon.ico");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let out = env::var("OUT_DIR").expect("OUT_DIR");
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let num = |k: &str| env::var(k).ok().and_then(|v| v.parse::<u16>().ok()).unwrap_or(0);
    let (major, minor, patch) = (
        num("CARGO_PKG_VERSION_MAJOR"),
        num("CARGO_PKG_VERSION_MINOR"),
        num("CARGO_PKG_VERSION_PATCH"),
    );
    let version = env::var("CARGO_PKG_VERSION").unwrap_or_default();

    // Forward slashes keep the paths valid inside RC string literals (rc.exe and windres accept them).
    let rc_path = |p: &Path| p.display().to_string().replace('\\', "/");
    let icon = Path::new(&manifest_dir).join("../../Assets/Brand/OmniHardwareIcon.ico");
    let manifest = Path::new(&out).join("omnihw.exe.manifest");

    fs::write(&manifest, app_manifest(major, minor, patch)).expect("write manifest");

    let rc = format!(
        r#"#pragma code_page(65001)
// Application icon. Resource id 1 is also what the tray loads (tray.rs).
1 ICON "{icon}"

// RT_MANIFEST (24), id 1: the manifest Windows applies to the process.
1 24 "{manifest}"

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
        manifest = rc_path(&manifest),
    );
    let rc_file = Path::new(&out).join("omnihw.rc");
    fs::write(&rc_file, rc).expect("write resource script");

    embed_resource::compile(&rc_file, embed_resource::NONE)
        .manifest_required()
        .expect("compile Windows resources");
}

/// Application manifest:
/// - asInvoker: start without elevation; elevate.rs asks for admin itself when SATA health needs it.
/// - Windows 10/11 compatibility, so the OS doesn't apply legacy shims or lie about its version.
/// - Per-monitor DPI awareness (v2), matching what winit sets at runtime, so the window is crisp
///   from the first frame.
/// - Long path support and Common Controls v6 for native dialogs.
fn app_manifest(major: u16, minor: u16, patch: u16) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <assemblyIdentity type="win32" name="OmniHW.App" version="{major}.{minor}.{patch}.0" processorArchitecture="*"/>
  <description>Omni-HW</description>
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="asInvoker" uiAccess="false"/>
      </requestedPrivileges>
    </security>
  </trustInfo>
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
    <application>
      <supportedOS Id="{{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}}"/>
    </application>
  </compatibility>
  <application xmlns="urn:schemas-microsoft-com:asm.v3">
    <windowsSettings>
      <dpiAware xmlns="http://schemas.microsoft.com/SMI/2005/WindowsSettings">true/pm</dpiAware>
      <dpiAwareness xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">PerMonitorV2</dpiAwareness>
      <longPathAware xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">true</longPathAware>
    </windowsSettings>
  </application>
  <dependency>
    <dependentAssembly>
      <assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0" processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*"/>
    </dependentAssembly>
  </dependency>
</assembly>
"#
    )
}
