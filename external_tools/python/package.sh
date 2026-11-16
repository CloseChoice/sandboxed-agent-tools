#!/usr/bin/env bash
set -euo pipefail
tool_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
archive_format="${1:-tar}"
if [[ $# -gt 1 || ( "$archive_format" != tar && "$archive_format" != zip ) ]]; then
    echo "Usage: $0 [tar|zip]" >&2
    exit 2
fi
python3 "$tool_dir/build.py"
python3 "$tool_dir/../package.py" "$tool_dir" "$tool_dir/target/python.wasm" "$archive_format"
