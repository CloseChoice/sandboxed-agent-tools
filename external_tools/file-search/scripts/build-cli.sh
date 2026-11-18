#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
. scripts/env.sh
cargo build --locked --manifest-path Cargo.toml --release --target wasm32-wasip2
mkdir -p dist
cp target/wasm32-wasip2/release/file-search.wasm dist/file-search.wasm
