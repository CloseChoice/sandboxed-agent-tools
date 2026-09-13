#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
. scripts/env.sh
cargo test --locked
cargo run --locked --quiet --example native_reference > target/native-reference.json
node tests/component.test.mjs
