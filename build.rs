//! With the `static-mpv` feature, link libmpv and everything it needs. On Windows, embed the app
//! icon and version information into `vibelc.exe`.
//!
//! Without the feature, other targets need nothing. A missing resource compiler (for example when
//! cross-compiling without mingw's `windres`) only produces a warning, so the build still links.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/icon/vibelc.ico");
    if std::env::var_os("CARGO_FEATURE_STATIC_MPV").is_some() {
        link_mpv();
    }
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

/// The libraries `pkg-config --static --libs mpv` lists. On Windows all of them static, also the
/// C++ runtime (with `+crt-static` for libunwind), so that vibelc.exe imports only system DLLs.
fn link_mpv() {
    println!("cargo:rerun-if-env-changed=PKG_CONFIG_PATH");
    println!("cargo:rerun-if-env-changed=PKG_CONFIG_LIBDIR");
    let out = std::process::Command::new("pkg-config")
        .args(["--static", "--libs", "mpv"])
        .output()
        .expect("static-mpv needs pkg-config");
    assert!(out.status.success(), "pkg-config --static --libs mpv: {}", String::from_utf8_lossy(&out.stderr));
    // rustc leaves the linker in dynamic mode; mingw's system import libraries are .a files too.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rustc-link-arg=-Wl,-Bstatic");
    }
    for flag in String::from_utf8(out.stdout).expect("pkg-config output").split_whitespace() {
        println!("cargo:rustc-link-arg={flag}");
    }
}
