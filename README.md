# Sandboxed Agent Tools (SAT)

**This is an early version of what SAT should become one day. Feel free to explore and give feedback. Please don't treat 
this project as production ready and feature complete.**

SAT provides sandboxes for tool your agent uses. It provides capabilities to allow read and write to files and http calls on a per-tool-call basis.
SAT uses [Wasmtime](https://wasmtime.dev/) to run tools compiled to the Webassembly System Interface (WASI) for sandboxing. This means we rely on proven architecture and just provide the glue
to make sandboxing per tool-call work.

![In the shared sandbox, the agent calls Python and a CLI tool. Both use the same mounted files, and the CLI tool also accesses the web. With SAT, the agent calls each tool in a separate sandbox. Python can read inputs/. The CLI tool can read docs/ and make HTTP requests to api.example.com.](docs/images/sandbox-comparison.png)

Currently there are 5 tools available

SAT is a native host that runs WebAssembly tools with explicit filesystem and
network permissions. The initial release includes five portable tools:

| Tool | Purpose |
| --- | --- |
| [python](external_tools/python/) | CPython 3.14 with a selected standard-library profile |
| [jaq](external_tools/jaq/) | The upstream jq-compatible CLI for structured data |
| [code](external_tools/code/) | Tree-sitter code search and definition retrieval |
| [markitdown](external_tools/markitdown/) | The upstream MarkItDown CLI's local PDF profile |
| [ripgrep](external_tools/ripgrep/) | Text search using upstream ripgrep libraries |

The host lives in [sat/](sat/). The five release tools are in
[external_tools/](external_tools/), with the selection in
[release-tools.toml](external_tools/release-tools.toml). Experimental tools
are preserved on the `backup-all-tools` branch.

## Upstream credits

SAT builds on work by the authors and maintainers of
[CPython](https://github.com/python/cpython),
[componentize-py](https://github.com/bytecodealliance/componentize-py),
[jaq](https://github.com/01mf02/jaq),
[Tree-sitter](https://github.com/tree-sitter/tree-sitter),
[MarkItDown](https://github.com/microsoft/markitdown), and
[ripgrep](https://github.com/BurntSushi/ripgrep).
Each tool's README identifies its upstream projects and licenses.
Release tool archives include `THIRD_PARTY_NOTICES.txt` with the retained
upstream license texts; SAT keeps this file when registering a tool.

## What you can link into the sandbox
Currently we allow the following permissions for the sandboxes:
- read: grant read access to an existing file or directory. For example, `--read "$PWD/src/*.py"` grants read access to matching regular files directly inside `src`, subject to the configured exclusions.
  Quote the pattern so SAT expands the filename wildcards (`*` and `?`).
  Use an absolute path or start it with `$PWD/` for a path relative to the current working directory.
  In Bash, `--read "$HOME/notes.txt"` also works because Bash expands `$HOME` before invoking SAT. In TOML, use the absolute path for that file.
- write: grant read and write access to an existing file or directory, e.g. `--write "$PWD/output"`.
- deny-read: exclude files or directories from reads in the tool's configuration, e.g. `deny-read = ["**/.env"]`.
- deny-write: exclude files or directories from writes in the tool's configuration, e.g. `deny-write = ["**/*.lock"]`.
- allow-http: allow HTTP and HTTPS requests to specific hosts, e.g. `--allow-http api.example.com`. Use `--allow-http '*.example.com'` to allow subdomains of `example.com`.
- deny-http: allow HTTP and HTTPS requests to all hosts except those listed, e.g. `--deny-http api.example.com`. Choose either `allow-http` or `deny-http` for a tool call.


## Getting Started

The Linux x86_64 installer downloads GitHub Release assets and verifies their
SHA-256 checksums.

To install SAT and all five default tools from the published prerelease:

```sh
curl -fsSL https://github.com/CloseChoice/sandboxed-agent-tools/releases/download/v0.1.0/install.sh | bash -s -- --version v0.1.0 default-tools
```

Replace `default-tools` with a comma-separated list such as `python,jaq`, or
omit it to install only SAT. Use the same tag in the URL and in `--version`.
GitHub's `latest` URL is available once a stable release is published.

SAT is installed to `~/.local/bin`; change this with `--bin-dir DIRECTORY`.
If needed, add that directory to your shell's PATH:

```sh
export PATH="$HOME/.local/bin:$PATH"
```

Add the [SAT skill](skills/sandboxed-agent-tools/SKILL.md) to your agent so it can choose among the five default tools and grant permissions per call.
From a checkout of this repository, copy the skill into your harness’s skills directory. For example, Pi discovers skills in `~/.agents/skills/`:

```sh
mkdir -p "$HOME/.agents/skills"
cp -R skills/sandboxed-agent-tools "$HOME/.agents/skills/"
```

New tools start with no filesystem or HTTP permissions. Existing project and
global permission settings are kept. After installation, `sat init` writes a
project configuration you can edit.

You can also register a tool archive manually:

```sh
sat register <path-to-tar>
```

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

SAT expands quoted read globs once per invocation. It skips symlinks and directories and grants the matched files individually. The call fails if a pattern matches no permitted files, or if expansion across all patterns exceeds 128 matches or 65,536 directory entries scanned. Use exact paths in configuration and for write grants.

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
# read = ["$PWD/some_path", "/some_absolute_path"]
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
access. Filesystem grant paths must exist. Use an absolute path or prefix a
project-relative path with `$PWD/`, which SAT resolves when the tool runs.

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


## Build your own Tools

You can build your own tools in any language that compiles to the Webassembly System Interface (WASI) Preview 2.
See a list of [supported languages](https://wasi.dev/languages).

Build your tool as a WASI Preview 2 component named `tool.wasm`. Package it with
`tool.toml` and `defaults.toml`. Keep the interface definition, `world.wit`,
alongside your source. See [external_tools/jaq/](external_tools/jaq/) for a
complete Rust example.

### tool.wasm

The component must export the entrypoint configured in `tool.toml`. SAT can
generate a CLI from the metadata; tools with an existing CLI can use argument
passthrough instead.

### world.wit

This defines the functions the component imports and exports. The compiler
embeds that interface in `tool.wasm`; the WIT file stays with your source.

### tool.toml

This contains the tool's identity, entrypoint, and CLI metadata. For jaq:

```toml
name = "jaq"
version = "3.1.2"
description = "Upstream jaq CLI: jq-compatible filters, positional files, stdin, variables, modules, and JSON/YAML/TOML/XML/CBOR formats"
entrypoint = "run"  # Exported function called by SAT
command_output = true  # Forward sandbox stdout/stderr
cli_passthrough = true  # Pass tool arguments unchanged to run(args: list<string>); the tool parses its CLI

[[arguments]]
name = "args"
help = "Arguments passed unchanged to upstream jaq"
```

`command_output` treats the returned integer as an exit status.
`cli_passthrough` forwards tool arguments unchanged.

### defaults.toml

This suggests permissions during registration. Start with no filesystem or
HTTP access:

```toml
[file_permissions]
read = []
write = []
deny-read = []
deny-write = []

[http_permissions]
allow-http = []
```

### Packaging

Package to tar with:

```sh
tar -cf jaq.tar tool.wasm tool.toml defaults.toml
```

Register it and check its CLI:

```sh
sat register jaq.tar
sat jaq --help
```

## Future Considerations

This is a very early version of what SAT can become. One interesting direction is composing tools through typed interfaces (see the [WIT Reference](https://component-model.bytecodealliance.org/design/wit.html)), passing structured data directly between components.

For example, one component could decode an image into dimensions, metadata, and pixels:

`loader.wit`:

```wit
package example:vision;

interface images {
    record image {
        width: u32,
        height: u32,
        metadata: list<tuple<string, string>>,
        pixels: list<u8>, // Row-major RGBA8: four bytes per pixel.
    }
}

interface loading {
    use images.{image};
    load: func(encoded: list<u8>) -> result<image, string>;
}

world image-loader {
    export images;
    export loading;
}
```

A second component could receive that image as a typed argument and locate areas of high contrast:

`contrast.wit` (using the same package):

```wit
package example:vision;

interface contrast {
    use images.{image};

    record region {
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    }

    locate: func(input: image, threshold: f32) -> list<region>;
}

world contrast-detector {
    import images;
    export contrast;
}
```

The intended flow is `loader.load(encoded) → image → contrast.locate(image, threshold)`, with a successful load passing the complete image record to the detector. It receives dimensions, metadata, and pixel bytes together and returns bounding boxes for the detected regions. No intermediate CLI arguments, JSON, or image file are needed.

Unlike a resource handle, this record passes data by value; the pixel buffer may be copied between components. This is a future composition sketch, not a currently supported SAT pipeline. SAT would need to wire the interfaces together and pass the typed values between tools.

Wasmtime supports per-call execution limits through [fuel metering](https://docs.wasmtime.dev/examples-interrupting-wasm.html#deterministic-fuel) and [epoch-based interruption](https://docs.wasmtime.dev/examples-interrupting-wasm.html#non-deterministic-epochs). Early experimentation showed that more work needs to be put into this area before adding it to SAT.

## Performance Considerations

Running tools in a sandbox can add overhead. In our early trials, the agent
workflow that included the `code` tool took longer than its native-tools
baseline. In trials without `code`, the workflow ran in about the same time
as its baseline. We need more measurements to understand how much the sandbox,
the tools the agent chooses, and the workload each contribute to runtime.

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

Packages require `tool.wasm`, `tool.toml`, and `defaults.toml`; release tools
also include `THIRD_PARTY_NOTICES.txt` for upstream licenses and notices.
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
entries depend on component content and engine configuration. SAT blocks
direct access to its registry and cache. Keep their parent directories and
all other ancestors outside tool write grants, as described in
[Permission limitations](#permission-limitations).

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
exactly `python.tar`, `jaq.tar`, `code.tar`, `markitdown.tar`, and `ripgrep.tar`; it rejects
missing or extra assets.

Branch pushes, pull requests, and manual runs produce Actions artifacts.
Only a pushed `v*` tag publishes a GitHub release. `sat/Cargo.toml` defines
the bundle version, currently `0.1.0`. Rust tool metadata and lockfiles must
match each tool's own Cargo version; Python-based tools keep their versions
in `tool.toml`.

With SAT at `0.1.0`, tag `v0.1.0-dev.1` to publish a prerelease. Its native
archives include `0.1.0-dev.1` in their names, and `sat --version` reports
`0.1.0`. Install it by specifying `0.1.0-dev.1` explicitly.

After reviewing the committed sources and successful CI checks:

```sh
python3 scripts/release.py --check-tag v0.1.0
git tag -a v0.1.0 -m "SAT 0.1.0"
git push origin v0.1.0
```

These commands are explicit publishing steps. See [release.md](release.md)
for this branch's scope. Release assets include the Linux x86_64 SAT archive,
the five tool archives, `install.sh`, and `SHA256SUMS`.

Download an archive from Releases and use `sat register` with its path or URL.
With the release archives and installer scripts downloaded, verify them using
`sha256sum --check SHA256SUMS`. To check local archive contents:

```sh
python3 external_tools/check_archives.py --complete \
  external_tools/python/dist/python.tar external_tools/jaq/dist/jaq.tar \
  external_tools/code/dist/code.tar external_tools/markitdown/dist/markitdown.tar \
  external_tools/ripgrep/dist/ripgrep.tar
```

Build outputs, local toolchains, credentials, personal sample data, logs,
and research checkouts are ignored by Git. Dependency lockfiles are tracked.

## Execution limits

SAT runs tools without Wasmtime fuel metering; legacy package `fuel` values
are ignored. Filesystem/network permissions remain enforced independently.
Callers can cancel a non-finishing invocation through the calling process.
