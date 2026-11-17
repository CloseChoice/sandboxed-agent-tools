# Sandboxed Agent Tools (SAT)

SAT provides sandboxing your tools. It provides capabilities to allow read and write to files and http calls on a per-tool-call basis.
SAT uses the Webassembly System Interface (WASI) for sandboxing. This means we rely on proven architecture and just provide the glue
to make sandboxing for tools work.
Currently there are 4 tools available

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

## What you can link into the sandbox
Currently we allow 

## Permission limitations

**Do not grant a tool write access to SAT configuration files or any directory
that contains them.** This applies both to CLI `--write` grants and to the
`write` lists in configuration files.

Keep all of the following outside a tool's write grants:

- `$SAT_HOME/config.toml` (normally `~/.sat/config.toml`).
- The project's `.sat/config.overrides.toml`.
- The directories containing those files, including the project's `.sat/`
  directory and `$SAT_HOME`.
- **Every ancestor directory of those files.** A broad write grant to the
  project root, your home directory, or the filesystem root can include a
  protected configuration path even when you never name that file explicitly.

Use dedicated output directories that neither contain SAT configuration files
nor are ancestors of those files. Keep their ancestor directories outside the
tool's write grants as well when moving those ancestors could relocate SAT
configuration or `$SAT_HOME`.

These restrictions address known weaknesses:

- A permitted tool can rewrite project settings when those settings are writable.
- Renaming a writable parent can bypass exclusions on its children.
- Renaming an ancestor can bypass `$SAT_HOME` protection.

**A child `deny-write` rule does not make a broad parent write grant safe.**
Denying writes to the configuration filenames alone is not a complete
mitigation for the rename weaknesses. Do not rely on such rules while granting
write access to the relevant ancestor directories.

These are explicit usage constraints while the weaknesses remain. Host-enforced
configuration protection must cover direct changes and renames affecting
protected directories or their ancestors; fixed filenames or a hard-coded
`SAT_HOME` path alone do not provide that guarantee.

## Usage

There are two ways how to control what permissions your tools get:
- the cli
- the sat config file

### SAT's CLI Interface

#### File Permissions
Run a tool via 
```bash
sat <toolname> toolarg1 toolarg2 --toolkwarg1 -- --read <file_or_dir1> --read <file_or_dir2> --read <file_or_dir3> --write <file_or_dir4>
```
E.g. for the python tool one can do
```bash
touch dummy1.txt
sat python -c "f = open('$PWD/dummy1.txt', 'w+');f.write('hi there from the sandbox') " -- --write $PWD/dummy1.txt
```
While this would fail
```bash
touch dummy1.txt
sat python -c "f = open('$PWD/dummy1.txt', 'w+');f.write('hi there from the sandbox') " -- --read $PWD/dummy1.txt
```
with an `PermissionError`.

Another example using http calls:
```bash
sat python -c 'import urllib.request; print(urllib.request.urlopen("https://example.com", timeout=10).read(200).decode())' -- --allow-http example.com
```

This fails if the called and target url don't align:
```bash
sat python -c 'import urllib.request; print(urllib.request.urlopen("https://example.com", timeout=10).read(200).decode())' -- --allow-http google.com
```

Redirects must be explicitly allowed
```bash
sat python -c 'from urllib import request; r = request.urlopen("https://httpbin.org/redirect-to?url=https%3A%2F%2Fexample.com", timeout=10); print(r.status, r.geturl())' -- --allow-http httpbin.org --allow-http example.com
```
but remove `--allow-http example.com` and this fails.

### SAT Config
#### File Permissions

By default sat stores its data at `~/.sat/`. This path is changable and needs to be exposed through `SAT_HOME`.
In there lives a toml file with the configuration for all tools: `config.toml`. This file has the following structure

```toml
[tools.python.file_permissions]
read = ["$PWD/inputs"]
write = ["$PWD/output"]
deny-read = ["**/*.key"]
deny-write = []

[tools.python.http_permissions]
allow-http = ["example.com", "httpbin.org"]

# we omit more examples here, the general pattern is
# [tools.<toolname>.file_permissions]
# read = ["$PWD/some_path", "$SAT_HOME/some_other_path", "/some_absolute_path"]
# write = []
# deny-read = []
# deny-write = []
# 
# [tools.<toolname>.http_permissions]
# allow-http = []
```

`read` grants read-only access; `write` grants read and write access.
Before adding a write grant, follow the [configuration and ancestor-directory
restrictions](#permission-limitations) above.
`deny-read` and `deny-write` contain exclusion globs. Empty lists grant no
access. Filesystem grant paths must exist and be absolute or start with the
literal `$PWD/` or `$SAT_HOME/`; SAT resolves these placeholders when a tool
runs.

`allow-http` lists permitted hostnames, without a scheme, port, or path.
An empty allowlist denies all HTTP requests. Redirect destinations need their
own matching hostname grant. Set either `allow-http` or `deny-http`, never both.

Project settings live in `.sat/config.overrides.toml` and replace the global
settings for each tool listed there. `sat init` creates a snapshot of the
effective settings for all registered tools.
`sat init --force` refreshes an
existing file while preserving its project overrides. Use `sat config` or
`sat config python` to inspect the effective settings. CLI permission flags
after `--` override the corresponding lists for that invocation.


## Getting Started

The installers download published GitHub Release assets and verify their SHA-256
checksums. These URLs become available after the first release is published.

### Linux and macOS

Install SAT and the four default tools (`python`, `jaq`, `code`, `markitdown`):

```sh
curl -fsSL https://github.com/CloseChoice/sandboxed-agent-tools/releases/latest/download/install.sh | bash -s -- default-tools
```

Or select tools and an exact release version:

```sh
curl -fsSL https://github.com/CloseChoice/sandboxed-agent-tools/releases/download/v0.1.0/install.sh | bash -s -- python,jaq --version 0.1.0
```

Omit the tool argument to install only SAT. The script detects Linux x64,
Intel macOS, or Apple Silicon macOS. Override detection with
`--platform linux-x64`, `macos-x64`, or `macos-arm64`.

SAT is installed to `~/.local/bin`; change this with `--bin-dir DIRECTORY`.
If needed, add that directory to your shell's PATH:

```sh
export PATH="$HOME/.local/bin:$PATH"
```

### Windows PowerShell

Install the Windows x64 host and all four tools:

```powershell
& ([scriptblock]::Create((Invoke-RestMethod "https://github.com/CloseChoice/sandboxed-agent-tools/releases/latest/download/install.ps1"))) -Tools "default-tools"
```

For an exact version and selected tools:

```powershell
& ([scriptblock]::Create((Invoke-RestMethod "https://github.com/CloseChoice/sandboxed-agent-tools/releases/download/v0.1.0/install.ps1"))) -Tools "python,jaq" -Version "0.1.0"
```

Omit `-Tools` to install only SAT. The default binary directory is
`$env:LOCALAPPDATA\SAT\bin`; override it with `-BinDir DIRECTORY`.
The script prints the command to add this directory to the current session's
PATH. Add it to your user PATH for future sessions.

Both scripts accept versions with or without the `v` prefix, including
prereleases such as `0.1.0-rc.1`. For a prerelease, use its explicit version
and versioned script URL. Omitting the version selects GitHub's latest release.
Download URLs are derived from the tag and asset name, so publishing another
version requires no URL edits in either script.

New tools start with no filesystem or HTTP permissions. Existing project and
global permission settings are kept. After installation, `sat init` writes a
project configuration you can edit.

You can also register a tool archive manually:

```sh
sat register <path-to-tar>
```

## Build your own Tools

### TODO: what do we need to do to build our own tools?

## Future Considerations

This is a very early version of what SAT can become. There are many more interesting paths to explore, of which I just want to point to 
WASI's type system (todo: provide link) and the possiblities this means for composing various tools without relying on serialization or the command line to hand over intermediate objects.

(todo: maybe we could sketch something here).

## Local Development: Build and use

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
for this branch's scope. Release assets include native SAT archives for the four
supported platforms, the four tool archives, `install.sh`, `install.ps1`, and
`SHA256SUMS`.

Download an archive from Releases and use `sat register` with its path or URL.
With the release archives and installer scripts downloaded, verify them using
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
