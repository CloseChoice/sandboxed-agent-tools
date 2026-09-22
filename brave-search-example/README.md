# Standalone Brave search through WASI HTTP

This example is independent of SAT: no registry, marketplace, installation flow,
or SAT configuration. It demonstrates a **WASI P2 component with an explicit WIT world** and the small
native Wasmtime host needed to enforce a provider policy and inject authentication.

```text
query → guest builds GET request → wasi:http → host checks destination
                                                  ↓
                                       host adds API-key header
                                                  ↓
guest returns typed search hits ← response ← Brave HTTPS API
```

Start with [`wit/world.wit`](wit/world.wit), the shared contract:

```wit
interface search {
    record hit {
        title: string,
        url: string,
        description: string,
    }
    search: func(query: string) -> result<list<hit>, string>;
}

world brave-search {
    import wasi:http/outgoing-handler@0.2.12;
    export search;
    export sandbox-probe: func() -> result<_, string>;
}
```

The HTTP import is the WASI Preview 2 (0.2.x) interface; WIT describes the
interface, while the host enforces its permissions. `wit/deps` vendors its
standard HTTP definitions and their transitive dependencies so builds need no WIT registry.

Read [`guest/src/lib.rs`](guest/src/lib.rs) for the implementation, then
[`host/src/main.rs`](host/src/main.rs) for the boundary. `HttpPolicy::authorize`
checks requests; `WasiHttpHooks::send_request` supplies transport. The host does
not construct the search query or parse the search results.

## Build and run without credentials

From this directory, with the repository's Rust toolchain installed:

```sh
rustup target add wasm32-wasip2
cargo build --locked -p brave-search-guest --target wasm32-wasip2
cargo run --locked -p brave-search-host -- --offline --approve-permissions tool.toml "WASI component model"
```

The `wasm32-wasip2` target produces a component directly. No component conversion
tool or native guest executable is needed. The first host build compiles Wasmtime
and can take a few minutes.

Offline mode runs the **real component**, authorizes its request through the same
policy, and substitutes [`fixture.json`](fixture.json) for the HTTPS response.
It makes no network requests and uses a dummy credential. Expected output:

```json
[
  {
    "description": "A synthetic search result returned by the host; no network request was made.",
    "title": "Offline example result",
    "url": "https://example.com/wasi"
  }
]
```

## Run a live search

Set `BRAVE_API_KEY` in the **native host's environment**, using your preferred
secret manager or a hidden terminal prompt. For example, in Bash:

```bash
read -rsp 'Brave API key: ' BRAVE_API_KEY
export BRAVE_API_KEY
cargo run --locked -p brave-search-host -- --approve-permissions --credential-env BRAVE_API_KEY tool.toml "WASI component model"
unset BRAVE_API_KEY
```

In PowerShell, set the same environment variable through your secret manager and
run the same Cargo command. There is no `.env` loader. If you add one, it should
run in the host and select only this credential.

The host calls the component’s typed `search(query)` export. The host never calls
`inherit_env`, and never preopens a directory. It inserts `X-Subscription-Token`
into the outbound request only after authorizing the destination. The key is not
passed as a command-line argument, guest environment variable, or guest file.

This uses Brave's documented
[GET web-search endpoint](https://api-dashboard.search.brave.com/app/documentation/web-search)
and [authentication header](https://api-dashboard.search.brave.com/documentation/guides/authentication).
Live execution consumes your provider quota. Offline tests do not validate your
account or contact Brave.

## Tool metadata and approval

[`tool.toml`](tool.toml) is the only tool configuration file. It contains the
component path (relative to that file), the exported interface/function to call,
and the required HTTP origin, methods, paths, authentication header, and logical
credential slot. It contains no API key or host environment variable name.

The host parses and validates it through [`host/src/metadata.rs`](host/src/metadata.rs).
There is no hardcoded Brave endpoint or search-result structure in the runner.
Its HTTP handler enforces the metadata on every request; redirects remain denied.
The guest still constructs provider requests and parses provider responses.

The declaration is not authorization: this standalone runner requires the operator
to review the file and explicitly pass `--approve-permissions`. For live execution,
`--credential-env BRAVE_API_KEY` binds the tool's single credential slot to a
host environment variable. A downloaded package cannot choose that variable.
There is no separate host-policy file or persisted approval in this example.

This flag is a manual demonstration mechanism, not an agent authorization system.
SAT must derive approval from its authoritative policy; an agent must not be able
to grant itself permissions merely by passing this flag.

The schema intentionally supports one HTTP origin and one credential slot.
The dynamic runner currently supports a single string input and a single output
composed of strings, booleans, lists, records, options, and results. Wasmtime
checks the actual function signature; other value types fail explicitly. It is
not yet a general converter for every WIT type.

## Permissions in this example

| Capability | Host policy |
| --- | --- |
| HTTP | One GET to `https://api.search.brave.com:443/res/v1/web/search`; query parameters permitted |
| Other methods, hosts, ports, paths, or plain HTTP | Denied before credential injection or transport |
| Redirects | Not followed; redirect responses rejected |
| Guest authentication headers | Forbidden; host owns authentication |
| Filesystem | No preopened directories |
| Environment and stdin | Not inherited |
| Raw TCP/UDP and guest DNS | Disabled in `WasiCtxBuilder` |
| HTTP DNS/TLS | Handled by the host transport for the authorized origin |
| Guest stdout/stderr | Discarded; search results are returned as WIT records |
| Execution | 100 million Wasm fuel units, 64 MiB per linear memory, 30 second execution deadline |

The distinction between WASI sockets and HTTP matters: disabling sockets does
**not** restrict the host's HTTP transport. The HTTP hook enforces that boundary.
See [Wasmtime's HTTP integration](https://docs.rs/wasmtime-wasi-http/48.0.2/wasmtime_wasi_http/).

The component caps its response at 2 MiB and returns only titles, URLs, and
descriptions. That response cap is guest behavior, not a host-enforced bound for
arbitrary replacement components. This is a focused example, not a production
runtime: compilation has no deadline, memory limits are per linear memory, and
there is no general private-IP egress policy or multi-user credential store.
The host transport verifies TLS for the approved provider hostname.

The guest is a `cdylib`, not a command: there is no guest `main`, command-line
argument parsing, or `wasi:cli/run` export. The host resolves the export named in `tool.toml` dynamically, passes a WIT
string, and receives WIT values. It has no compile-time dependency on the
Brave world or its result record. Only the native host prints JSON.
The demonstration-only `sandbox-probe` export runs the security checks separately
from search; it is not a special search query.

## Verify the boundary

Build the guest first, then run:

```sh
cargo test --locked -p brave-search-host
```

Tests check metadata validation and explicit approval, configurable enforcement,
rejected origins/methods/paths, credential insertion, and the one-request budget. They also instantiate the built component and exercise offline search and
its typed `sandbox-probe` export, checking guest environment isolation, denied filesystem access,
failed socket access, and explicit policy rejection of an unrelated HTTP origin.
No API key is required.

To inspect the actual imports/exports, if `wasm-tools` is installed:

```sh
wasm-tools component wit target/wasm32-wasip2/debug/brave_search_guest.wasm
```

Rust's standard library imports additional WASI interfaces, including filesystem
and sockets; imports alone do not grant authority. This host links those interfaces
but supplies no directory handles or socket permissions.
