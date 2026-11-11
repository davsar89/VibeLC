# Cross-compiles libmpv (build-libmpv.sh) and vibelc.exe with llvm-mingw: clang, lld and libc++
# for Windows, which also write PDB symbols for Windows profilers. Its Ubuntu base has gcc for the
# build scripts, which run on Linux. The Rust toolchain comes from the host (mounted by build-zip.sh).
FROM mstorsjo/llvm-mingw:20261020
RUN apt-get update && apt-get install -y --no-install-recommends meson nasm pkg-config python3 curl git \
        ca-certificates xz-utils zip \
    && rm -rf /var/lib/apt/lists/*
