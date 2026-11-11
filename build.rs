//! Windows only: embed the app icon and version information into `vibelc.exe`.
//!
//! On other targets this does nothing. A missing resource compiler (for example when
//! cross-compiling without mingw's `windres`) only produces a warning, so the build still links.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/icon/vibelc.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/icon/vibelc.ico")
        .set("ProductName", "VibeLC")
        .set("FileDescription", "VibeLC - fully vibe coded lightweight vlc like video player")
        .set("LegalCopyright", "VibeLC contributors. GPL-3.0-or-later.")
        .set("OriginalFilename", "vibelc.exe")
        .set("InternalName", "vibelc");
    if let Err(e) = res.compile() {
        println!("cargo:warning=vibelc.exe built without icon/version resources: {e}");
    }
}
