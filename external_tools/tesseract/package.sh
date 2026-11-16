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
python3 build.py
python3 ../package.py "$tool_dir" "$tool_dir/target/wasm32-wasip2/release/sat_tesseract_guest.wasm" "$archive_format"
