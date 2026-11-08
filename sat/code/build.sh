#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
export SAT_HOME="${SAT_HOME:-$ROOT/sat/target/demo/registry}"
WASI_SDK_PATH="${WASI_SDK_PATH:-$ROOT/.toolchain/wasi-sdk-34.0-x86_64-linux}"
if [[ ! -x "$WASI_SDK_PATH/bin/clang" ]]; then
  echo 'Set WASI_SDK_PATH to an extracted wasi-sdk (see sat/code/README.md).' >&2
  exit 1
fi
# Only C compilation uses the SDK. Rust supplies the WASIp2 linker and libc.
export CC_wasm32_wasip2="$WASI_SDK_PATH/bin/clang"
export AR_wasm32_wasip2="$WASI_SDK_PATH/bin/llvm-ar"
export CFLAGS_wasm32_wasip2="--sysroot=$WASI_SDK_PATH/share/wasi-sysroot"
SAT="$ROOT/sat/target/debug/sat"
GUEST="$ROOT/sat/code/target/wasm32-wasip2/release/sat_code_guest.wasm"
PACKAGE="$ROOT/sat/target/code.wasm"
cargo build --locked --manifest-path "$ROOT/sat/Cargo.toml"
cargo build --locked --manifest-path "$ROOT/sat/code/Cargo.toml" --target wasm32-wasip2 --release
"$SAT" pack "$GUEST" --metadata "$ROOT/sat/code/tool.toml" \
  --policy "$ROOT/sat/code/policy.toml" --output "$PACKAGE"
"$SAT" install "$PACKAGE" --replace
