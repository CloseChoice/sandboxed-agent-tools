#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SAT_HOME="${SAT_HOME:-$ROOT/sat/target/demo/registry}"
export SAT_HOME
SAT="$ROOT/sat/target/debug/sat"
GUEST="$ROOT/sat/pdf-tools/target/wasm32-wasip2/release/sat_pdftools_guest.wasm"
PACKAGE="$ROOT/sat/target/pdftools.wasm"

cargo build --locked --manifest-path "$ROOT/sat/Cargo.toml"
cargo build --locked --manifest-path "$ROOT/sat/pdf-tools/Cargo.toml" --target wasm32-wasip2 --release
mkdir -p "$(dirname "$PACKAGE")"
"$SAT" pack "$GUEST" --metadata "$ROOT/sat/pdf-tools/tool.toml" --output "$PACKAGE"
"$SAT" install "$PACKAGE" --replace
