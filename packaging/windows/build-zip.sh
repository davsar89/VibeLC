#!/bin/sh
# Builds target/vibelc-<version>.zip, one VibeLC folder with everything:
# - vibelc.exe cross-compiled in Docker with llvm-mingw, libmpv and FFmpeg linked in
#   (build-libmpv.sh), so that it is the only file the Windows program needs;
# - the Linux .deb from packaging/linux/build-deb.sh;
# - source/: this project with its git history, without the build outputs in target/.
# Needs Docker, cargo-about and the host's rustup target x86_64-pc-windows-gnullvm.
set -eu
cd "$(dirname "$0")/../.."
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n1)
packaging/linux/build-deb.sh
out=target/windows
mkdir -p "$out"
docker build -q -t vibelc-win packaging/windows >/dev/null
run() {
    docker run --rm --user "$(id -u):$(id -g)" -e HOME=/tmp -e CARGO_HOME=/cargo -e RUSTUP_HOME=/rustup \
        -v "${CARGO_HOME:-$HOME/.cargo}:/cargo" -v "${RUSTUP_HOME:-$HOME/.rustup}:/rustup" -v "$PWD:/src" -w /src \
        vibelc-win "$@"
}
run sh packaging/windows/build-libmpv.sh
run env PKG_CONFIG_LIBDIR=/src/target/windows/prefix/lib/pkgconfig \
    CARGO_TARGET_X86_64_PC_WINDOWS_GNULLVM_LINKER=x86_64-w64-mingw32-clang \
    /cargo/bin/cargo build --release --target x86_64-pc-windows-gnullvm --features static-mpv
packaging/third-party.sh

stage="$out/VibeLC"
rm -rf "$stage" "target/vibelc-$version.zip"
mkdir -p "$stage/source"
cp target/x86_64-pc-windows-gnullvm/release/vibelc.exe LICENSE README.md "target/vibelc_${version}_amd64.deb" \
    target/THIRD-PARTY.md assets/fonts/OFL-Inter.txt assets/fonts/LICENSE-lucide.txt "$stage/"
tar -cf - --exclude=./target . | tar -xf - -C "$stage/source"
run sh -c "cd $out && zip -qr ../vibelc-$version.zip VibeLC"
ls -l "target/vibelc-$version.zip"
