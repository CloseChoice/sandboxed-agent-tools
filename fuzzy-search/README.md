# FFF fuzzy matching library

A stateful WebAssembly Component Model library using **neo_frizbee 0.11.0**,
the fuzzy matcher used by [FFF](https://github.com/dmtrKovalenko/fff).
The host supplies strings; the component retains them and returns ranked IDs,
scores, and highlight ranges. It has **no host imports**, including no WASI
filesystem access. It can match arbitrary strings, not only file paths.

For file-content results in `filename:line text` format with grep-style
`-C`, `-A`, and `-B` context, use the separate file-content CLI. See its
[context options and example commands](../README.md#context-options--c--a-and--b).

This is a wrapper around FFF's underlying matcher, not a port of its complete
search SDK. It does not implement FFF's query language, Git/frecency ranking,
file discovery, or content search. Matching is case-insensitive, with bonuses
for matching case when the query contains uppercase. Unicode matching is
enabled for all queries to keep highlighting correct. Wasm uses the upstream
portable scalar backend; native FFF SIMD performance is not implied.

## Build

All commands below run from `fuzzy-search/`. Install Rustup and a native C
linker/build toolchain. The scripts require Bash and grep; on Windows, use WSL.
`rust-toolchain.toml` pins Rust 1.98.1 and `wasm32-unknown-unknown`; Rustup
installs them on first use. Dependency installation needs internet access.

```sh
cd fuzzy-search # from the repository root
. scripts/env.sh
cargo install --locked wasm-tools --version 1.254.0
bash scripts/build.sh
```

Alternatively, install the matching binary from the official
[wasm-tools releases](https://github.com/bytecodealliance/wasm-tools/releases/tag/v1.254.0)
and put it on `PATH` or in `.toolchain/bin/`. An existing Rust installation
under `.toolchain/` is optional; `scripts/env.sh` otherwise uses the machine's
installed tools.

The build compiles the Rust wrapper and vendored matcher to a core Wasm module,
then packages it with `wasm-tools component new`. It validates the component,
extracts its WIT, and fails if any host imports are present. No WASI adapter is
needed.

- `dist/fff-fuzzy.wasm`: portable component to run with Wasmtime.
- `dist/component.wit`: interface extracted from the built artifact.
- `wit/world.wit`: public source interface.

## Run with Wasmtime

Install [Wasmtime 48.0.2](https://github.com/bytecodealliance/wasmtime/releases/tag/v48.0.2)
and put `wasmtime` on `PATH`. After building, call the `match-paths` export:

```sh
wasmtime run \
  --invoke 'match-paths(["src/main.rs", "docs/readme.md"], "main", 20, 0)' \
  dist/fff-fuzzy.wasm
```

Output:

```text
ok([{id: 0, score: 48, highlights: [{start: 4, end: 8}]}])
```

The four arguments are candidate strings, query, result limit, and allowed
missing query characters (0–2). Each result ID indexes the supplied list.
For example, allow one missing character:

```sh
wasmtime run \
  --invoke 'match-paths(["schema.rs", "README.md"], "schexma", 20, 1)' \
  dist/fff-fuzzy.wasm
```

The candidates are strings, not files to open. No `--dir` grant is needed.
This is a library component, so use `--invoke` rather than a plain
`wasmtime run dist/fff-fuzzy.wasm`. Arguments use
[Wasmtime's WAVE syntax](https://docs.wasmtime.dev/cli-options.html#running-custom-component-exports).

## Use from a Rust application

For repeated queries over the same candidates, embed Wasmtime and retain a
`finder` resource. The runnable [Rust example](examples/search.rs) creates a
finder, sets candidates, searches, prints results, and drops the resource:

```sh
. scripts/env.sh
cargo run --locked --example search
cargo run --locked --example search -- main src/main.rs docs/readme.md
```

The first argument is the query; remaining arguments are candidate strings.
With no candidates it uses a small built-in list. This example uses a result
limit of 20 and a typo budget of 0. It loads `dist/fff-fuzzy.wasm`, so build the
component first. Cargo downloads the Wasmtime Rust dependency automatically;
the standalone `wasmtime` executable is only needed for the commands above.

[Host setup](examples/support/mod.rs) generates bindings from `wit/world.wit`
with `wasmtime::component::bindgen!` and instantiates the component with an
empty linker. No WASI context or filesystem grants are used. Keep the store
and finder alive to call `search` repeatedly; call `set-candidates` to replace
the list, and `resource_drop` when finished.

## Contract

- Each `finder` owns an independent list. Drop it to release that list.
- `set-candidates` replaces the list atomically on success. IDs are zero-based
  input positions, including duplicates, and reset after each replacement.
- Search sorts by descending score, breaking ties by original candidate order.
- `max-typos` accepts 0, 1, or 2. Upstream defines this as allowed missing query
  characters, not a general edit-distance guarantee. Use 0 for strict fuzzy
  subsequences. The query is a literal fuzzy string, not glob/query syntax.
- Empty queries return input order, zero scores, and no highlights. A zero
  result limit returns an empty list.
- Highlights are sorted, merged, half-open **UTF-8 byte ranges** into the
  original string.
- Limits per finder: 1,000,000 candidates, 64 MiB of candidate UTF-8 text,
  4,096 UTF-8 bytes per candidate, and 256 UTF-8 bytes per query.
  Errors leave previously stored candidates intact. These checks happen after
  ABI argument decoding; hosts should additionally set runtime memory/fuel
  limits when accepting untrusted input.

## Verify

```sh
. scripts/env.sh
bash scripts/test.sh
```

The test script builds a fresh component, then runs Rust unit tests and
Wasmtime integration tests with `cargo test --locked`. No standalone Wasmtime
executable is required for these tests.

Tests compare native and Wasm scores/order across 24 query/budget
combinations. They also cover matching, typo budgets, state replacement, duplicate ordering,
Unicode highlights, resource isolation, and the absence of component imports.

## Provenance

The matcher is vendored at version 0.11.0 with a focused Unicode prefilter fix;
see [vendor/neo_frizbee/PATCHES.md](vendor/neo_frizbee/PATCHES.md). Its upstream
MIT license is retained. See [FFF's dependency declaration](https://github.com/dmtrKovalenko/fff/blob/main/Cargo.toml)
and [matching integration](https://github.com/dmtrKovalenko/fff/blob/main/crates/fff-core/src/score.rs).
Native/Wasm parity tests use this same patched matcher. The wrapper follows
FFF's uppercase scoring bonuses, but does not reproduce
FFF's additional file-specific ranking. `safe_read` selects upstream's safer
read path. Cargo.lock pins Rust dependencies for repeatable builds.
