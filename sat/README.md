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
s32/s64, f32/f64, or `option<T>` of those types. Booleans use `--option true`/`false`.
Results are printed as text or JSON (`--json` forces JSON). WIT errors become
nonzero exits.

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

Build the pinned componentize-py toolchain once:

```sh
python3 python-example/build.py --no-install
```

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
fresh temporary tree, then runs the pinned `componentize-py` builder. The builder
pre-initializes the Python guest under Wasmtime with the temporary app and package
paths mapped in. Package imports and `.pth` startup hooks therefore run in that
WASI guest; no registry credentials or network are passed to it. SAT validates
the produced component with its normal package loader, registers it, and removes
the temporary tree on success or failure. `SAT_BUILD_TMP` selects the temporary
directory; `SAT_PYTHON_BUILD_TIMEOUT_SECS` sets the per-process limit (default
600 seconds); `SAT_COMPONENTIZE_PY` can point to a separately installed pinned
componentize-py 0.25.0 builder.
The generated tool also embeds Python's deny-HTTP policy, so `--allow-http` does
not grant network access to installed packages.

This first implementation intentionally rejects direct package URLs and pip
options, and does not build source distributions or native extensions. It does
not run PEP 517 build backends. It mitigates startup payloads such as malicious
`.pth` files by executing them in the componentizer's WASI initialization guest,
but it does not make the package safe to run: the generated component remains
untrusted and needs narrow runtime grants. Wheel resolution/download still occurs
on the host, so the configured registry is trusted to serve the selected wheel
files. The WASI pre-initialization is resource-limited by a process timeout, but
SAT does not yet enforce a build memory or disk quota. It caches completed builds
by tool name, sorted requirement strings, registry identity and downloaded wheel
hashes; it still resolves/downloads wheels before checking that cache.

## Pi and Brave search

See [pi-extension](../pi-extension/README.md) for the local `web_search` extension
and Brave component setup. SAT reads optional host-owned `SAT_HOME/credentials.toml`
entries keyed by tool name. Each specifies an environment variable, header, and
exact HTTPS method/origin/path scope. Secrets are injected only after both the
embedded HTTP policy and the host credential scope authorize the request.
This configuration is never supplied by component metadata or inherited by guests.
