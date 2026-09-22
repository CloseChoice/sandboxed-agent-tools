#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
. scripts/env.sh
command -v wasm-tools >/dev/null || {
    echo 'Install wasm-tools first: cargo install --locked wasm-tools --version 1.254.0' >&2
    exit 1
}
cargo build --locked --release --target wasm32-unknown-unknown
mkdir -p dist
wasm-tools component new target/wasm32-unknown-unknown/release/fff_fuzzy_component.wasm -o dist/fff-fuzzy.wasm
wasm-tools validate dist/fff-fuzzy.wasm
wasm-tools component wit dist/fff-fuzzy.wasm -o dist/component.wit
if grep -Eq '^[[:space:]]*import[[:space:]]' dist/component.wit; then
    echo 'Unexpected host imports in dist/component.wit' >&2
    exit 1
fi
echo 'Built dist/fff-fuzzy.wasm (zero host imports)'
