# SAT MVP

A local component runner with embedded metadata, generated CLI/help, and enforced
HTTP and file/folder restrictions. No external Wasmtime CLI is needed to run tools.

## Try it

From the repository root (Rust and Python 3 required):

```sh
python3 sat/demo.py
export SAT_HOME="$PWD/sat/target/demo/registry"
export PATH="$PWD/sat/target/debug:$PATH"

sat list
sat random
sat httpbin --help
sat httpbin 'https://httpbin.org/get?hello=world' --allow-http
```

The demo script builds and installs local examples; it does not send HTTP requests.
On Windows, set the same `SAT_HOME` and add `sat/target/debug` to `PATH` using your
shell's environment syntax. Without `SAT_HOME`, the registry is `~/.sat`.

HTTP is denied until `--allow-http` is supplied. That flag cannot widen the embedded
policy: this package allows only GET to `https://httpbin.org/get`. For example,
`https://example.com/get` and `https://httpbin.org/post` are denied. The URL's query
parameters may vary. Redirects are not followed; any new guest request is checked.

Read a folder, read one file, and write one file:

```sh
sat files read --path=hello.txt \
  --documents=sat/target/demo/documents \
  --input=sat/target/demo/input.txt \
  --output=sat/target/demo/output.txt

sat files input \
  --documents=sat/target/demo/documents \
  --input=sat/target/demo/input.txt \
  --output=sat/target/demo/output.txt

sat files output --text='Hello from SAT' \
  --documents=sat/target/demo/documents \
  --input=sat/target/demo/input.txt \
  --output=sat/target/demo/output.txt
```

All declared bindings are required, even if an operation does not use them. Files
and directories must already exist. The documents policy is read-only: changing
`read` to `write-dir --text=oops` is denied even though the WIT interface includes
that function. The output file is writable without exposing its parent directory.
File/folder errors returned to guests do not reveal host paths.

## Standard filesystem grants

Use `--read PATH` or `--read-write PATH` for components that use standard WASI
filesystem calls, including Python. Repeat flags for multiple paths:

```sh
sat python --read ../data.csv --read-write ./results -- \
  -c 'print(open("../data.csv").read()); open("results/output.txt", "w").write("done")'
```

Optionally rename a file or directory inside the guest with `HOST::GUEST`:

```sh
sat python sat/python_example2.py --read sat/python_example.py::sat/python_example2.py
```

Only the host path must exist; the guest alias is virtual and is not created on
host disk. Relative guest aliases resolve from the guest working directory;
absolute guest aliases start at `/`. The permission applies equally to aliases.

Grant paths must exist. Files expose only that file; directories expose their
contents recursively. Relative paths retain their relationship to the invocation
working directory, including `..`; no `HOST::GUEST` mapping is needed. New files
can be created inside writable directories. Single-file grants permit reading,
writing, truncating and appending, but not deleting/renaming the file or listing
its parent. Directory symlinks cannot escape the granted directory.

The host pins single-file handles and exposes them through a small WASI adapter;
it never preopens their parent directories. Directory access uses Wasmtime's
read-only/read-write preopens. Grants are additive; granting a writable directory
also permits writing its children. Relative paths use a synthetic guest working
directory, exposed as `PWD`; the host environment is not inherited. Guests must
initialize their working directory from `PWD` (the included Python wrapper does);
WASI does not provide a host-controlled working-directory setting. Absolute
POSIX grant paths are preserved; use relative grants on Windows.

Existing `files`/`folders` policy lists describe named WIT bindings. If either is
present (including `[]`), generic `--read`/`--read-write` grants are rejected:
use those named bindings instead. This prevents generic grants from bypassing
package restrictions. With neither category present, the package imposes no
filesystem ceiling, but every invocation still requires explicit grants.

## Package your own component

```sh
sat pack component.wasm --metadata tool.toml --policy policy.toml -o tool.wasm
sat install tool.wasm
sat TOOL-NAME --help
sat TOOL-NAME --help --json
sat inspect TOOL-NAME          # tool metadata
sat inspect TOOL-NAME --policy # permanent restrictions
sat inspect TOOL-NAME --wit    # standalone WIT extracted from the binary
```

`pack` adds outermost `sat.tool.v1` and optional `sat.policy.v1` custom sections.
It rejects already-packaged input. Omit `--policy` for no package-specific ceiling;
HTTP still needs `--allow-http`, and files/folders still need explicit bindings.
`install` explicitly trusts a local package and pins its complete SHA-256 digest.
Replacement requires `sat install tool.wasm --replace`. Every invocation, help,
and inspection verifies that digest before loading the component.

Compiled code is cached under `SAT_HOME/.cache`; source package hashes are still
verified on every invocation. The registry and compilation cache are trusted local state. This is not publisher authentication: anyone
able to modify both its registration record and binary can replace the package.
Remote catalogs and signatures are not implemented.

## Metadata

Example `tool.toml` for `run(query: string, count: u32)`:

```toml
name = "search"
version = "0.1.0"
description = "Search some text."
entrypoint = "run"

[[arguments]]
name = "query"
help = "Search query."

[options.count]
default = 5
help = "Maximum results."
```

An option may also specify `short = "c"` to expose `-c` alongside its long name.
SAT rejects duplicate short flags and reserves `-h` for help.
SAT matches names to WIT parameters and derives types. This becomes
`sat search 'hello' --count 10`. Scalar parameters without defaults are required.
WIT `option<T>` parameters are optional: absence becomes `none`, while a supplied value becomes `some(value)`.
Optional positional arguments must follow required ones. A top-level metadata field
`exactly_one_of = ["code", "script"]` requires precisely one of those optional
parameters (without defaults); Python uses this for `-c CODE` versus `script.py`.
Place this field before TOML sub-tables.
Only root function exports are supported. Inputs can be string, bool, u32/u64,
s32/s64, f32/f64, `option<T>` of those types, or `option<list<string>>`. A
`list<string>` option accepts repeated flags and preserves their order. Booleans use `--option true`/`false`.
Results are printed as text or JSON (`--json` forces JSON). WIT errors become
nonzero exits.

Tools use 50 million Wasmtime fuel by default. Metadata may request a larger
budget up to the host ceiling of 1 billion; SAT rejects values outside the
supported range and enforces the ceiling at execution. Use the optional
top-level `fuel` field for a tool with a documented compute need; for example,
`sat/ripgrep/tool.toml` requests the maximum for regex compilation and recursive traversal.

Help is generated without executing the guest or opening its bound files.
`pack`/`install` validate the component, metadata, defaults, policy, and import
signatures. Invalid/duplicate metadata is rejected; an absent policy is permitted.

## File/folder WIT convention

A named imported interface becomes `--NAME=HOST_PATH`. Its operations determine
whether the resource can be read or written. For example:

```wit
world example {
    import documents: interface {
        %list: func(path: string) -> result<list<string>, string>;
        read: func(path: string) -> result<string, string>;
    }
    import output: interface {
        write: func(contents: string) -> result<_, string>;
    }
    export run: func() -> result<string, string>;
}
```

A directory interface has `%list(path)` and `read(path)` and/or
`write(path, contents)`. A file interface has `read()` and/or `write(contents)`.
All paths/content are strings; writes return `result<_, string>`, reads return
`result<string, string>`. `%list` escapes the WIT keyword; the function name is
`list`. Bindings are text-only for this MVP. Directory paths are confined by
`cap-std`, including traversal and escaping symlinks. Writes replace file contents;
there are no delete/rename operations yet.

Optional `policy.toml` narrows what these interfaces can do:

```toml
[[http]]
methods = ["GET"]
origin = "https://httpbin.org"
paths = ["/get"]

[[folders]]
name = "documents"
access = ["read"]

[[files]]
name = "output"
access = ["write"]
```

An omitted category adds no restriction; an explicit empty list denies that
category. Within a present category, only listed resources/methods/routes are
allowed. CLI bindings never override these limits. With no HTTP policy,
`--allow-http` permits arbitrary HTTP(S) destinations, including local addresses;
there is no public-internet-only mode yet.

The host inherits no environment or guest stdio, grants no raw sockets, and exposes standard
WASI filesystem paths only through explicit grants, and uses WASI's normal clock/random services. The MVP
uses a 64 MiB guest memory limit, fuel, and a 30-second async execution timeout;
these are not comprehensive limits on host I/O buffering. Pipelines, binary file
resources and external WIT comparison are not implemented.
The earlier broader permission sketch is retained in `src/permissions.rs` but is
not wired into this runner.

## Checks

```sh
cargo test --locked --manifest-path sat/Cargo.toml
python3 sat/test.py
```

Integration checks build actual components and use a local HTTP server: no remote
provider is required. They cover grants and route restrictions, read-only policy,
file/folder isolation, environment/socket denial, help, WIT extraction, and digest
mismatches. Build dependencies may need downloading on the first run.

## Python

See [`python-example`](../python-example/README.md) for a separately built CPython
WASI tool: `sat python -c "print(1 + 2)"`. It uses the same packaging, metadata,
and sandbox; no host-Python execution path is added to SAT.

### Build a Python environment

Configure one or more trusted package indexes in `SAT_HOME/python-registries.toml`:

```toml
[registries.pypi]
index_url = "https://pypi.org/simple"

[registries.anaconda]
index_url = "https://pypi.anaconda.org/conda-forge/simple"

[registries.company]
index_url_env = "SAT_JFROG_PYPI_URL"
```

The environment variable can contain the complete authenticated HTTPS index URL;
it is read by the host-side download step only. Do not put passwords directly in
the TOML file. Install requirements by passing package specifiers:

```sh
sat python-venv install 'requests==2.32.3' --name python_requests
sat python-venv install 'idna==3.10' --registry company --name python_idna
sat python_requests -c 'import requests; print(requests.__version__)'
```

SAT downloads universal wheels only (`py3-none-any`), installs their files into a
fresh temporary tree, provisions the pinned `componentize-py` 0.25.0 builder in
`SAT_HOME/.cache/python-build-tools` on first use, then runs it. The builder
pre-initializes the Python guest under Wasmtime with the temporary app and package
paths mapped in. Package imports and `.pth` startup hooks therefore run in that
WASI guest; no registry credentials or network are passed to it. SAT validates
the produced component with its normal package loader, registers it, and removes
the temporary tree on success or failure. `SAT_BUILD_TMP` selects the temporary
directory; `SAT_PYTHON_BUILD_TIMEOUT_SECS` sets the per-process limit (default
600 seconds); `SAT_COMPONENTIZE_PY` can point to a separately installed pinned
componentize-py 0.25.0 builder. SAT bootstraps its default builder from public
PyPI using an isolated Python environment, without passing package-index
credentials to that bootstrap. The componentize-py version is pinned, but its
transitive Python dependencies are currently resolved by pip at bootstrap time
and are not locked by hashes; those dependencies also run in the host build
process and belong to its trusted supply chain. A host Python 3 installation
with `pip` and `venv` is required; SAT uses it for wheel handling and builder
provisioning, but does not execute the installed package code with host Python.
The generated tool also embeds Python's deny-HTTP policy, so `--allow-http` does
not grant network access to installed packages.

This first implementation intentionally rejects direct package URLs and pip
options, and does not build source distributions or native extensions. It does
not run PEP 517 build backends. It mitigates startup payloads such as malicious
`.pth` files by executing them in the componentizer's WASI initialization guest,
but it does not make the package safe to run: the generated component remains
untrusted and needs narrow runtime grants. Wheel resolution/download still occurs
on the host, so the configured registry is trusted to serve the selected wheel
files. **The pinned componentizer itself currently runs as a host process.** It
receives a clean environment and the staged venv path, but still has the host
user's operating-system authority. A vulnerability in the componentizer or its
host-side dependencies could therefore compromise the host; moving the builder
inside an outer WASI sandbox is a future hardening step. The WASI
pre-initialization is resource-limited by a process timeout, but
SAT does not yet enforce a build memory or disk quota. It caches completed builds
by tool name, sorted requirement strings, registry identity and downloaded wheel
hashes; it still resolves/downloads wheels before checking that cache.

## JSON queries with jaq

The [`jaq`](jaq/README.md) WASIp2 component runs jq-style filters with jaq's
standard library. It supports inline JSON, explicitly piped stdin, granted files,
variables, raw/slurp input, and pretty/compact/raw output:

```sh
bash sat/jaq/build.sh # Builds the release host and component
printf '%s\n' '{"items":[1,2,3]}' | sat jaq '.items | map(. * 2)' --stdin -c true
sat jaq '.users[] | select(.active) | .name' \
  --files /data/users.json --read ./sat/jaq/examples::/data -r true
```

Select the build script's `SAT_HOME` (default `sat/target/demo/registry`) and use
`sat/target/release/sat` or an updated SAT on PATH. SAT boolean options use values
such as `-r true`; see the tool README for the jq CLI mapping and limitations.

Tool metadata may set `stdin = "input"` to enable the explicit host `--stdin`
bridge for an optional string option with no default. This bridge reads at most
8 MiB of UTF-8, passes it through WIT, and grants no guest filesystem or stdio
authority. Tools without this metadata do not receive stdin. Setting
`verbatim_output = true` presents returned strings without adding a newline;
jaq uses this for exact empty/raw/joined output. Generic `--json` presentation
still encodes the result as JSON.

## Code retrieval

The [`code`](code/README.md) WASIp2 component statically links Tree-sitter and
Python, Rust, JavaScript, TypeScript, and TSX grammars. It finds text and returns
whole enclosing functions with deduplicated hits, exact ranges, hashes, and parse
status. It also provides outline, read-symbol, and read-enclosing operations:

```sh
bash sat/code/build.sh # Requires WASI SDK; see code/README.md
sat code search 'TemporaryDirectory' --paths /src --read ./sat::/src
sat code outline --paths /src/runtime.rs --read ./sat/src::/src
sat code read-symbol execution_error --paths /src/runtime.rs --read ./sat/src::/src --format json
```

Use the build script's `SAT_HOME` (default `sat/target/demo/registry`). Source
access is restricted by the same host-enforced `--read` grants as ripgrep.
Structural ast-grep patterns and compiler/type resolution are not included.

## Ripgrep

See [`ripgrep`](ripgrep/README.md) for a WASIp2 component built on ripgrep's
matcher, searcher, and ignore libraries. It searches granted files/directories
and returns readable excerpts through WIT (or structured results with `--format json`):

```sh
bash sat/ripgrep/build.sh
sat ripgrep 'fn\s+run' --paths /src --read ./sat/src::/src
sat ripgrep 'TODO' --paths /src --read ./sat/src::/src -C 2 --max-results 20
```

Use the same `SAT_HOME` as the build script (default `sat/target/demo/registry`).
The component supports regex/literal and case-insensitive search, recursive
ignore filtering, and context lines. `--read` grants are enforced by the host;
searching without filesystem authority fails. Run `python3 sat/ripgrep/test.py`
for component integration and confinement checks.

## Pi and Brave search

See [pi-extension](../pi-extension/README.md) for the local `web_search` extension
and Brave component setup. SAT reads optional host-owned `SAT_HOME/credentials.toml`
entries keyed by tool name. Each specifies an environment variable, header, and
exact HTTPS method/origin/path scope. Secrets are injected only after both the
embedded HTTP policy and the host credential scope authorize the request.
This configuration is never supplied by component metadata or inherited by guests.
