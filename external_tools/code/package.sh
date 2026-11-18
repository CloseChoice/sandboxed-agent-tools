#!/usr/bin/env bash
set -euo pipefail
tool_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
archive_format="${1:-tar}"
if [[ $# -gt 1 || ( "$archive_format" != tar && "$archive_format" != zip ) ]]; then
    echo "Usage: $0 [tar|zip]" >&2
    exit 2
fi
cd "$tool_dir"
. ../env.sh
if [[ -z "${WASI_SDK_PATH:-}" || ! -x "$WASI_SDK_PATH/bin/clang" ]]; then
    echo 'Set WASI_SDK_PATH to an extracted WASI SDK (see README.md).' >&2
    exit 1
fi
export CC_wasm32_wasip2="$WASI_SDK_PATH/bin/clang"
export AR_wasm32_wasip2="$WASI_SDK_PATH/bin/llvm-ar"
export CFLAGS_wasm32_wasip2="--sysroot=$WASI_SDK_PATH/share/wasi-sysroot"
cargo build --locked --manifest-path Cargo.toml --target-dir "$tool_dir/target" \
    --release --target wasm32-wasip2
python3 ../package.py "$tool_dir" "$tool_dir/target/wasm32-wasip2/release/sat_code_guest.wasm" "$archive_format"
