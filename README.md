# Safe agent tools

- [FFF fuzzy matching library](fuzzy-search/README.md): a self-contained WebAssembly fuzzy matcher. All source, dependencies, build configuration, scripts, tests, and examples are in `fuzzy-search/`.
- [File-content search CLI](#search-file-contents-with-wasmtime): a sandboxed literal search tool in `cli/`.

## Set up another machine

For the file-content search CLI, run commands from the repository root. Install
Rustup, Wasmtime 48.0.2, and Python 3 plus grep for the CLI tests. The scripts
require Bash; on Windows, use WSL. `rust-toolchain.toml` pins Rust 1.98.1 and
the WebAssembly targets, which Rustup downloads on first use.

```sh
bash scripts/build-cli.sh
python3 tests/cli.test.py
```

For the fuzzy matcher, see its [setup and usage instructions](fuzzy-search/README.md):

```sh
cd fuzzy-search
. scripts/env.sh
cargo install --locked wasm-tools --version 1.254.0
bash scripts/test.sh
cargo run --locked --example search -- main src/main.rs docs/readme.md
```

Build output and optional local toolchains (`target/`, `dist/`,
and `.toolchain/`) are ignored by Git. Dependency lockfiles are tracked.

## Search file contents with Wasmtime

`dist/file-search.wasm` is a WASIp2 CLI component that opens and reads files
inside Wasm and searches their contents. Supply a literal, case-sensitive
search string and one or more file paths. Results are `filename:line text`,
with line numbers starting at 1. Each result includes the full matching line.
Use `-C N` for surrounding context, `-A N` for following lines, or `-B N`
for preceding lines.

Build from the project root (Rust and Wasmtime must be installed):

```sh
. scripts/env.sh
rustup target add wasm32-wasip2
bash scripts/build-cli.sh
```

Grant access to a specific folder and search files within it:

```sh
wasmtime run --dir ./cli/src::/data dist/file-search.wasm 'fn run' /data/main.rs
```

### Understanding the command and `::`

For example, search with two lines of context on each side:

```sh
wasmtime run --dir ./cli/src::/data dist/file-search.wasm -C 2 'fn run' /data/main.rs
```

| Part | Meaning |
| --- | --- |
| `wasmtime run` | Run a WebAssembly program using Wasmtime. |
| `--dir ./cli/src::/data` | Make the host directory `./cli/src` accessible inside the program as `/data`. |
| `dist/file-search.wasm` | The compiled search program to run. |
| `-C 2` | Print up to two lines before and after each matching line. |
| `'fn run'` | The literal search text. Shell quotes keep the space inside one argument. |
| `/data/main.rs` | The file to search, using its path inside the Wasm program. |

The `::` separates the **host directory** (left) from the **guest directory**
(right): `--dir HOST_DIRECTORY::GUEST_DIRECTORY`. Here “host” means your
machine and “guest” means the running Wasm program. With this mapping:

```text
Your machine:  ./cli/src/main.rs
Inside Wasm:  /data/main.rs
```

`./cli/src` is relative to the directory where you run the command (the
repository root in these examples). `/data` is a name you choose for the
directory inside Wasm; it does not need to exist on your machine. The mapping
exposes the existing files without copying them. If you change `/data` to
`/source`, also change the search argument to `/source/main.rs`.

`--dir` belongs to Wasmtime and goes before the `.wasm` file. Everything after
the `.wasm` file is passed to the search program. See the
[Wasmtime CLI documentation](https://docs.wasmtime.dev/cli-options.html).

To search multiple files:

```sh
wasmtime run --dir ./cli/src::/data dist/file-search.wasm 'use' /data/main.rs /data/another.rs
```

Replace these paths with existing files. Output looks like:

```text
/data/example.rs:12 use std::fs::File;
```

### Context options: `-C`, `-A`, and `-B`

Run these commands from the repository root after building the CLI. Place
context options **after `dist/file-search.wasm` and before the query**:

| Option | Lines included around each match |
| --- | --- |
| `-C N` | Up to N lines before and N lines after |
| `-A N` | Up to N lines after |
| `-B N` | Up to N lines before |

Each command below searches the existing `cli/src/main.rs` file for `fn run`:

```sh
# Two lines before and after each match
wasmtime run --dir ./cli/src::/data dist/file-search.wasm -C 2 'fn run' /data/main.rs

# Three lines after each match
wasmtime run --dir ./cli/src::/data dist/file-search.wasm -A 3 'fn run' /data/main.rs

# Three lines before each match
wasmtime run --dir ./cli/src::/data dist/file-search.wasm -B 3 'fn run' /data/main.rs

# Three lines before and one line after each match
wasmtime run --dir ./cli/src::/data dist/file-search.wasm -B 3 -A 1 'fn run' /data/main.rs

# Compact form: two lines on each side
wasmtime run --dir ./cli/src::/data dist/file-search.wasm -C2 'fn run' /data/main.rs
```

Counts must be non-negative integers; attached forms such as `-C2` also work.
Explicit `-A` and `-B` override `-C` for their respective side regardless of
option order: `-C 2 -A 0` shows two lines before and none after. Repeating an
option uses its last value. With no context options, only matching lines are
printed.
Matching lines use `filename:line text`; surrounding lines use
`filename-line-text`. Overlapping or adjacent groups are merged without
duplicating lines, and separate groups are divided by `--`.

### Filesystem access and exit codes

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

## Proposed SAT tool package and CLI design

Proposed behavior, not implemented yet. One `.wasm` component is the authoritative
package; separate WIT/TOML files are extractable inspection copies.

| Contents | Location |
| --- | --- |
| Code, imports, exports, and interface types | Standard component encoding; used to extract WIT |
| Name, version, description, entrypoint, CLI argument/option descriptions, defaults, examples | Required `sat.tool.v1` custom section, containing UTF-8 TOML |
| Permanent HTTP and file/folder restrictions | Optional `sat.policy.v1` custom section, containing UTF-8 TOML |

Both custom sections belong on the outermost component. SAT reads them without
executing guest code. Ordinary Wasmtime does not enforce SAT metadata; SAT's
host implements the restrictions. Extracted WIT may not preserve source comments.

### CLI metadata: `sat.tool.v1`

Given the exported WIT function:

```wit
export run: func(query: string, count: u32) -> result<string, string>;
```

The tool metadata could be:

```toml
name = "brave-search"
version = "0.1.0"
description = "Search the web using Brave."
entrypoint = "run"

[[arguments]]
name = "query"
help = "The search query."

[options.count]
default = 5
help = "Maximum number of results."

[[examples]]
command = 'sat brave-search "WASI components" --count 10'
```

Argument/option names match WIT parameter names; types come from WIT. No `bind`
or `type` field is needed. SAT supplies values in WIT parameter order. Parameters
without defaults are required. Keyed option tables let the TOML parser reject
duplicate definitions such as `[options.count]`.

SAT generates `sat brave-search --help` and `--help --json` without executing the
guest. SAT reserves these help flags.

### Permanent restrictions: `sat.policy.v1`

```toml
[[http]]
methods = ["GET"]
origin = "https://api.search.brave.com"
paths = ["/res/v1/web/search"]
```

CLI input may change the search query, but cannot widen the allowed method,
origin (scheme, host, port), or exact path. Query parameters are separate from
path matching. Redirect destinations must satisfy the same policy or be denied.
File/folder restrictions follow the same ceiling model; their TOML schema remains
to be specified.

WIT-defined named resources can be bound at invocation time, for example
`sat tool --documents=./documents --templates=./templates`. SAT needs explicit
WIT-to-CLI resource conventions; standard WASI imports alone do not identify
these names or read/write modes. Host implementations enforce resource access.
No duplicate policy declaration is needed unless extra restrictions are wanted.
Ordinary path strings do not grant access.

Effective authority is the intersection of invocation grants, package restrictions,
and host policy. **Missing policy means no additional package restrictions**, not
automatic authority. CLI input cannot override embedded restrictions.

### Validation and integrity

Before execution, SAT must:

1. Verify the final binary against a trusted registry digest or authenticated
   signature. Hash/sign **after** embedding both sections. Replacing or removing
   metadata changes package identity and invalidates previous verification and
   approval; a replaceable adjacent hash file is insufficient.
2. Validate the component and read its actual imports, exports, and types.
3. Require one `sat.tool.v1` section and at most one `sat.policy.v1` section at
   the outermost level. Reject duplicates, invalid TOML, unsupported metadata
   versions, unknown fields, and invalid values. Invalid policy is never absence.
4. Validate the entrypoint, argument/option names, defaults, unique mappings,
   reserved flags, and parameter coverage against the component's types. If an
   external WIT file is supplied, compare resolved interfaces structurally,
   not by comparing formatted `wasm-tools` output.
5. Reject unsupported host imports and incompatible policy declarations. Imports
   reveal required interfaces, not which URLs or files guest code will use.
6. Resolve approved invocation grants, apply permanent restrictions and host
   policy, then instantiate with restricted resources and host functions.
   Enforce HTTP rules on every request and filesystem rights on every operation.
