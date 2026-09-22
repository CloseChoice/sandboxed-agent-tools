# AGENTS.md

## Purpose

Build a portable, sandboxed tool runtime for AI agents based on WebAssembly/WASI and the WebAssembly Component Model.

The project should let an agent discover, install, and execute tools through one host binary instead of directly installing and running arbitrary host-native binaries. Tools should run with explicitly granted capabilities and should behave consistently across supported operating systems.

The WebAssembly implementation is an implementation detail. The user-facing goals are:

1. safer execution of third-party tools;
2. portable tool availability across Linux, macOS, and Windows;
3. efficient tool discovery and setup for agents;
4. lower token use by keeping intermediate data outside the LLM context;
5. composable pipelines between tools;
6. reproducible, on-demand tool environments, including Python environments.

## Core architecture

Use a native host CLI, preferably written in Rust, with Wasmtime embedded.

The host CLI is trusted. Marketplace tools are untrusted WebAssembly components.

The host is responsible for:

- discovering tools;
- downloading/installing components;
- maintaining the local tool registry;
- validating tool metadata and component interfaces;
- granting filesystem, network, and other capabilities;
- instantiating and executing components;
- composing pipelines;
- caching components and build artifacts;
- presenting a stable CLI to humans and agents.

Do not require every coding agent to have a custom native integration. The initial integration model is:

```text
coding agent
    |
    | shell/CLI
    v
host binary
    |
    +-- search/list/install/run/pipe/build
    |
    v
Wasmtime
    |
    v
WASI components
```

Coding-agent-specific skills should mainly teach the agent when and how to use the host CLI.

## Security model

Treat every downloaded tool as untrusted.

A tool must receive only the capabilities required for the current invocation.

Examples:

- fuzzy/file search: read-only access to explicitly granted directories;
- OCR: read-only access to the selected input file and, if needed, a controlled output location;
- web search: network access limited to the required provider endpoint;
- converter: access only to its input artifacts and controlled output artifacts;
- Python: no ambient access to the host filesystem or network unless explicitly granted.

WIT describes component interfaces. Actual authority is granted by the host at instantiation time.

Do not rely on Python monkey-patching, shell conventions, environment variables, or documentation as the security boundary. Enforce permissions in the host/runtime.

The agent may request permissions, but the host policy is authoritative. Never let an agent expand its own authority beyond policy.

Prefer deny-by-default.

## Tool package

A marketplace tool should contain enough metadata for the host to understand:

- identity and version;
- human-readable name and description;
- searchable tags/capabilities;
- WebAssembly component binary;
- component/WIT interface information;
- required capability classes;
- optional provider configuration such as an API-key reference;
- integrity information such as a cryptographic digest.

Do not assume that declared metadata is trustworthy. Validate the component's actual imports/exports before execution.

Keep the package format small and deterministic.

## CLI design

The exact binary name is not fixed. Keep commands stable and machine-friendly.

The CLI should support at least these concepts:

```text
<cli> list
<cli> search <query>
<cli> install <tool>
<cli> run <tool> ...
<cli> pipe ...
```

Also provide human-readable output.

Prefer structured output such as JSON behind a flag so agents do not have to parse decorative terminal output.

Only the host CLI should need to be on `PATH`. Installed WASI tools should remain namespaced behind the host rather than adding every tool binary to the global `PATH`.

## Tool discovery

Tool discovery is part of the product value.

An agent should be able to issue a short semantic/fuzzy query such as:

```text
search "OCR PDF"
search "fuzzy file search"
search "HTML to Markdown"
```

without doing a web search or investigating host-specific package managers first.

Marketplace entries should have concise descriptions and tags suitable for low-token discovery.

Track searches, failed searches, installs, and executions in a privacy-respecting way when telemetry is enabled. These signals should inform which tools are worth adding next.

## Pipelines and token efficiency

Pipelines are a first-class feature.

Intermediate results should stay outside the LLM context whenever possible.

Do not implement pipelines as a sequence of agent-visible tool calls where every intermediate result is returned to the model.

Instead, the host should orchestrate the pipeline and pass data directly between components.

Preferred interchange forms, in increasing order of size/complexity:

1. simple WIT values such as strings, bytes, records, and lists;
2. a small set of common project-defined interchange types;
3. host-managed artifact handles/files for large data.

Avoid stdout as the primary data transport. Reserve stdout/stderr mainly for logs, diagnostics, and CLI presentation.

The initial type system should stay simple. Do not create many bespoke WIT types that make independently developed tools impossible to compose.

Useful generic interchange concepts include:

- text;
- bytes;
- JSON;
- file/artifact handle;
- directory handle;
- lists/records composed from these.

Add explicit converters when useful. Conversion inside the tool runtime costs compute but does not consume LLM context tokens.

Provide a preview/sample mode where practical so an agent can test a pipeline on a small result before processing the full input.

## Artifact handling

For large outputs, use a host-managed artifact store.

A component should only receive access to artifacts explicitly passed to it.

Artifacts may be backed by temporary files initially. The abstraction should not require consumers to know their host path.

The host should clean up ephemeral artifacts safely.

Persistent artifacts must be explicit.

## Python tool

A sandboxed Python interpreter is an important showcase and target capability.

For the MVP, do not depend on runtime `pip install` inside WASI CPython.

Instead, provide a trusted builder outside the generated component:

```text
agent requests Python + packages
        |
        v
Python environment builder
        |
        | resolves/builds supported dependencies
        v
new WASI CPython component
        |
        v
registered runnable tool
```

Example target environments:

```text
Python 3.14
Python 3.14 + NumPy + pandas
Python 3.14 + NumPy + pandas + SHAP
```

The exact set of native-extension packages supported depends on packages that can actually be built for the chosen WASI target. Do not claim arbitrary PyPI compatibility.

Cache environments by a normalized specification/content hash so equivalent requests reuse existing builds.

A later project may support installing packages from inside a running WASI CPython environment. That is not required for the initial implementation.

## Initial tools

Prioritize tools that gain real value from portability, sandboxing, availability, or composability.

Good initial candidates discussed for the project are:

- fuzzy/file search;
- text/regex search;
- Tesseract-based OCR;
- document/file conversion;
- web search providers such as Exa or Brave, with provider-specific network restrictions;
- web/content fetching and extraction;
- the generated WASI CPython environments.

Git is not a priority. It already has a stable cross-platform interface and is normally installed when an agent is operating in a Git repository, so the incremental value is lower.

A file watcher and SQLite may be useful later, but they are not required for the first showcase.

## Showcase pipelines

Build at least one pipeline that demonstrates why composition matters.

A strong initial example is:

```text
PDF
 -> text/OCR extraction
 -> text search/fuzzy search
 -> only matching excerpts returned to the agent
```

This demonstrates:

- sandboxed file access;
- multiple independent tools;
- data staying outside the model context;
- lower token use;
- typed or controlled data flow.

Another useful direction is:

```text
HTTP fetch
 -> HTML/content extraction
 -> search/filter
 -> selected excerpts
```

Network access must remain provider/domain restricted by host policy.

## Cross-platform requirements

The host should target Linux, macOS, and Windows.

Do not make tool behavior depend on host commands such as `grep`, `sed`, `awk`, `fzf`, or platform package managers.

The point is for the agent to use the same host CLI and tool interface regardless of the host OS.

Mobile support is a possible future benefit, not an MVP requirement.

## Coding-agent integration

Start with skills/instructions rather than deep integrations.

A generic skill should teach an agent:

- before installing a host-native utility, search the sandboxed marketplace;
- use an already installed registered tool when suitable;
- request only the minimum filesystem/network capabilities needed;
- prefer pipelines when intermediate results would otherwise consume context;
- use preview/sample execution before expensive full runs where supported.

Pi and OpenCode are useful early targets because a CLI-based integration can remain largely agent-agnostic.

Do not make project correctness depend on prompt instructions. Skills influence agent behavior; the runtime enforces security.

## Non-goals for the MVP

Do not attempt to:

- provide arbitrary Linux binary compatibility;
- replace containers or Firecracker-style VMs;
- implement a general POSIX shell inside WASI;
- support arbitrary runtime compilation of Python C extensions;
- support every PyPI package;
- build complex language-specific repository indexing before basic search/conversion pipelines work;
- recreate Git.

## Development priorities

Implement in this order unless there is a concrete blocker:

1. Rust host with embedded Wasmtime and Component Model support.
2. Local registry and `list`/`search`.
3. Install and integrity verification.
4. Capability-restricted `run`.
5. One small filesystem tool, preferably search.
6. One conversion/OCR tool.
7. Host-managed artifacts.
8. `pipe`.
9. One network-restricted search/fetch provider.
10. WASI CPython environment builder with build-time dependencies.
11. Agent skills for Pi/OpenCode.
12. Remote marketplace/catalog.

Keep the MVP narrow enough to test adoption quickly.

## Testing

Tests must verify security boundaries, not just successful execution.

At minimum test that:

- a component cannot read paths outside granted directories;
- a read-only component cannot write;
- a component without network authority cannot access the network;
- a network-restricted component cannot access unrelated hosts;
- malformed or incompatible components are rejected;
- tool output does not accidentally leak host paths or unrelated environment variables;
- pipelines do not return intermediate payloads to the caller unless requested;
- cached Python environments are reused only when their normalized specs match.

Also run the same representative tools on Linux, macOS, and Windows in CI when practical.

## Design rule

When choosing between two features, prefer the one that strengthens at least one of these project promises:

- safe;
- portable;
- easy to discover/use;
- token-efficient;
- composable;
- reproducible.

Avoid adding tools merely because they can be compiled to WebAssembly.
