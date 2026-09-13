#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
. scripts/env.sh
cargo build --locked --release --target wasm32-unknown-unknown
node scripts/package.mjs
