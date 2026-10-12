fn main() {
    println!("cargo:rerun-if-changed=omnihw.rc");
    println!("cargo:rerun-if-changed=../../Assets/Brand/OmniHardwareIcon.ico");
    // Embed the app icon so Explorer, the taskbar and the tray show it.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("omnihw.rc", embed_resource::NONE)
            .manifest_optional()
            .expect("compile Windows resources");
    }
}
