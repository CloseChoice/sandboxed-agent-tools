#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
export SAT_HOME="${SAT_HOME:-$ROOT/sat/target/demo/registry}"
SAT="$ROOT/sat/target/debug/sat"
GUEST="$ROOT/sat/ripgrep/target/wasm32-wasip2/release/sat_ripgrep_guest.wasm"
PACKAGE="$ROOT/sat/target/ripgrep.wasm"

cargo build --locked --manifest-path "$ROOT/sat/Cargo.toml"
cargo build --locked --manifest-path "$ROOT/sat/ripgrep/Cargo.toml" --target wasm32-wasip2 --release
"$SAT" pack "$GUEST" --metadata "$ROOT/sat/ripgrep/tool.toml" \
  --policy "$ROOT/sat/ripgrep/policy.toml" --output "$PACKAGE"
"$SAT" install "$PACKAGE" --replace
