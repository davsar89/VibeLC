#!/bin/sh
# Builds a self-contained libmpv.so.2 into target/linux/prefix: mpv with FFmpeg, dav1d,
# libplacebo, libass, LuaJIT and libdrm linked in statically, so that only common desktop
# libraries (X11, EGL, VA-API, PulseAudio, ALSA, fontconfig, GnuTLS) stay shared.
# Runs in the image from packaging/linux/Dockerfile; build-deb.sh calls it.
set -eu
P=$PWD/target/linux/prefix
S=$PWD/target/linux/src
mkdir -p "$P" "$S"
export PKG_CONFIG_PATH="$P/lib/pkgconfig"
jobs=$(nproc)

fetch() { # url name
    [ -d "$S/$2" ] && return
    curl -fsSL -o "$S/$2.tar" "$1"
    mkdir "$S/$2.tmp" && tar -xf "$S/$2.tar" -C "$S/$2.tmp" --strip-components=1
    mv "$S/$2.tmp" "$S/$2" && rm "$S/$2.tar"
}
meson_static() { # name options...
    d=$S/$1; shift
    [ -d "$d/build" ] || meson setup "$d/build" "$d" --prefix="$P" --libdir=lib --buildtype=release \
        -Ddefault_library=static "$@"
    ninja -C "$d/build" install >/dev/null
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

# mpv builds its zero-copy VA-API path (EGL dmabuf interop) only with libdrm 2.4.114 or newer;
# 20.04 has 2.4.107.
fetch https://dri.freedesktop.org/libdrm/libdrm-2.4.134.tar.xz libdrm
meson_static libdrm -Dintel=disabled -Dradeon=disabled -Damdgpu=disabled -Dnouveau=disabled \
    -Dvmwgfx=disabled -Dtests=false -Dman-pages=disabled -Dvalgrind=disabled -Dcairo-tests=disabled

fetch https://downloads.videolan.org/pub/videolan/dav1d/1.5.4/dav1d-1.5.4.tar.xz dav1d
meson_static dav1d -Denable_tools=false -Denable_tests=false

fetch https://github.com/FFmpeg/nv-codec-headers/archive/refs/tags/n13.0.19.0.tar.gz nv-codec-headers
make -C "$S/nv-codec-headers" PREFIX="$P" install >/dev/null

fetch https://github.com/FFmpeg/FFmpeg/archive/refs/tags/n8.1.3.tar.gz ffmpeg
fold_private
if [ ! -f "$P/lib/libavcodec.a" ]; then
    (cd "$S/ffmpeg" && ./configure --prefix="$P" --enable-static --disable-shared \
        --enable-pic --disable-programs --disable-doc --disable-debug --disable-avdevice --disable-encoders \
        --enable-encoder=png,mjpeg --disable-muxers --enable-libdav1d --enable-gnutls --enable-vaapi \
        && make -j"$jobs" install >/dev/null)
fi

[ -d "$S/libplacebo" ] || git clone -q --depth 1 --branch v7.387.0 --recurse-submodules --shallow-submodules \
    https://github.com/haasn/libplacebo "$S/libplacebo"
meson_static libplacebo -Dvulkan=disabled -Dglslang=disabled -Dshaderc=disabled -Dlcms=disabled \
    -Ddovi=disabled -Dlibdovi=disabled -Dunwind=disabled -Dxxhash=disabled -Ddemos=false -Dtests=false

fetch https://github.com/libass/libass/releases/download/0.17.5/libass-0.17.5.tar.xz libass
if [ ! -f "$P/lib/libass.a" ]; then
    (cd "$S/libass" && ./configure --prefix="$P" --enable-static --disable-shared --with-pic \
        --disable-libunibreak --disable-require-system-font-provider && make -j"$jobs" install >/dev/null)
fi

fetch https://github.com/LuaJIT/LuaJIT/archive/c4b1e2f73a2e10fbcc637fcec474f639c38daafe.tar.gz luajit
make -C "$S/luajit" -j"$jobs" amalg BUILDMODE=static CFLAGS="-O2 -fPIC" PREFIX="$P" >/dev/null
make -C "$S/luajit" install PREFIX="$P" >/dev/null


fetch https://github.com/mpv-player/mpv/archive/refs/tags/v0.42.0.tar.gz mpv
fold_private
[ -d "$S/mpv/build" ] || meson setup "$S/mpv/build" "$S/mpv" --prefix="$P" --libdir=lib --buildtype=release \
    -Dlibmpv=true -Dcplayer=false -Dlua=luajit -Djavascript=disabled -Dwayland=disabled -Dvulkan=disabled \
    -Dmanpage-build=disabled -Dhtml-build=disabled -Dc_link_args=-Wl,--exclude-libs,ALL
ninja -C "$S/mpv/build" install >/dev/null
