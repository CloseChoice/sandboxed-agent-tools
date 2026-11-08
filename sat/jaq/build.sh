#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
export SAT_HOME="${SAT_HOME:-$ROOT/sat/target/demo/registry}"
PROFILE="${SAT_BUILD_PROFILE:-release}"
if [[ "$PROFILE" != release && "$PROFILE" != debug ]]; then
  echo 'SAT_BUILD_PROFILE must be release or debug' >&2
  exit 1
fi
if [[ "$PROFILE" == release ]]; then
  cargo build --release --locked --manifest-path "$ROOT/sat/Cargo.toml"
else
  cargo build --locked --manifest-path "$ROOT/sat/Cargo.toml"
fi
SAT="$ROOT/sat/target/$PROFILE/sat"
GUEST="$ROOT/sat/jaq/target/wasm32-wasip2/release/sat_jaq_guest.wasm"
PACKAGE="$ROOT/sat/target/jaq.wasm"
cargo build --locked --manifest-path "$ROOT/sat/jaq/Cargo.toml" --target wasm32-wasip2 --release
"$SAT" pack "$GUEST" --metadata "$ROOT/sat/jaq/tool.toml" \
  --policy "$ROOT/sat/jaq/policy.toml" --output "$PACKAGE"
"$SAT" install "$PACKAGE" --replace
