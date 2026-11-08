# jaq / jq-style JSON processing in WASI

`jaq` embeds the jaq filter parser/compiler/interpreter and standard library in
a WASIp2 component. It provides jq-style JSON processing, without invoking a
native jq/jaq binary. Filters, JSON parsing, file reads, and formatting execute
inside the sandbox. JSON text is returned through WIT; guest stdout is not the
data transport. No YAML/XML/CBOR format options are exposed.

Build and install from the repository root (Rust with `wasm32-wasip2`):

```sh
bash sat/jaq/build.sh
export SAT_HOME="${SAT_HOME:-$PWD/sat/target/demo/registry}"

# Files require an explicit read grant. Only guest paths go to the component.
./sat/target/release/sat jaq '.users[] | select(.active) | .name' \
  --files /data/users.json --read ./sat/jaq/examples::/data -r true

# Pipe input explicitly through SAT's bounded stdin bridge.
printf '%s\n' '{"a": [1,2,3]}' |
  ./sat/target/release/sat jaq '.a | map(. * 2)' --stdin -c true

# Inline data and a jq variable.
./sat/target/release/sat jaq '. + $n' --input 40 --argjson n=2

# Generate data without an input stream.
./sat/target/release/sat jaq '[range(3)]' -n true
```

The first command above prints `Ada`; the pipe prints `[2,4,6]`. The script
builds the **release host** by default, packages `sat/target/jaq.wasm`, and
installs it into `SAT_HOME` (default `sat/target/demo/registry`).
`SAT_BUILD_PROFILE=debug bash sat/jaq/build.sh` builds/uses the debug host instead.
Build scripts require Bash; use WSL on Windows. The component uses standard
WASIp2 interfaces; macOS and Windows execution have not yet been checked in CI.
No C compiler or WASI SDK is required to build this Rust-only tool.

The supported filter language comes from pinned `jaq-core` 3.1.2,
`jaq-std` 3.0.4, and `jaq-json` 2.0.3 through `jaq-all` 0.4.0.
It includes jq's familiar field/index access, pipelines, selection, mapping,
sorting/grouping, updates, recursion, `reduce`/`foreach`, function definitions,
path operations, `try`/`catch`, regex, math, date, and string/format filters.
This is a jq-compatible implementation, with
[upstream differences](https://github.com/01mf02/jaq), rather than libjq.

## CLI mapping

SAT generates CLI arguments from WIT. Boolean flags take a value (`-r true`),
files are supplied with repeated `--files`, and variable bindings are
`NAME=VALUE`. Clustered flags such as `-nrc` are not jq-compatible here.
Use `sat jaq --help` for the generated interface.

| jq option/concept | SAT jaq equivalent |
| --- | --- |
| `jq FILTER` reading stdin | `sat jaq FILTER --stdin` |
| `jq FILTER file1 file2` | `sat jaq FILTER --files /data/file1 --files /data/file2 --read HOST_DIR::/data` |
| Inline JSON | `--input '{"a":1}'` |
| Default identity filter | Omit `FILTER` (defaults to `.`) |
| `-n`, `--null-input` | `-n true` |
| `-R`, `--raw-input` | `-R true` |
| `-s`, `--slurp` | `-s true` |
| `-r`, `--raw-output` | `-r true` |
| `-j`, `--join-output` | `-j true` (raw output, no separator or final newline) |
| `--raw-output0` | `--raw-output0 true` (NUL separators; reject strings containing NUL) |
| `-c`, `--compact-output` | `-c true` |
| `-S`, `--sort-keys` | `-S true` |
| `-a`, `--ascii-output` | `-a true` |
| `--tab`, `--indent N` | `--tab true`, `--indent N` (0..8) |
| `--arg NAME VALUE` | `--arg NAME=VALUE` (repeatable) |
| `--argjson NAME JSON` | `--argjson 'NAME=JSON'` (repeatable) |
| `--slurpfile NAME FILE` | `--slurpfile NAME=/data/file.json` plus `--read` (repeatable) |
| `--rawfile NAME FILE` | `--rawfile NAME=/data/file.txt` plus `--read` (repeatable) |
| `--args ...` | Repeated `--args VALUE`; available through `$ARGS.positional` |
| `--jsonargs ...` | Repeated `--jsonargs JSON`; available through `$ARGS.positional` |
| `-f`, `--from-file FILE` | `--from-file /data/filter.jq` plus `--read` |
| `-e`, `--exit-status` | `-e true` (see exit-status limitation below) |

`$ARGS.named` contains the supplied named variables. `input`, `inputs`, and
`input_filename` work across the ordered input files. `$ENV` and `env` describe
only the guest environment; the host's environment and secrets are not inherited.
Use `(-1)` for a filter beginning with a minus sign; SAT's current generated
argument parser does not implement jq's complete positional/`--` syntax.

Raw input preserves line contents (including CR in CRLF) while removing the
newline delimiter. `-R true -s true` returns all raw input as a single string.
Slurping JSON collects all input documents into an array. Inputs can contain
multiple whitespace-separated JSON values, including JSON Lines.

Output is pretty JSON by default, with two spaces, no ANSI colors, and one
newline per result. Empty output stays empty. `-j true` adds no final newline.
SAT's generic `--json` flag encodes the returned WIT string again; it is normally
unnecessary because the tool's default output is already JSON.

## Boundaries and compatibility

`--stdin` is explicit. The host reads at most 8 MiB of UTF-8 text and passes it
only to the metadata-selected `input` WIT parameter. It does not inherit guest
stdio or grant any filesystem capability. Help never consumes stdin. Without
`--stdin`, use `--input`, `--files`, or `-n true`. Choose one source class;
inline/stdin input cannot be combined with file inputs.

Main inputs, `--from-file`, `--slurpfile`, and `--rawfile` all use standard WASI
filesystem access. Reading a file requires an explicit `--read` grant; an
exact-file grant does not expose its parent or siblings. Traversal and escaping
symlinks are confined by the host. Embedded policy denies HTTP, and no raw
network interfaces are imported. The component never writes files. Shell output
redirection remains a host-shell operation, as with native jq.

The wrapper accepts JSON, rather than jaq's XJON input extensions, and rejects
byte strings and non-string object keys in JSON output. It maps NaN to `null`
and clamps infinities to finite limits as jq does. jaq can still differ in
numeric precision and language edge cases.

Current limitations:

- Module `import`/`include`, `-L`, `--stream`, `--stream-errors`, JSON sequences
  (`--seq`), test-suite mode, and in-place modification are not exposed.
- Inputs and outputs are buffered, with all input JSON parsed before execution.
  This can reject malformed trailing data that lazy native jq would not consume.
  `--unbuffered` and interactive `input` prompts are not supported.
- Errors discard accumulated output and use SAT's common error exit status **1**.
  With `-e true`, false/null or no output fails, but the false/null output is not
  printed; jq's distinct statuses 4/5/etc. are not preserved. `halt` stops cleanly;
  a nonzero `halt_error` is reported as a tool error. Guest `debug`/`stderr` output
  is not forwarded to host stderr.
- The CLI and WIT text boundary require UTF-8. Arbitrary binary raw input/output
  and color-control options are not exposed.

Limits: 64 KiB filter source, 128 input files, 8 MiB aggregate input-file/inline
data, 100,000 input/output values, and 8 MiB output. Variable values supplied
directly as CLI arguments also remain subject to the host memory/fuel limits.
The host additionally enforces a 64 MiB guest memory limit, 1 billion fuel, and
its execution timeout. Limit failures are explicit, with no silent partial data.

## Checks

```sh
cargo test --locked --manifest-path sat/jaq/Cargo.toml --lib
cargo test --locked --manifest-path sat/Cargo.toml
python3 sat/jaq/test.py
```

Tests execute the actual component in a private registry. When native jq is
available, they compare filter and formatting results directly against it.
They also verify stdin limits/UTF-8, empty and joined output, module rejection,
environment isolation, file-variable grants, traversal/symlink denial,
single-file isolation, and invalid stdin metadata rejection.
