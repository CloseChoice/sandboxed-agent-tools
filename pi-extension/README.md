# Pi web search through SAT

A local Pi extension registering `web_search(query)`. It invokes the SAT CLI,
which executes the Brave WASI component and returns up to five search results.
No MCP server or persistent SAT service is needed. Pi's other tools are unchanged.

Tested with Pi 0.87.0 (`@earendil-works/pi-coding-agent`). Pi itself is trusted
native code; the WASI component is sandboxed by SAT.

## Setup

From the repository root, with Rust, Python 3.11+, and Node 22+:

```sh
python3 pi-extension/setup.py
npm ci --ignore-scripts --prefix pi-extension
export SAT_BIN="$PWD/sat/target/debug/sat"
export SAT_HOME="$PWD/sat/target/demo/registry"
```

Setup builds SAT and the Brave component, registers `brave-search` (replacing a
previous local installation), and adds its entry to `SAT_HOME/credentials.toml`
if absent. Existing credential entries are preserved. Set `SAT_HOME` before
setup to choose a different registry. Setup makes no searches and needs no key.
On Windows, use `sat.exe` and your shell's environment-variable syntax.

Supply `BRAVE_API_KEY` to the native host environment using your secret manager.
For example, Bash can read it without echoing it or putting it in shell history:

```sh
read -rsp 'Brave API key: ' BRAVE_API_KEY
export BRAVE_API_KEY
```

Then launch your installed Pi:

```sh
pi --extension ./pi-extension/sat.ts
```

If Pi is not on PATH, the development dependencies include a local installation:

```sh
./pi-extension/node_modules/.bin/pi --extension ./pi-extension/sat.ts
```

Use your normal Pi model/provider configuration. Ask it to search the web. You
can also check SAT directly (this sends a real request and uses Brave quota):

```sh
"$SAT_BIN" brave-search --query 'WASI components' --allow-http --json
```

## Permissions and credentials

The component's embedded policy permits only GET requests to
`https://api.search.brave.com/res/v1/web/search`, with varying query parameters.
It denies filesystem grants, and SAT supplies no raw sockets or guest environment.

`credentials.example.toml` is **trusted host configuration**, not component
metadata. It binds this tool to `BRAVE_API_KEY` and independently restricts the
key to that same method, HTTPS origin, and route. SAT injects the
`X-Subscription-Token` header only after both checks pass. The guest cannot set
that header. Redirects are not followed. No secret is stored in the component,
configuration file, command arguments, or Pi tool schema.

The Pi extension uses an argument array without a shell and exposes only `query`
to the model. It supplies `--allow-http` for this dedicated search tool; it never
accepts model-supplied filesystem grants or arbitrary SAT commands. Queries
starting with `-` remain query data. Cancellation stops the SAT child process;
execution and output are bounded. Results contain titles, URLs, and snippets,
not full page content.

## Checks

```sh
npm run check --prefix pi-extension
npm test --prefix pi-extension
cargo test --locked --manifest-path sat/Cargo.toml
cargo test --locked --manifest-path sat/Cargo.toml brave_component_offline -- --ignored
python3 sat/test.py
```

Run setup before the offline component test. It uses the actual Brave Wasm and
SAT linker with a test-only HTTP response, and exercises the guest's sandbox
probes. Other tests check destination-scoped credential injection, actual Pi
extension loading, query argument isolation, cancellation, and result bounds.
No API key, model provider, or live search is needed for these checks.
