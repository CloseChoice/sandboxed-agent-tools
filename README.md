# Sandboxed file search and FFF fuzzy matching

## Set up another machine

Clone this repository and run commands from its root. The scripts use Bash;
on Windows, use WSL. Install Rustup, Wasmtime 48.0.2, and Python 3 plus grep
for the CLI tests. Building the fuzzy library additionally requires Node.js
24.12.0 (recorded in `.nvmrc`), npm, and a native C linker/build toolchain.
`rust-toolchain.toml` pins Rust 1.98.1 and both WebAssembly targets; Rustup
downloads these on first use. Initial dependency installation needs internet
access.

```sh
# Build and test the file-content search CLI
bash scripts/build-cli.sh
python3 tests/cli.test.py

# Build and test the fuzzy matching library and its JavaScript host
npm ci
npm run build
npm test
npm run demo
```

Source, WIT interfaces, build scripts, tests, dependency lockfiles, and the
patched `vendor/neo_frizbee` dependency are tracked in Git. `.toolchain/`,
`target/` (including nested crates), `node_modules/`, and `dist/` are generated
or machine-local and are excluded. `scripts/env.sh` uses an optional local
toolchain if present and otherwise uses the machine's installed tools.

## Search file contents with Wasmtime

`dist/file-search.wasm` is a WASIp2 CLI component that opens and reads files
inside Wasm and searches their contents. Supply a literal, case-sensitive
search string and one or more file paths. Results are `filename:line:text`,
with line numbers starting at 1.

Build from the project root (Rust and Wasmtime must be installed):

```sh
. scripts/env.sh
rustup target add wasm32-wasip2
bash scripts/build-cli.sh
```

Grant access to a specific folder and search files within it:

```sh
wasmtime run --dir ./src::/data dist/file-search.wasm 'fn search' /data/lib.rs
```

Here `./src` is the host folder; `/data` is its name inside the component.
To search multiple files:

```sh
wasmtime run --dir ./src::/data dist/file-search.wasm 'use' /data/lib.rs /data/another.rs
```

Replace these paths with existing files. Output looks like:

```text
/data/example.rs:12:use std::fs::File;
```

Use grep-style context options before the query:

```sh
# Two lines before and after each match
wasmtime run --dir ./src::/data dist/file-search.wasm -C 2 'fn search' /data/lib.rs

# Three lines before and one line after each match
wasmtime run --dir ./src::/data dist/file-search.wasm -B 3 -A 1 'fn search' /data/lib.rs
```

`-A N` shows lines after, `-B N` shows lines before, and `-C N` sets both.
Counts must be non-negative integers; attached forms such as `-C2` also work.
Explicit `-A` and `-B` override `-C` for their respective side regardless of
option order. Repeating an option uses its last value.
Matching lines use `filename:line:text`; surrounding lines use
`filename-line-text`. Overlapping or adjacent groups are merged without
duplicating lines, and separate groups are divided by `--`.

The filesystem boundary is enforced by Wasmtime's WASI implementation:
the component can access the **entire granted folder**, including subfolders,
but cannot open files outside it using absolute paths, `..`, or escaping
symlinks. With no `--dir`, it has no filesystem directory access. Passing a
file path as a CLI argument does not grant access to that file. Grant only
the folder you intend to expose. The searcher opens files for reading;
`--dir` itself is not a read-only permission restriction.
See [WASI filesystem capabilities](https://docs.wasmtime.dev/api/wasmtime_wasi/struct.WasiCtxBuilder.html).

Matching is line-based and expects UTF-8 text; each matching line is printed
once. Paths are explicit files, with no recursive discovery or glob expansion
inside Wasm. An empty query matches every line. Use `--` before the query to
search for a query beginning with `-`, including `--help` or `-h`. Exit status is 0 for matches
(or help), and 1 for no matches or errors; errors are printed to stderr.
An error in one file does not prevent searching the remaining files, so an
unsuccessful run may still print partial results.

Verify the built component with Wasmtime (tested with 48.0.2):

```sh
python3 tests/cli.test.py # Requires grep for context-output comparisons
```

This checks matching, line numbers, grep-compatible context, Unicode, and denial of reads without a
grant or outside the granted folder, including traversal and symlink escapes.

## FFF fuzzy matching library

A stateful WebAssembly Component Model library using **neo_frizbee 0.11.0**,
the fuzzy matcher used by [FFF](https://github.com/dmtrKovalenko/fff).
The host supplies strings; the component retains them and returns ranked IDs,
scores, and highlight ranges. It has **no host imports**, including no WASI
filesystem access. It can match arbitrary strings, not only file paths.

This is a wrapper around FFF's underlying matcher, not a port of its complete
search SDK. It does not implement FFF's query language, Git/frecency ranking,
file discovery, or content search. Matching is case-insensitive, with bonuses
for matching case when the query contains uppercase. Unicode matching is
enabled for all queries to keep highlighting correct. Wasm uses the upstream
portable scalar backend; native FFF SIMD performance is not implied.

## Build

Install the prerequisites above. An existing installation in `.toolchain`
is optional. Verified with Rust 1.98.1 and Node 24.12.0.
From this directory:

```sh
. scripts/env.sh
npm ci --cache .toolchain/npm-cache
npm run build
npm run demo
```

The build compiles to `wasm32-unknown-unknown` and uses Jco to package the core
module as a component. No WASI adapter is needed. It also extracts the final
WIT and fails if the resulting component has any imports.

- `dist/fff-fuzzy.wasm`: portable component artifact.
- `dist/component.wit`: actual imports and exports of the built artifact.
- `dist/js/`: Jco-generated JavaScript bindings and core Wasm for Node/browser hosts.
- `wit/world.wit`: public source interface.

## Use

```js
import { search } from './dist/js/fff-fuzzy.js';

const paths = ['src/main.rs', 'docs/readme.md'];
const finder = new search.Finder();
finder.setCandidates(paths);
const hits = finder.search('main', 20, 0);
// Each hit: { id, score, highlights: [{ start, end }] }
console.log(hits.map(hit => ({ path: paths[hit.id], ...hit })));
finder[Symbol.dispose]();
```

After building, `. scripts/env.sh` followed by `npm run demo` runs a small
example from `examples/search.mjs`. This is a library component with custom
exports, not a WASI CLI command; plain `wasmtime run dist/fff-fuzzy.wasm` is
not its entry point. JavaScript is only the example host:
other Component Model runtimes can consume the component using its WIT.

To supply your own query and candidates:

```sh
npm run demo -- main src/main.rs docs/readme.md
```

These arguments are candidate strings; the example does not read those files.

### Run with Wasmtime

Wasmtime can call the `match-paths` export directly with `--invoke`; no custom
host is required for a single search. From the project root, after building
(verified with Wasmtime 48.0.2):

```sh
wasmtime run \
  --invoke 'match-paths(["src/main.rs", "docs/readme.md"], "main", 20, 0)' \
  dist/fff-fuzzy.wasm
```

Output:

```text
ok([{id: 0, score: 48, highlights: [{start: 4, end: 8}]}])
```

Arguments are candidate strings, query, result limit, and allowed typos (0–2).
The result ID indexes the supplied candidates. This matches strings without
reading files, so no `--dir` grant is needed. The invocation uses
[Wasmtime's WAVE syntax](https://docs.wasmtime.dev/cli-options.html#running-custom-component-exports).

To retain a `finder` across multiple searches in a Wasmtime application,
generate host bindings from `wit/world.wit` and instantiate with an empty
component linker; no WASI context or directory grants are required.

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
  original string, not JavaScript UTF-16 indices.
- Limits per finder: 1,000,000 candidates, 64 MiB of candidate UTF-8 text,
  4,096 UTF-8 bytes per candidate, and 256 UTF-8 bytes per query.
  Errors leave previously stored candidates intact. These checks happen after
  ABI argument decoding; hosts should additionally set runtime memory/fuel
  limits when accepting untrusted input.

## Verify

```sh
. scripts/env.sh
npm run build
npm test
```

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
read path. Rust and npm dependency lockfiles make subsequent builds repeatable.
