# Source this file from the project root: . scripts/env.sh
# Use the optional local installation when present; otherwise keep the
# machine's normal Rust/Node setup and any explicitly configured homes.
if [ -x "$PWD/.toolchain/cargo/bin/rustup" ] && [ -d "$PWD/.toolchain/rustup/toolchains" ]; then
    export CARGO_HOME="${CARGO_HOME:-$PWD/.toolchain/cargo}"
    export RUSTUP_HOME="${RUSTUP_HOME:-$PWD/.toolchain/rustup}"
fi
export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"
if [ -x "$PWD/.toolchain/node-v24.12.0-linux-x64/bin/node" ]; then
    export PATH="$PWD/.toolchain/node-v24.12.0-linux-x64/bin:$PATH"
fi
