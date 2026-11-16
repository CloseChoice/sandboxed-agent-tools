# Sandboxed Agent Tools (SAT)

SAT is a native host that runs WebAssembly tools with explicit filesystem and
network permissions. The initial release includes four portable tools:

| Tool | Purpose |
| --- | --- |
| [python](external_tools/python/) | CPython 3.14 with a selected standard-library profile |
| [jaq](external_tools/jaq/) | The upstream jq-compatible CLI for structured data |
| [code](external_tools/code/) | Tree-sitter code search and definition retrieval |
| [markitdown](external_tools/markitdown/) | The upstream MarkItDown CLI's local PDF profile |

The host lives in [sat/](sat/). Other tool sources in [external_tools/](external_tools/)
are experimental and are excluded from the initial release. The exact release
selection lives in [release-tools.toml](external_tools/release-tools.toml).

## Build and use

Build the host with the repository's pinned Rust toolchain:

```sh
cargo build --locked --release --manifest-path sat/Cargo.toml
```

Build and register a tool:

```sh
bash external_tools/python/package.sh
sat/target/release/sat register external_tools/python/dist/python.tar
sat/target/release/sat python -c 'print(1 + 2)'
```

Every selected tool provides `package.sh [tar|zip]`. Build scripts require Bash
and Python 3. Rust tools also need `wasm32-wasip2`; `code` needs WASI SDK 34
selected with `WASI_SDK_PATH` for its Tree-sitter C grammars. Python and
MarkItDown use pinned `componentize-py` in a host-side virtual environment.
The first build needs network access to obtain build dependencies.

Packages contain exactly `tool.wasm`, `tool.toml`, and `defaults.toml`.
They are portable WASI components; SAT compiles and caches native code on the
machine where a tool is registered.

Registration defaults grant no filesystem access. Review the suggested
permissions and grant only the paths needed for an invocation after `--`:

```sh
sat python /absolute/script.py -- --read /absolute/script.py
sat code outline --path /absolute/project/src -- --read /absolute/project/src
sat markitdown /absolute/input.pdf -- --read /absolute/input.pdf
printf '{"name":"Ada"}\n' | sat jaq -r .name
```

The runtime enforces permissions. Tools do not inherit unrelated host
environment variables or ambient network authority. The Python package has no
runtime pip or arbitrary PyPI support. MarkItDown supports local PDFs and
stdin, without OCR, remote URLs, plugins, or cloud backends. See each tool's
README for its interface and limitations.

SAT caches compiled components under `$SAT_HOME/.cache` (default
`~/.sat/.cache`). Registration validates and compiles the component. Cache
entries depend on component content and engine configuration. Guests cannot
access SAT's registry/cache, even through a broader filesystem grant.

## Checks

```sh
cargo fmt --manifest-path sat/Cargo.toml --all -- --check
cargo clippy --manifest-path sat/Cargo.toml --all-targets --locked
cargo test --manifest-path sat/Cargo.toml --locked
python3 -m unittest discover -s scripts -p 'test_release.py'
python3 scripts/release.py
```

The release workflow also checks archive contents, tool registration, CLI
behavior, and filesystem confinement. Python build-cache tests and the
MarkItDown upstream PDF/CLI comparison run in CI.

## Release preparation

[Build and release tools](.github/workflows/tool-releases.yml) derives its build
matrix from the release selection file. The complete archive check requires
exactly `python.tar`, `jaq.tar`, `code.tar`, and `markitdown.tar`; it rejects
missing or extra assets.

Branch pushes, pull requests, and manual runs produce Actions artifacts.
Only a pushed `v*` tag publishes a GitHub release. `sat/Cargo.toml` defines
the bundle version, currently `0.1.0`. Rust tool metadata and lockfiles must
match each tool's own Cargo version; Python-based tools keep their versions
in `tool.toml`.

After reviewing the committed sources and successful CI checks:

```sh
python3 scripts/release.py --check-tag v0.1.0
git tag -a v0.1.0 -m "SAT 0.1.0"
git push origin v0.1.0
```

These commands are explicit publishing steps. See [release.md](release.md)
for this branch's scope. Release assets are the four tool archives and
`SHA256SUMS`; build the native SAT host from source.

Download an archive from Releases and use `sat register` with its path or URL.
With all four archives downloaded, verify them using
`sha256sum --check SHA256SUMS`. To check local archive contents:

```sh
python3 external_tools/check_archives.py --complete \
  external_tools/python/dist/python.tar external_tools/jaq/dist/jaq.tar \
  external_tools/code/dist/code.tar external_tools/markitdown/dist/markitdown.tar
```

Build outputs, local toolchains, credentials, personal sample data, logs,
and research checkouts are ignored by Git. Dependency lockfiles are tracked.

## Execution limits

SAT runs tools without Wasmtime fuel metering; legacy package `fuel` values
are ignored. Filesystem/network permissions remain enforced independently.
Callers can cancel a non-finishing invocation through the calling process.
