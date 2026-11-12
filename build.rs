fn main() {
    // App icon and manifest. Optional: a GNU toolchain without a working `windres` still builds,
    // just without the exe icon (the tray icon is drawn at runtime anyway).
    println!("cargo:rerun-if-changed=assets/snowie.rc");
    println!("cargo:rerun-if-changed=assets/snowie.ico");
    println!("cargo:rerun-if-changed=assets/snowie.manifest");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    if let Err(e) = embed_resource::compile("assets/snowie.rc", embed_resource::NONE).manifest_optional() {
        println!("cargo:warning=exe icon not embedded: {e}");
    }
}
