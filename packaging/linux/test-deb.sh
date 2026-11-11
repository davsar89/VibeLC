#!/bin/sh
# Installs target/vibelc_*.deb in a clean container and checks that it plays the fixture under
# Xvfb with its own libmpv. Where the host-built test binary runs (glibc 2.35 and later), it also
# runs the test suite against that libmpv. Usage: test-deb.sh [ubuntu:24.04]
set -eu
cd "$(dirname "$0")/../.."
tests=$(cargo test --no-run 2>&1 | grep -o 'target/debug/deps/vibelc-[0-9a-f]*' | head -n1)
docker run --rm -v "$PWD:$PWD:ro" -w "$PWD" -e TESTS="$tests" "${1:-ubuntu:24.04}" sh -euc '
    export DEBIAN_FRONTEND=noninteractive
    apt-get update -qq
    apt-get install -y -qq ./target/vibelc_*_amd64.deb xvfb >/dev/null 2>&1
    Xvfb :1 -screen 0 1280x800x24 >/dev/null 2>&1 &
    export DISPLAY=:1 VIBELC_CONFIG_DIR=/tmp/vibelc XDG_RUNTIME_DIR=/tmp
    sleep 1
    timeout 8 vibelc --stats tests/fixtures/av.mkv >/dev/null 2>&1 || true
    log=/tmp/vibelc/vibelc.log
    grep -E "loaded|OpenGL|VO:" $log
    grep -q "VO: \[libmpv\]" $log && grep -q "stats: [1-9]" $log && ! grep -q panicked $log
    if "$TESTS" --list >/dev/null 2>&1; then
        VIBELC_LIBMPV=/usr/lib/vibelc/libmpv.so.2 VIBELC_REQUIRE_LIBMPV=1 "$TESTS" --test-threads=1 2>&1 | tail -n 2
    else
        echo "The test binary needs a newer glibc; only the playback check ran."
    fi
'
