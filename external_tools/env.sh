# Source after changing to a tool directory. Honor the optional repository-local Rust setup.
sat_repo_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ -x "$sat_repo_dir/.toolchain/cargo/bin/rustup" && -d "$sat_repo_dir/.toolchain/rustup/toolchains" ]]; then
    export CARGO_HOME="${CARGO_HOME:-$sat_repo_dir/.toolchain/cargo}"
    export RUSTUP_HOME="${RUSTUP_HOME:-$sat_repo_dir/.toolchain/rustup}"
fi
export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"
unset sat_repo_dir
