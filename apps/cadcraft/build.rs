//! Windows only: embed the app icon and version info into `cadcraft.exe`. Elsewhere a no-op.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../assets/app-icon/cadcraft.ico");
    println!("cargo:rerun-if-env-changed=CADCRAFT_REQUIRE_WINRES");
    println!("cargo:rerun-if-env-changed=CADCRAFT_BUILD_SHA");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    if std::path::Path::new("../../assets/app-icon/cadcraft.ico").exists() {
        res.set_icon("../../assets/app-icon/cadcraft.ico");
    }
    res.set("ProductName", "DraftCraft")
        .set("FileDescription", "DraftCraft computer-aided design (built on CADCraft)")
        .set("CompanyName", "WithShalom Studios")
        .set("LegalCopyright", "Copyright (c) 2026 ArtCraft Team, the CADCraft contributors and WithShalom Studios. MIT OR Apache-2.0.")
        .set("OriginalFilename", "draftcraft.exe")
        .set("InternalName", "draftcraft");
    if let Err(e) = res.compile() {
        if std::env::var_os("CADCRAFT_REQUIRE_WINRES").is_some() {
            eprintln!("embedding Windows resources failed: {e}");
            std::process::exit(1);
        }
        println!("cargo:warning=cadcraft.exe built without icon/version resources: {e}");
    }
}
