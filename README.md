# Sandboxed agent tools

The native host lives in [`sat/`](sat/). Tool source, examples, tests, and packaging scripts live in [`external_tools/`](external_tools/).

| Tool | Directory |
| --- | --- |
| File-content search | [file-search](external_tools/file-search/) |
| Fuzzy matching | [fuzzy-search](external_tools/fuzzy-search/) |
| Directory listing and selected-file read | [file-list](external_tools/file-list/) |
| Brave web search | [brave-search](external_tools/brave-search/) |

Build the host:

```sh
cargo build --locked --manifest-path sat/Cargo.toml
```

Build an installable tool archive from any working directory:

```sh
bash external_tools/file-search/package.sh        # dist/file-search.tar
bash external_tools/file-search/package.sh zip    # dist/file-search.zip
```

Each tool has the same `package.sh [tar|zip]` command. Archives contain
`tool.wasm`, `tool.toml`, and `defaults.toml` at their root. Output lives in that
tool's `dist/` directory. Scripts require Bash, Python 3, and the repository's
Rust toolchain (including `wasm32-wasip2`).

Install with SAT and review the suggested permissions:

```sh
sat/target/debug/sat register external_tools/file-search/dist/file-search.tar
sat/target/debug/sat file-search --help
```

Defaults grant no filesystem access. Supply the minimum paths needed for an
invocation after `--`; see each tool's README for examples. Brave suggests only
`api.search.brave.com` for HTTP access.

Check the host:

```sh
cargo fmt --manifest-path sat/Cargo.toml --all -- --check
cargo clippy --manifest-path sat/Cargo.toml --all-targets --locked
cargo test --manifest-path sat/Cargo.toml --locked
```

Run the packaging and SAT installation checks:

```sh
python3 external_tools/test_packages.py
```

Build outputs (`target/`, `dist/`) and optional local toolchains (`.toolchain/`)
are ignored by Git. Dependency lockfiles are tracked.
