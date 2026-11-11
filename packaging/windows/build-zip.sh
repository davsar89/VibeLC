#!/bin/sh
# Builds target/vibelc-<version>.zip, one VibeLC folder with everything:
# - vibelc.exe cross-compiled in Docker, with libmpv-2.dll from shinchiro's mpv-winbuild-cmake
#   (the generic x86-64 build; "-v3" needs AVX2), and the Vulkan loader that DLL links;
# - the Linux .deb from packaging/linux/build-deb.sh;
# - source/: this project with its git history, without the build outputs in target/.
# Needs Docker, curl, cargo-about and the host's rustup target x86_64-pc-windows-gnu.
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
run /cargo/bin/cargo build --release --target x86_64-pc-windows-gnu
packaging/third-party.sh

if [ ! -f "$out/libmpv-2.dll" ]; then
    url=$(curl -fsSL https://api.github.com/repos/shinchiro/mpv-winbuild-cmake/releases/latest |
        grep -o 'https://[^"]*/mpv-dev-x86_64-[0-9][^"]*\.7z' | head -n1)
    echo "Downloading $url"
    curl -fsSL -o "$out/mpv-dev.7z" "$url"
    echo "$url" > "$out/libmpv-source.txt"
    run 7z e -y -o"$out" "$out/mpv-dev.7z" libmpv-2.dll >/dev/null
fi

# libmpv-2.dll links vulkan-1.dll, which only Vulkan-capable graphics drivers install, so the zip
# carries the Khronos loader from LunarG's Vulkan Runtime.
vk=1.4.335.0
if [ ! -f "$out/vulkan-1.dll" ]; then
    curl -fsSL -o "$out/vulkan-rt.zip" "https://sdk.lunarg.com/sdk/download/$vk/windows/VulkanRT-X64-$vk-Components.zip"
    run 7z e -y -o"$out" "$out/vulkan-rt.zip" "VulkanRT-X64-$vk-Components/x64/vulkan-1.dll" \
        "VulkanRT-X64-$vk-Components/VulkanRT-License.txt" >/dev/null
fi

stage="$out/VibeLC"
rm -rf "$stage" "target/vibelc-$version.zip"
mkdir -p "$stage/source"
cp target/x86_64-pc-windows-gnu/release/vibelc.exe "$out"/libmpv-2.dll "$out"/libmpv-source.txt "$out"/vulkan-1.dll \
    "$out"/VulkanRT-License.txt LICENSE README.md "target/vibelc_${version}_amd64.deb" \
    target/THIRD-PARTY.md assets/fonts/OFL-Inter.txt assets/fonts/LICENSE-lucide.txt "$stage/"
tar -cf - --exclude=./target . | tar -xf - -C "$stage/source"
run sh -c "cd $out && zip -qr ../vibelc-$version.zip VibeLC"
ls -l "target/vibelc-$version.zip"
