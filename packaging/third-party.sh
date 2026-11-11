#!/bin/sh
# Writes target/THIRD-PARTY.md, the licences of everything VibeLC ships. Needs cargo-about
# (`cargo install cargo-about --locked --features cli`).
set -eu
cd "$(dirname "$0")/.."
mkdir -p target
cargo about generate --config packaging/about.toml packaging/about.hbs -o target/THIRD-PARTY.md
