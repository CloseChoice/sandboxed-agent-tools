# Sandboxed agent tools

The native host lives in [`sat/`](sat/). Tool source, examples, tests, and packaging scripts live in [`external_tools/`](external_tools/).

| Tool | Directory |
| --- | --- |
| File-content search | [file-search](external_tools/file-search/) |
| Fuzzy matching | [fuzzy-search](external_tools/fuzzy-search/) |
| Directory listing and selected-file read | [file-list](external_tools/file-list/) |
| Brave web search | [brave-search](external_tools/brave-search/) |
| Chrome browser automation and debugging | [browser](external_tools/browser/) |
| Tree-sitter code retrieval | [code](external_tools/code/) |
| jq-style JSON queries | [jaq](external_tools/jaq/) |
| Tesseract image OCR | [tesseract](external_tools/tesseract/) |
| Apply unified diffs | [patch](external_tools/patch/) |
| MarkItDown PDF CLI | [markitdown](external_tools/markitdown/) |
| Page-labelled PDF text extraction | [pdf-text](external_tools/pdf-text/) |
| Image-to-PDF conversion and PDF merging | [pdf-tools](external_tools/pdf-tools/) |
| Recursive regex search | [ripgrep](external_tools/ripgrep/) |
| Sandboxed CPython 3.14 | [python](external_tools/python/) |
| Sandboxed coreutils | [uutils](external_tools/uutils/) |

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
Rust toolchain (including `wasm32-wasip2`). The `code` tool also needs a
WASI SDK selected with `WASI_SDK_PATH` to compile Tree-sitter’s C grammars.
The `python` and `markitdown` tools use Python 3 with venv/pip and pinned `componentize-py`
instead of Cargo; its first build downloads the componentizer and WASI CPython runtime.

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
python3 external_tools/test_imported_tools.py
```

Build outputs (`target/`, `dist/`) and optional local toolchains (`.toolchain/`)
are ignored by Git. Dependency lockfiles are tracked.

For real Chrome DOM inspection, clicks, typing, waits, screenshots and diagnostics,
see [the browser tool](external_tools/browser/README.md). Start a fresh browser
with `sat chrome-session`, then grant its exact endpoint with
`sat browser snapshot -- --allow-cdp WEBSOCKET_URL`. This grants control of that
dedicated Chrome instance; Chrome's own file/network access is separate from WASI
permissions.

SAT caches compiled components under `$SAT_HOME/.cache` (default `~/.sat/.cache`).
Registration compiles and validates the component before reporting success;
the first execution reuses that cached native code. Older registrations, cache
removal, or runtime upgrades can require recompilation.
Wasmtime keys cache entries by component content and engine configuration, so
changed components or runtime settings trigger recompilation. Guests cannot
access the SAT home directory, even through a broader filesystem grant.

## GitHub builds and releases

[Build and release tools](.github/workflows/tool-releases.yml) builds all thirteen
packaged tools on Linux for `wasm32-wasip2`, using the checked-in Rust toolchain
and Cargo lockfiles. Tree-sitter uses WASI SDK 34 with a pinned SHA-256 digest.
The resulting WASI components are portable; there is no separate tar per host OS.
CPU-specific native code is compiled and cached by SAT when a tool is registered.

Pull requests and branch pushes affecting tools or SAT build downloadable Actions
artifacts. You can also run the workflow manually from the Actions tab to build
artifacts without publishing a release.

`sat/Cargo.toml` is the source of truth for the bundle release version. Each tool
keeps its independent Cargo version (for example jaq `3.1.2`); its `tool.toml`
and local package entry in `Cargo.lock` must match. CI checks these relationships
before starting builds and rejects release tags that differ from `v` plus the
SAT version. It does not rewrite versions after a commit has been tagged.

To prepare a bundle release:

```sh
python3 scripts/release.py --set-version 0.2.0
```

This updates `sat/Cargo.toml` and its lockfile and synchronizes tool metadata with
each tool's own Cargo version. Review and commit the changes along with all tool
sources, packaging scripts, release scripts, and workflows. Then create the tag
from the committed version, without entering it a second time:

```sh
python3 scripts/release.py --tag
git push origin v0.2.0
```

The helper creates a local annotated tag and prints the matching push command;
it never commits or pushes automatically. To release the current version, skip
`--set-version` and run `--tag` after committing the release files.

When changing a tool's version, edit its component `Cargo.toml`, then run:

```sh
python3 scripts/release.py --sync  # Updates tool.toml and local Cargo.lock entries
python3 scripts/release.py         # Checks all versions without changing files
```

Only a pushed `v*` tag publishes a release. All tool builds and archive checks
must succeed first. A validation job builds SAT and registers every archive in an
isolated registry, with empty permissions, before checking its CLI help. It
validates the actual component imports and entrypoints without calling providers.
The release contains thirteen `.tar` assets plus `SHA256SUMS`;
`pdf-tools` is published as `pdftools.tar`, matching its registered name. Tags
containing a hyphen (for example `v0.1.0-rc.1`) create prereleases. Rerunning a tag
workflow replaces matching release assets and can finish interrupted uploads.
The workflow uses GitHub's `GITHUB_TOKEN`; no additional secret is required.
Only the publishing job requests `contents: write`.

Download the desired archive from Releases and register it directly, or use its
release URL with `sat register`. Each tar contains exactly `tool.wasm`,
`tool.toml`, and `defaults.toml` at its root. To verify downloaded assets, run
`sha256sum --check SHA256SUMS` in the directory containing all thirteen tar files.
The release checker can also be run locally:

```sh
python3 external_tools/check_archives.py external_tools/jaq/dist/jaq.tar
```

## Execution fuel

SAT deliberately disables Wasmtime fuel metering. Runs have no instruction-count
budget, and legacy `fuel` fields in tool archives are accepted but ignored.
This also changes the Wasmtime compilation configuration, so older compiled
cache entries are regenerated automatically. Filesystem/network grants, interface
validation, output bounds and tool-specific input/decompression limits still apply.
SAT does not impose a substitute execution deadline; terminate a non-finishing
invocation through the calling process.
