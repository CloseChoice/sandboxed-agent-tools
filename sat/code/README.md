# Tree-sitter code retrieval

`code` is a WASIp2 component containing Tree-sitter's C runtime, official Rust
bindings, and statically linked Python, Rust, JavaScript/JSX, TypeScript, and TSX
grammars. Parsing, source reads, traversal, text matching, and extraction all
happen inside the guest. It never runs a host language server, parser, shell,
or native search binary. Filesystem access uses SAT's existing WASI grants.

Build requirements are Rust with `wasm32-wasip2` and the official
[WASI SDK](https://github.com/WebAssembly/wasi-sdk/releases/tag/wasi-sdk-34)
for the C runtime and generated grammars. SDK 34 was verified on Linux x86-64.
For that platform, download and verify it from the repository root:

```sh
mkdir -p .toolchain
curl -fL -o .toolchain/wasi-sdk-34.0-x86_64-linux.tar.gz \
  https://github.com/WebAssembly/wasi-sdk/releases/download/wasi-sdk-34/wasi-sdk-34.0-x86_64-linux.tar.gz
echo 'b761e3a0721dbae9c09a0059e5fdb2bf917d1b4a8a7b430fb3b5aafb0984b2c4  .toolchain/wasi-sdk-34.0-x86_64-linux.tar.gz' | sha256sum -c -
tar -xzf .toolchain/wasi-sdk-34.0-x86_64-linux.tar.gz -C .toolchain
bash sat/code/build.sh
```

The SDK path above is the script's default. On other platforms, extract the
corresponding SDK release and set `WASI_SDK_PATH` to its directory. Build scripts
require Bash (use WSL on Windows). The produced component uses standard WASIp2
bindings; macOS/Windows host execution has not yet been verified in CI.
No SDK or compiler is needed to **run** a prebuilt component.

The build packages `sat/target/code.wasm` and installs it into `SAT_HOME`
(default `sat/target/demo/registry`). With the same registry selected:

```sh
export SAT_HOME="${SAT_HOME:-$PWD/sat/target/demo/registry}"

# Search literal text, return each enclosing function/method once.
sat/target/debug/sat code search 'TemporaryDirectory' \
  --paths /src --read ./sat::/src

# Outline a source file, then retrieve one exact symbol by its bare name.
sat/target/debug/sat code outline --paths /src/runtime.rs --read ./sat/src::/src
sat/target/debug/sat code read-symbol 'execution_error' \
  --paths /src/runtime.rs --read ./sat/src::/src

# Read the smallest enclosing function at a zero-based UTF-8 byte position.
sat/target/debug/sat code read-enclosing --position 50 \
  --paths /src/auth.py --read ./sat/code/examples::/src
```

SAT's interface currently uses operation names as the first positional argument,
followed by the optional search query/symbol. Use `sat code --help` for generated
help. Repeat `--paths` and `--read` for multiple roots. Exact-file grants work too.
Results use guest paths such as `/src`; no host path is needed in the output.
Bindings and grants are never inferred from a search query.

Operations:

| Operation | Selection and output |
| --- | --- |
| `search TEXT` | Literal, case-sensitive text hits expanded to enclosing definitions. `--mode regex` enables Rust regex syntax. |
| `outline` | Supported function, method, class/struct/enum/trait/impl declarations with labels, signatures, and ranges; no bodies. |
| `read-symbol NAME` | All declarations with this exact bare name in the selected paths. Narrow `--paths` to disambiguate. |
| `read-enclosing --position N` | Enclosing definition for a zero-based UTF-8 byte offset; requires exactly one file. |

Search first checks file contents, parses each matching file once, and groups
hits by declaration range. Repeated identical input paths are deduplicated.
`--context function` selects the smallest supported callable;
`--context outer-function` selects the outermost; `--context class` selects the
smallest enclosing class/struct/enum/trait/impl. A hit at module scope produces
`no-enclosing-definition` rather than a made-up function. Read-symbol and outline
select declarations directly; `--context` applies to search and read-enclosing.

The initial callable selection covers named functions/methods, JavaScript/TS
function expressions, generators, and arrows. Python lambdas and Rust closures
are not separate extraction units yet. Class context does not perform type
resolution or follow references to definitions.

By default, source includes adjacent comment blocks, Python decorators, and Rust
attributes. Blank lines break attachment. For Python's first method, leading
comments can sit outside its block in the syntax tree; these are included too.
Docstrings are already inside Python definitions. `--include-docs false` omits
attached prefixes. Assigned JavaScript arrows/functions retain their binding
and export declaration (a multi-binding declaration returns only the selected
declarator). The nearest owning class/impl signature is returned separately;
`--include-parent-signature false` omits it. These are explicit selection rules,
not a complete implementation of every language's documentation conventions.

Default output is readable source with a path, line range, symbol, and optional
owner signature. Add `--format json` for structured metadata:

```sh
sat/target/debug/sat code search 'TemporaryDirectory' \
  --paths /src --read ./sat::/src --format json --max-results 5
```

Each definition includes:

- `path`, `language`, `symbol`, Tree-sitter node `kind`, and `signature`;
- `source`: the exact original UTF-8 slice (or `null` for outline/size limits);
- `range`: the returned source extent, including attached prefixes;
- `definition_range`: the underlying syntax node, without attached prefixes;
- `hits`: original search ranges (or the supplied zero-width byte position);
- `file_hash`: `sha256:` plus the hash of the exact file bytes parsed;
- `parent_signature`, file-wide `parse_errors`, selected-node `complete`, and `status`.

Byte ranges are half-open `[start_byte, end_byte)`. Lines start at **1**;
columns start at **0** and count **UTF-8 bytes**, including at the exclusive end
point. The hash is metadata for a future stale-edit check; SAT does not yet
provide an edit API. Parse errors are reported even when recovery finds no
declaration. A definition can be complete while another part of its file has
parse errors. Outline results have `source: null`, `complete: false`, and
`status: outline` because they intentionally omit source bodies.

Large functions return their ranges and signature with `source: null`,
`complete: false`, and `status: size-limit`; no silent partial body is returned.
Use `--max-bytes` to adjust the per-definition limit (default 64 KiB, max 1 MiB).
Signatures exceeding 4 KiB are explicitly marked as truncated.

Limits are 4 MiB per UTF-8 source file, 128 parsed files, 128 input roots,
10,000 hits, 100 diagnostics, and 1 MiB of serialized definition records per
invocation. `--max-results` defaults to 20 (max 1,000). Limits set `truncated`
and `truncation_reason`; oversized/non-UTF-8 files have explicit diagnostics.
Diagnostics add bounded metadata beyond the definition-record budget. Use a
small result limit for previews. SAT additionally enforces its memory, compute,
and timeout limits; a limit trap fails the invocation without partial output.

Directory traversal skips hidden entries, symlinks, and special files, respects
local `.ignore`/`.gitignore`, and does not read parent/global ignore settings.
Place ignore rules inside the filesystem grant (for example `sat/.ignore`
excludes build output). An explicitly selected symlink still passes through
WASI confinement. Unsupported extensions are skipped during traversal and
rejected when supplied as an explicit file.

This implementation deliberately does not include ast-grep structural patterns,
rewriting, semantic/embedding search, a persistent index, language-server type
resolution, or C/C++ grammars. It is a stable SAT retrieval interface over the
verified static Tree-sitter grammars; those features can be added separately.
All results return through WIT, never guest stdout. `--format json` returns a
JSON object encoded as a WIT string; SAT's separate `--json` flag wraps that
string again, so it is unnecessary here.

Checks:

```sh
cargo test --locked --manifest-path sat/code/Cargo.toml --lib
python3 sat/code/test.py
```

The integration test uses a private registry and real WASIp2 components. It
checks all grammars, original source ranges, Unicode byte positions, nesting,
deduplication, decorators/attributes, hashes, parse errors, limits, traversal,
escaping symlinks, and single-file isolation. No native parser or provider is
needed at execution time.
