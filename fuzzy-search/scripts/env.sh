# Source this file from the project root: . scripts/env.sh
# Use the optional local installation when present; otherwise keep the
# machine's normal Rust setup and any explicitly configured homes.
if [ -x "$PWD/.toolchain/cargo/bin/rustup" ] && [ -d "$PWD/.toolchain/rustup/toolchains" ]; then
    export CARGO_HOME="${CARGO_HOME:-$PWD/.toolchain/cargo}"
    export RUSTUP_HOME="${RUSTUP_HOME:-$PWD/.toolchain/rustup}"
fi
export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"
if [ -d "$PWD/.toolchain/bin" ]; then
    export PATH="$PWD/.toolchain/bin:$PATH"
fi
