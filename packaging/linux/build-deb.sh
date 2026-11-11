#!/bin/sh
# Builds target/vibelc_<version>_amd64.deb: vibelc with its own libmpv (build-libmpv.sh), both
# compiled on Ubuntu 20.04 in Docker, so the package runs on 20.04 and later with no media
# packages installed. Needs Docker and cargo-about.
set -eu
cd "$(dirname "$0")/../.."
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n1)
if [ -z "${IN_BUILDER:-}" ]; then
    packaging/third-party.sh
    docker build -q -t vibelc-linux packaging/linux >/dev/null
    exec docker run --rm --user "$(id -u):$(id -g)" -e IN_BUILDER=1 -e HOME=/tmp -e CARGO_HOME=/cargo \
        -e RUSTUP_HOME=/rustup -e MAINTAINER="$(git config user.name) <$(git config user.email)>" \
        -v "${CARGO_HOME:-$HOME/.cargo}:/cargo" -v "${RUSTUP_HOME:-$HOME/.rustup}:/rustup" -v "$PWD:/src" -w /src \
        vibelc-linux sh packaging/linux/build-deb.sh
fi

# In the builder from here on.
sh packaging/linux/build-libmpv.sh
/cargo/bin/cargo build --release --target-dir target/linux/cargo
root=target/linux/deb
rm -rf "$root"
install -Dm755 target/linux/cargo/release/vibelc "$root/usr/bin/vibelc"
install -Dm644 target/linux/prefix/lib/libmpv.so.2 "$root/usr/lib/vibelc/libmpv.so.2"
strip --strip-unneeded "$root/usr/lib/vibelc/libmpv.so.2"
install -Dm644 packaging/linux/vibelc.desktop "$root/usr/share/applications/vibelc.desktop"
for size in 16 24 32 48 64 128 256 512; do
    install -Dm644 "assets/icon/vibelc-$size.png" "$root/usr/share/icons/hicolor/${size}x$size/apps/vibelc.png"
done
for f in packaging/linux/copyright target/THIRD-PARTY.md assets/fonts/OFL-Inter.txt assets/fonts/LICENSE-lucide.txt; do
    install -Dm644 "$f" "$root/usr/share/doc/vibelc/$(basename "$f")"
done

# Linked libraries, versioned for this 20.04 baseline, plus what winit and glutin load at run time.
mkdir -p target/linux/shlibs/debian
printf 'Source: vibelc\n\nPackage: vibelc\nArchitecture: amd64\n' > target/linux/shlibs/debian/control
linked=$(cd target/linux/shlibs && dpkg-shlibdeps -O "/src/$root/usr/bin/vibelc" "/src/$root/usr/lib/vibelc/libmpv.so.2" 2>/dev/null)
linked=${linked#shlibs:Depends=}
mkdir -p "$root/DEBIAN"
cat > "$root/DEBIAN/control" <<CONTROL
Package: vibelc
Version: $version
Architecture: amd64
Maintainer: $MAINTAINER
Installed-Size: $(du -sk "$root/usr" | cut -f1)
Depends: $linked, libgl1, libx11-xcb1, libxcursor1, libxi6, libxkbcommon-x11-0
Recommends: va-driver-all | va-driver, xdg-desktop-portal
Section: video
Priority: optional
Description: fully vibe coded lightweight vlc like video player
 VibeLC plays local files, folders, playlists and network streams, with
 hardware decoding, a playlist panel, an equalizer and the familiar menu and
 keyboard layout of desktop media players. It brings its own libmpv and FFmpeg.
CONTROL
dpkg-deb --build --root-owner-group "$root" "target/vibelc_${version}_amd64.deb"
