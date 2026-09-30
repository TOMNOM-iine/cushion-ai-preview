#!/bin/sh
set -eu
cd "$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
export CARGO_TARGET_DIR=target
# Prevent compiler paths from identifying the build machine.
export RUSTFLAGS="--remap-path-prefix=$(pwd)=/cushion-source --remap-path-prefix=$HOME=/builder-home"
cargo build -p cushion-core --target wasm32-unknown-unknown --features wasm --release --locked
wasm-bindgen --target web --out-dir apps/desktop/src/wasm target/wasm32-unknown-unknown/release/cushion_core.wasm
