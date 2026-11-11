#!/bin/sh
# Builds a static libmpv for Windows into target/windows/prefix, for `cargo build --features
# static-mpv` (build-zip.sh): mpv with only what VibeLC uses (Direct3D 11 output and D3D11VA,
# the driver's OpenGL as fallback, WASAPI, ytdl for URLs) and FFmpeg with dav1d, Schannel for
# TLS, and no encoders but those of snapshots. vibelc.exe then imports only system DLLs.
# Runs in the image from packaging/windows/Dockerfile. JOBS sets the parallel jobs (default:
# half the processors, the first build takes about an hour).
set -eu
P=$PWD/target/windows/prefix
S=$PWD/target/windows/src
mkdir -p "$P/lib/pkgconfig" "$P/include" "$S"
export PKG_CONFIG_LIBDIR="$P/lib/pkgconfig" PKG_CONFIG_PATH=
jobs=${JOBS:-$(( ($(nproc) + 1) / 2 ))}
host=x86_64-w64-mingw32
# -gcodeview: debugging information as PDB symbols (for profilers; it does not change the code).
flags="-O2 -g -gcodeview"

fetch() { # url name
    [ -d "$S/$2" ] && return
    curl -fsSL -o "$S/$2.tar" "$1"
    mkdir "$S/$2.tmp" && tar -xf "$S/$2.tar" -C "$S/$2.tmp" --strip-components=1
    mv "$S/$2.tmp" "$S/$2" && rm "$S/$2.tar"
}
cat > "$S/cross.txt" <<EOF
[binaries]
c = '$host-clang'
cpp = '$host-clang++'
ar = 'llvm-ar'
strip = 'llvm-strip'
windres = '$host-windres'
pkg-config = 'pkg-config'
[built-in options]
c_args = '$flags'
cpp_args = '$flags'
[host_machine]
system = 'windows'
cpu_family = 'x86_64'
cpu = 'x86_64'
endian = 'little'
EOF
meson_static() { # name options...
    d=$S/$1; shift
    [ -d "$d/build" ] || meson setup "$d/build" "$d" --cross-file="$S/cross.txt" --prefix="$P" \
        --libdir=lib --buildtype=release -Ddefault_library=static "$@"
    ninja -C "$d/build" -j"$jobs" install >/dev/null
}
cmake_static() { # name options...
    d=$S/$1; shift
    [ -d "$d/build" ] || cmake -S "$d" -B "$d/build" -G Ninja -DCMAKE_SYSTEM_NAME=Windows \
        -DCMAKE_C_COMPILER=$host-clang -DCMAKE_CXX_COMPILER=$host-clang++ -DCMAKE_RC_COMPILER=$host-windres \
        -DCMAKE_BUILD_TYPE=Release -DCMAKE_C_FLAGS="$flags" -DCMAKE_CXX_FLAGS="$flags" \
        -DCMAKE_INSTALL_PREFIX="$P" -DCMAKE_INSTALL_LIBDIR=lib -DBUILD_SHARED_LIBS=OFF "$@" >/dev/null
    ninja -C "$d/build" -j"$jobs" install >/dev/null
}
autotools() { # configure options..., in the current directory
    CC=$host-clang CXX=$host-clang++ AR=llvm-ar RANLIB=llvm-ranlib CFLAGS="$flags" \
        ./configure --host=$host --prefix="$P" --enable-static --disable-shared "$@" >/dev/null
    make -j"$jobs" install >/dev/null
}
pc() { # name version libs, for libraries that install no pkg-config file
    printf 'prefix=%s\nName: %s\nDescription: %s\nVersion: %s\nCflags: -I%s/include\nLibs: -L%s/lib %s\n' \
        "$P" "$1" "$1" "$2" "$P" "$P" "$3" > "$P/lib/pkgconfig/$1.pc"
}

# Our libraries exist only as static archives, so whoever links them must also link their
# private dependencies: fold Libs.private and Requires.private into Libs and Requires.
fold_private() {
    python3 - "$P"/lib/pkgconfig/*.pc <<'PY'
import re, sys
for path in sys.argv[1:]:
    text = open(path).read()
    for field in ("Libs", "Requires"):
        private = re.search(rf"^{field}\.private:(.*)$", text, re.M)
        if not private:
            continue
        text = re.sub(rf"^{field}\.private:.*\n?", "", text, flags=re.M)
        if re.search(rf"^{field}:", text, re.M):
            text = re.sub(rf"^({field}:.*)$", lambda m: f"{m.group(1)} {private.group(1).strip()}", text, count=1, flags=re.M)
        else:
            text += f"{field}: {private.group(1).strip()}\n"
    open(path, "w").write(text)
PY
}

fetch https://github.com/madler/zlib/releases/download/v1.3.1/zlib-1.3.1.tar.xz zlib
if [ ! -f "$P/lib/pkgconfig/zlib.pc" ]; then
    make -C "$S/zlib" -f win32/Makefile.gcc -j"$jobs" CC=$host-clang AR=llvm-ar RC=$host-windres \
        CFLAGS="$flags" libz.a >/dev/null
    cp "$S/zlib/libz.a" "$P/lib/"
    cp "$S/zlib/zlib.h" "$S/zlib/zconf.h" "$P/include/"
    pc zlib 1.3.1 -lz
fi

fetch https://downloads.videolan.org/pub/videolan/dav1d/1.5.4/dav1d-1.5.4.tar.xz dav1d
meson_static dav1d -Denable_tools=false -Denable_tests=false

fetch https://github.com/FFmpeg/FFmpeg/archive/refs/tags/n8.1.3.tar.gz ffmpeg
fold_private
if [ ! -f "$P/lib/libavcodec.a" ]; then
    (cd "$S/ffmpeg" && ./configure --prefix="$P" --enable-cross-compile --arch=x86_64 --target-os=mingw32 \
        --cc=$host-clang --cxx=$host-clang++ --ar=llvm-ar --ranlib=llvm-ranlib --nm=llvm-nm \
        --pkg-config=pkg-config --pkg-config-flags=--static --extra-cflags=-gcodeview \
        --enable-static --disable-shared --disable-programs --disable-doc --disable-avdevice \
        --disable-encoders --enable-encoder=png,mjpeg --disable-muxers \
        --enable-d3d11va --enable-dxva2 --enable-schannel --enable-zlib --enable-libdav1d >/dev/null \
        && make -j"$jobs" install >/dev/null)
fi

fetch https://download.savannah.gnu.org/releases/freetype/freetype-2.14.3.tar.xz freetype
meson_static freetype -Dharfbuzz=disabled -Dpng=disabled -Dbzip2=disabled -Dbrotli=disabled -Dzlib=disabled
fetch https://github.com/fribidi/fribidi/releases/download/v1.0.16/fribidi-1.0.16.tar.xz fribidi
meson_static fribidi -Ddocs=false -Dbin=false -Dtests=false
fetch https://github.com/harfbuzz/harfbuzz/releases/download/11.2.1/harfbuzz-11.2.1.tar.xz harfbuzz
meson_static harfbuzz -Dfreetype=enabled -Dglib=disabled -Dgobject=disabled -Dcairo=disabled -Dicu=disabled \
    -Dtests=disabled -Ddocs=disabled -Dutilities=disabled
fold_private
fetch https://github.com/libass/libass/releases/download/0.17.5/libass-0.17.5.tar.xz libass
[ -f "$P/lib/libass.a" ] || (cd "$S/libass" && autotools --disable-libunibreak --disable-fontconfig \
    --disable-require-system-font-provider)

[ -d "$S/libplacebo" ] || git clone -q --depth 1 --branch v7.387.0 --recurse-submodules --shallow-submodules \
    https://github.com/haasn/libplacebo "$S/libplacebo"
meson_static libplacebo -Dvulkan=disabled -Dopengl=disabled -Dd3d11=disabled -Dglslang=disabled \
    -Dshaderc=disabled -Dlcms=disabled -Ddovi=disabled -Dlibdovi=disabled -Dunwind=disabled -Dxxhash=disabled \
    -Ddemos=false -Dtests=false

# mpv's Direct3D 11 renderer turns its GLSL into SPIR-V (shaderc), then HLSL (SPIRV-Cross).
if [ ! -d "$S/shaderc" ]; then
    git clone -q --depth 1 --branch v2026.3 https://github.com/google/shaderc "$S/shaderc"
    (cd "$S/shaderc" && python3 utils/git-sync-deps >/dev/null)
fi
[ -f "$P/lib/libshaderc_combined.a" ] || cmake_static shaderc -DSHADERC_SKIP_TESTS=ON -DSHADERC_SKIP_EXAMPLES=ON \
    -DSHADERC_SKIP_COPYRIGHT_CHECK=ON -DENABLE_GLSLANG_BINARIES=OFF -DSPIRV_SKIP_EXECUTABLES=ON \
    -DSPIRV_TOOLS_BUILD_STATIC=ON -DSKIP_GLSLANG_INSTALL=ON -DSKIP_SPIRV_TOOLS_INSTALL=ON
pc shaderc 2026.3 "-lshaderc_combined -lc++"
[ -d "$S/spirv-cross" ] || git clone -q --depth 1 --branch vulkan-sdk-1.4.357.0 \
    https://github.com/KhronosGroup/SPIRV-Cross "$S/spirv-cross"
[ -f "$P/lib/libspirv-cross-c.a" ] || cmake_static spirv-cross -DSPIRV_CROSS_SHARED=OFF -DSPIRV_CROSS_STATIC=ON \
    -DSPIRV_CROSS_CLI=OFF -DSPIRV_CROSS_ENABLE_TESTS=OFF -DSPIRV_CROSS_ENABLE_MSL=OFF -DSPIRV_CROSS_ENABLE_CPP=OFF \
    -DSPIRV_CROSS_ENABLE_REFLECT=OFF -DSPIRV_CROSS_ENABLE_UTIL=OFF
pc spirv-cross-c-shared 0.64 "-lspirv-cross-c -lspirv-cross-hlsl -lspirv-cross-glsl -lspirv-cross-core -lc++"
sed -i 's|^Cflags: .*|&/spirv_cross|' "$P/lib/pkgconfig/spirv-cross-c-shared.pc"

fetch https://github.com/LuaJIT/LuaJIT/archive/c4b1e2f73a2e10fbcc637fcec474f639c38daafe.tar.gz luajit
if [ ! -f "$P/lib/pkgconfig/luajit.pc" ]; then
    make -C "$S/luajit" -j"$jobs" amalg HOST_CC=gcc CROSS=$host- CC=clang TARGET_SYS=Windows \
        BUILDMODE=static TARGET_AR="llvm-ar rcus" TARGET_CFLAGS="-g -gcodeview" >/dev/null
    # Its `make install` wants a Unix luajit program; only the library and headers are needed.
    mkdir -p "$P/include/luajit-2.1"
    (cd "$S/luajit/src" && cp lua.h lualib.h lauxlib.h luaconf.h lua.hpp luajit.h "$P/include/luajit-2.1/" \
        && cp libluajit.a "$P/lib/libluajit-5.1.a")
    pc luajit 2.1.0 -lluajit-5.1
    sed -i 's|^Cflags: .*|&/luajit-2.1|' "$P/lib/pkgconfig/luajit.pc"
fi

fetch https://github.com/mpv-player/mpv/archive/refs/tags/v0.42.0.tar.gz mpv
fold_private
meson_static mpv -Dlibmpv=true -Dcplayer=false -Dlua=luajit -Djavascript=disabled -Dd3d11=enabled \
    -Dd3d-hwaccel=enabled -Dgl=enabled -Dgl-win32=enabled -Degl-angle=disabled -Dvulkan=disabled \
    -Dshaderc=enabled -Dspirv-cross=enabled -Dlibavdevice=disabled -Dlibarchive=disabled -Dlibbluray=disabled \
    -Ddvdnav=disabled -Dcdda=disabled -Drubberband=disabled -Dzimg=disabled -Dlcms2=disabled -Duchardet=disabled \
    -Diconv=disabled -Djpeg=disabled -Dmanpage-build=disabled -Dhtml-build=disabled -Dpdf-build=disabled
fold_private
