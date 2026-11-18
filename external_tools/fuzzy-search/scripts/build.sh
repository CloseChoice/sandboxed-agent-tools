#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
. scripts/env.sh
cargo build --locked --release --target wasm32-wasip2
mkdir -p dist
cp target/wasm32-wasip2/release/fff_fuzzy_component.wasm dist/fff-fuzzy.wasm
echo 'Built dist/fff-fuzzy.wasm; cargo test checks that no host imports are needed'
