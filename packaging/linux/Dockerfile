# Builds vibelc and a self-contained libmpv on Ubuntu 20.04, so the package runs on 20.04 and
# later. The Rust toolchain comes from the host (mounted by build-deb.sh).
FROM ubuntu:20.04
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
        build-essential pkg-config ninja-build python3-pip python3.9 curl git ca-certificates xz-utils \
        libva-dev libx11-dev libxext-dev libxpresent-dev libxrandr-dev libxss-dev libxinerama-dev \
        libegl1-mesa-dev libgl1-mesa-dev libpulse-dev libasound2-dev libgnutls28-dev zlib1g-dev \
        libfreetype6-dev libfribidi-dev libharfbuzz-dev libfontconfig1-dev \
    && pip3 install meson==1.6.1 \
    && rm -rf /var/lib/apt/lists/*
# mpv's code generators need Python 3.9; meson itself keeps 20.04's /usr/bin/python3.
RUN ln -s /usr/bin/python3.9 /usr/local/bin/python3
# FFmpeg 8 needs a newer NASM than 20.04's 2.14.
RUN curl -fsSL https://www.nasm.us/pub/nasm/releasebuilds/2.16.03/nasm-2.16.03.tar.xz | tar -xJ -C /tmp \
    && cd /tmp/nasm-2.16.03 && ./configure -q --prefix=/usr/local && make -s -j"$(nproc)" install \
    && rm -rf /tmp/nasm-2.16.03
