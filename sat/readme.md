# SAT: Safe Agent Toolbox

SAT is the native host for discovering and executing sandboxed tools. The current
implementation is a CLI and runtime scaffold; it does not execute tools yet.

## Current status

| Area | Implemented | Still needed |
| --- | --- | --- |
| CLI | Clap parses `--tool NAME` and preserves ordered `Vec<OsString>` arguments | Execution dispatch, host permission options, stable subcommands and JSON output |
| Tool lookup | `verify_path` checks a directory under `WAT_HOME`, rejects invalid names and canonical paths outside that root | Registry records, manifests, component-file resolution and integrity checks |
| WASI configuration | `WasiConfig` describes stdio inheritance, explicit environment variables, directory access, network destinations and P1/P2 selection | Policy validation and conversion into a WASI context |
| Execution | `execute(&str, &WasiConfig, &[OsString])` exists | Engine, component loading, store, linker, instantiation and export invocation |
| Integration | `runtime` is exposed by the library | `main` currently prints parsed arguments; `execute` returns an unsupported error |

P2 is the configuration default. P1 is represented in the enum but has no
implementation. Default settings request no inherited stdio, environment,
directories or network access. These are data definitions, not enforced security
boundaries. Clock and randomness settings are outside the current configuration.

The sibling `fuzzy-search` package already contains a component host example and
tests for typed exports. It is not integrated into SAT's runtime.

## Next milestone: run a directory-listing component

Use a small, locally built P2 command component that lists a supplied guest
directory using Rust filesystem APIs. This is an `ls`-style execution fixture,
with no dependency on the host's `ls` binary. It completes the embedded-runtime
foundation before marketplace work.

The execution sequence is:

1. Make `wasmtime` a regular host dependency alongside a compatible
   `wasmtime-wasi` version; it is currently only a dev-dependency.
2. Create an `Engine`, load a `wasmtime::component::Component`, and check that
   its imports and exports match the supported P2 command interface.
3. Validate the requested configuration against host policy. Initially support
   read-only directory grants and deny networking; reject unsupported settings.
4. Build a `WasiCtx` with explicit arguments, environment and directory preopens.
   Reject non-UTF-8 arguments instead of silently changing them. Supply the tool
   name as argument zero. Explicitly enable output for this fixture, since the
   default discards it.
5. Put the context and a `ResourceTable` into host state implementing the WASI
   view required by the selected library version, then create a `Store` using
   the same engine.
6. Create a component `Linker`, register the WASI host implementations, and
   instantiate the component with the linker and store.
7. Call the command's `wasi:cli/run` export. Distinguish guest failure from host
   errors and traps, then connect this path to the CLI.

The engine compiles code; the store holds execution state. `WasiCtxBuilder::build`
produces context data, not a runnable instance. Invocation happens through the
instantiated component's exports. See the official
[WASI command example](https://docs.wasmtime.dev/api/wasmtime_wasi/p2/bindings/sync/struct.Command.html)
and [context builder](https://docs.wasmtime.dev/api/wasmtime_wasi/struct.WasiCtxBuilder.html).
Use documentation matching the dependency version when implementing this flow.

Acceptance checks should prove that the fixture lists a granted directory and
cannot access an ungranted directory or escape through parent paths or symlinks.
Companion probes should verify that writes and network access are denied, host
environment variables are absent, and malformed or incompatible components fail.
Test representative tools on Linux, macOS and Windows in CI when practical.

P1 needs a separate core-module execution path or an explicit component adapter;
changing the enum alone does not make a P1 module a P2 component. Defer P1 support
until the P2 path works, and reject P1 requests meanwhile.

## Roadmap after the execution fixture

1. Add a local registry and `list`/`search`, then installation and digest checks.
2. Complete policy-enforced `run`, including execution resource limits and clear
   outcome reporting, before accepting downloaded tools.
3. Integrate a useful filesystem search tool, followed by conversion/OCR.
4. Add host-managed artifacts and pipelines that keep intermediate data outside
   the model context. Use typed results or artifact handles for composition;
   the listing fixture's console output is only an initial demonstration.
5. Implement a restricted network provider with endpoint enforcement.
6. Add the cached WASI Python environment builder, agent skills and a remote
   catalog, following the project priorities in `../AGENTS.md`.

## Development notes

Use descriptive parameter names without underscore prefixes. Incomplete code may
produce unused-parameter warnings until its implementation consumes the inputs.

The runtime module was previously checked with standalone `rustc`. A full Cargo
check was blocked by an uncached `wasmtime-wasi` dependency, so successful
end-to-end compilation and execution have not yet been established.
