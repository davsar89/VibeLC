# Cross-compiler for vibelc.exe (gcc links the build scripts, which run on Linux). The Rust toolchain comes from the host (mounted by build-zip.sh).
FROM ubuntu:24.04
RUN apt-get update && apt-get install -y --no-install-recommends gcc libc6-dev gcc-mingw-w64-x86-64 p7zip-full zip \
    && rm -rf /var/lib/apt/lists/*
