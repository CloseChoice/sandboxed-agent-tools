# Resources for understanding sandboxed-agent-tools

SAT is a trusted native Rust host that runs untrusted WebAssembly components.
Its central idea is to make tools portable and easy for agents to use while
granting each invocation only the capabilities it needs. Read the following
resources in order; the tool-specific references can wait until needed.

## Essential reading

1. **[Project overview](README.md) and [project principles](AGENTS.md).**
   Start with the build/install/run workflow, then read the security model,
   capability rules, pipeline goals, Python environment design, and non-goals.
   Distinguish the product roadmap from features implemented in this checkout.

2. **[WebAssembly Component Model guide](https://component-model.bytecodealliance.org/).**
   Read the sections on components, interfaces, worlds, and composition. These
   explain why SAT uses components rather than arbitrary native executables,
   and how independently built tools can expose typed interfaces.
   Follow with the [WIT reference](https://component-model.bytecodealliance.org/design/wit.html):
   focus on imports/exports, strings, lists, options, results, and resources.
   WIT describes an interface contract; SAT's host policy grants authority.

3. **[WASI introduction](https://wasi.dev/) and
   [Wasmtime security model](https://docs.wasmtime.dev/security.html).**
   Understand the distinction between isolated Wasm execution and the host
   functions that expose filesystem, network, and other capabilities. The
   embedding host's configuration is part of the security boundary.

4. **[Wasmtime's WASIp2 embedding example](https://docs.wasmtime.dev/examples-wasip2.html).**
   Learn how `Engine`, `Component`, `Linker`, `Store`, `ResourceTable`, and
   `WasiCtx` fit together. Then read
   [interrupting execution](https://docs.wasmtime.dev/examples-interrupting-wasm.html)
   for optional runtime interruption mechanisms. SAT currently disables fuel
+   metering. Compare examples with the versions pinned in
   [sat/Cargo.toml](sat/Cargo.toml) and [Cargo.lock](sat/Cargo.lock); upstream
   documentation may describe newer APIs. SAT also calls typed root exports,
   so the standard WASI command example is a foundation rather than its exact API.

5. **[The architecture notes](sat/NEW_ARCHITECTURE.md), alongside the source map below.**
   Read the CLI, registry layout, configuration precedence, and permission rules.
   These notes include proposals and placeholders; verify behavior in code and
   tests. [sat/readme.md](sat/readme.md) describes an older scaffold and is not
   a reliable guide to current implementation status.

## Read one invocation through the code

Follow `sat <tool> ... -- --read /absolute/path` from command parsing to execution:

| Read | What to understand |
| --- | --- |
| [main.rs](sat/src/main.rs), [cli.rs](sat/src/cli.rs) | Built-in commands, tool dispatch, and the separation between tool arguments and host permission flags. |
| [config.rs](sat/src/config.rs), [metadata.rs](sat/src/metadata.rs) | Global/project/invocation settings, defaults, and the metadata used to describe a tool's CLI. |
| [registry.rs](sat/src/registry.rs) | Archive extraction, registration, component loading, and preparation of the compilation cache. |
| [runtime.rs](sat/src/runtime.rs) | Actual component interface validation, argument conversion, Wasmtime setup, host linking, invocation, and result handling (fuel metering is disabled). |
| [access.rs](sat/src/access.rs), [grants.rs](sat/src/grants.rs), [files.rs](sat/src/files.rs) | Path authorization, standard WASI filesystem grants, and project-defined filesystem imports. Read all three before changing file access. |
| [http.rs](sat/src/http.rs), [browser.rs](sat/src/browser.rs) | HTTP destination checks and the separate CDP transport/authority boundary. |

For a small complete tool, read the file-search
[WIT interface](external_tools/file-search/wit/world.wit),
[implementation](external_tools/file-search/src/lib.rs),
[metadata](external_tools/file-search/tool.toml),
[permission defaults](external_tools/file-search/defaults.toml), and
[package script](external_tools/file-search/package.sh).
Then read the shared [archive packager](external_tools/package.py) and
[archive checker](external_tools/check_archives.py). This connects the guest
interface to the installed three-file package and the host-generated CLI.

## Tests that explain the guarantees

- **[Host tests](sat/src/runtime.rs), plus tests in the permission and transport
  modules above:** component validation and runtime boundary checks. Search for
  `#[cfg(test)]` and `#[test]`/`#[tokio::test]` within the source files.
- **[Imported-tool integration tests](external_tools/test_imported_tools.py):**
  successful execution together with denied access, write restrictions, symlink
  isolation, and registry protection.
- **[Python tests](external_tools/python/test.py) and
  [builder cache tests](external_tools/python/test_build.py):** actual WASI
  execution, supported standard-library operations, capability restrictions,
  and cache invalidation when inputs or artifacts change.
- **[Browser integration tests](external_tools/browser/test.py):** real Chrome
  interaction and rejection of invalid or ungranted debugging endpoints.
- **[Release workflow](.github/workflows/tool-releases.yml):** how packaged tools
  are built, registered, and checked in CI. Local test results and configured
  CI jobs are distinct from evidence that every CI platform has passed.

## Tool-specific follow-up

| Area | Read | Why it matters |
| --- | --- | --- |
| OCR | [Full original upstream tests](external_tools/tesseract/UPSTREAM_TESTS.md), [Tesseract guide](external_tools/tesseract/README.md), [locked dependencies](external_tools/tesseract/sources.json), [native/WASI and security checks](external_tools/tesseract/test.py), and [results](external_tools/tesseract/TEST_RESULTS.json) | Tesseract + Leptonica, embedded English model, image inputs and explicit grants. |
| PDF Python dependencies | [Full native dependency graph and WASI scope](external_tools/markitdown/DEPENDENCIES.md), [machine-readable graph](external_tools/markitdown/DEPENDENCIES.json) | Transitive dependency audit; distinguishes bundled Python sources from excluded crypto/rendering/ML branches. |
| Patch application | [Unified-diff CLI](external_tools/patch/README.md), [implementation](external_tools/patch/src/lib.rs), [differential/security tests](external_tools/patch/test.py), and [original GNU regression](external_tools/patch/test_upstream.py) | Strict unified-diff profile with dry-run, explicit grants and tests against GNU patch. |
| MarkItDown compatibility | [WASI PDF CLI](external_tools/markitdown/README.md), [upstream test driver](external_tools/markitdown/test_upstream.py), and [results](external_tools/markitdown/UPSTREAM_RESULTS.json) | Original MarkItDown parser/converter, a pinned PDF profile, and unchanged upstream PDF/CLI assertions. |
| PDF text | [Local extraction guide](external_tools/pdf-text/README.md), [implementation](external_tools/pdf-text/src/lib.rs), and [security tests](external_tools/pdf-text/test.py) | Embedded text with page numbers, bounded previews, and decompression limits; scanned pages still need OCR. |
| Python | [Local Python guide](external_tools/python/README.md), [builder](external_tools/python/build.py), [wrapper](external_tools/python/app.py), and upstream [componentize-py](https://github.com/bytecodealliance/componentize-py) | Explains build-time imports, the trusted host-side builder, cached components, and the limits of package/stdlib support. Host subprocesses and arbitrary runtime pip installations are not implied by having Python. |
| Code retrieval | [Local code guide](external_tools/code/README.md) and [Tree-sitter introduction](https://tree-sitter.github.io/tree-sitter/) | Explains syntax-aware extraction of enclosing definitions, outlines, and symbols. |
| Browser/CDP | [Local browser guide](external_tools/browser/README.md) and [Chrome DevTools Protocol](https://chromedevtools.github.io/devtools-protocol/) | Explains the native browser transport. Read the local authority section carefully: granting CDP controls that browser instance; WASI filesystem/HTTP grants do not confine Chrome's own access. |
| Fuzzy matching | [Local fuzzy-search guide](external_tools/fuzzy-search/README.md) and [vendored matcher notes](external_tools/fuzzy-search/vendor/neo_frizbee/PATCHES.md) | Explains FFF matching and the changes carried for the WASI build. |

If time is limited, read the project overview and principles, the Component
Model/WIT introductions, `runtime.rs`, the filesystem permission modules, and
the integration tests first. Those cover the project's purpose and its most
important trust boundaries.
