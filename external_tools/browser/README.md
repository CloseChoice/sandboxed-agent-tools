# Browser

A Rust WASIp2 component for controlling real Chrome. DOM selection, interaction
logic, waits, snapshots, diagnostic filtering and screenshot decoding run inside
WebAssembly. Chrome remains native. SAT supplies only the CDP transport through
`sat:browser/connection@0.1.0`.

## Build and start

```sh
cargo build --locked --manifest-path sat/Cargo.toml
bash external_tools/browser/package.sh
sat/target/debug/sat register external_tools/browser/dist/browser.tar
```

In one terminal, start a dedicated session:

```sh
sat/target/debug/sat chrome-session
# Optional: --headed --chrome /path/to/chrome
```

The host creates a fresh temporary profile and prints a JSON line containing
`endpoint`. Keep stdin open. Closing stdin or pressing Ctrl-C shuts down Chrome
and removes the profile. The launcher preserves Chrome's OS sandbox; it never
adds `--no-sandbox` or disables web security. Chrome must already be installed.

For an agent's noninteractive shell, use `chrome-session --duration 600` in a
managed background command. It stays alive for up to 600 seconds even when stdin
is closed; Ctrl-C still stops it early. The supported duration is 1–3600 seconds.

In another terminal, copy that endpoint:

```sh
SAT_CDP='ws://127.0.0.1:PORT/devtools/browser/ID'
sat/target/debug/sat browser navigate --value http://127.0.0.1:8000 -- --allow-cdp "$SAT_CDP"
sat/target/debug/sat browser wait-visible --target '#save' -- --allow-cdp "$SAT_CDP"
sat/target/debug/sat browser snapshot -- --allow-cdp "$SAT_CDP"
sat/target/debug/sat browser fill --target '#name' --value 'Ada Lovelace' -- --allow-cdp "$SAT_CDP"
sat/target/debug/sat browser click --target '#save' -- --allow-cdp "$SAT_CDP"
sat/target/debug/sat browser wait-text --target '#status' --value Saved -- --allow-cdp "$SAT_CDP"
mkdir -p artifacts
sat/target/debug/sat browser screenshot --output "$PWD/artifacts/page.png" -- --allow-cdp "$SAT_CDP" --write "$PWD/artifacts"
```

Output is JSON text returned through the component export. SAT's generic `--json`
flag wraps string results as JSON strings; the browser already returns JSON, so
that additional flag is unnecessary. Screenshot bytes stay out of the response.
An individual granted file must exist; a granted directory permits new files.

## Commands

| Action | Arguments and result |
| --- | --- |
| `tabs` | List pages in this Chrome instance |
| `new` | `--value URL` (default `about:blank`); returns `targetId` |
| `close` | Close the selected page |
| `navigate` | `--value URL`; starts navigation, then use a concrete wait |
| `snapshot` | Bounded text and interactive elements with CSS selectors |
| `query` | `--target CSS`; text, value, HTML and bounds, or null |
| `click` | `--target CSS`; scroll into view and send mouse input |
| `fill` | `--target CSS --value TEXT`; replace an editable field's contents |
| `type` | `--target CSS --value TEXT`; click and insert text at the caret |
| `press` | `--value Enter`, `Tab`, `Escape`, `Backspace`, `Delete`, or an arrow key |
| `scroll` | `--value '{"x":0,"y":500}'` |
| `evaluate` | `--value JAVASCRIPT`; await promises and return a JSON value |
| `wait-visible` | `--target CSS` |
| `wait-text` | `--target CSS --value TEXT`; wait for visible matching text |
| `screenshot` | `--output PATH`; `--value full` for full-page PNG |
| `diagnostics` | Console, exceptions, request/response summaries, failed requests |
| `batch` | `--value JSON_ARRAY`; ordered steps inside one invocation |
| `cdp` | `--value '{"method":"Page.getFrameTree","params":{}}'`; page-session CDP |

Use `--tab TARGET_ID` when multiple pages exist; the tool refuses to guess. Page
state persists between CLI invocations, while each invocation attaches a fresh
CDP session. Selectors support `>>>` for open shadow roots and same-origin frames,
for example `#widget >>> button` and `#frame >>> #name`.

Snapshots stop after 200 elements or 5,000 visited nodes, limit extracted text,
and redact password-field values. They are previews, not complete DOM exports.
Use `query`, `evaluate`, or `cdp` to inspect further. High-level selectors do not
traverse closed shadow roots or cross-origin frames; those require explicit CDP
execution-context/target handling. Geometrically transformed frames are not
handled by the high-level click coordinate helper.

Clicks check visibility, disabled state and coverage at the click point. Mouse
and text input go through Chrome's Input domain, so normal event handlers and
framework input state run. `press` operates on the focused element. Click
completion does not imply that subsequent application work has finished.

## Batches and diagnostics

```sh
sat/target/debug/sat browser batch --value '[
  {"action":"fill","target":"#name","value":"Ada"},
  {"action":"click","target":"#save"},
  {"action":"wait-text","target":"#status","value":"Saved","timeout":10000},
  {"action":"query","target":"#status"},
  {"action":"diagnostics","timeout":500}
]' -- --allow-cdp "$SAT_CDP"
```

The component performs polling internally. Intermediate protocol messages and
events stay inside the runtime; the response includes each step's selected
result. A failed step stops the batch and reports its index; earlier side effects
are not rolled back. Batches accept 1–100 steps. Per-step waits accept 1–30000 ms,
defaulting to 5000 ms. The component checks a 120-second budget between steps.

Diagnostics cover **this invocation only**. Put the action, wait and diagnostics
in one batch. Separate CLI calls do not retain event history. The host buffers up
to 256 events / 1 MiB; the component retains 128 diagnostic summaries and reports
dropped events. Diagnostics omit headers and response bodies. Host CDP commands
have a 15-second transport deadline and 16 MiB message limit. A timed-out
connection is discarded. Full screenshots are limited to 16 million pixels by
this component.

## Authority

Defaults grant no filesystem, HTTP or browser access. `--allow-cdp` grants exactly
one canonical `ws://127.0.0.1:PORT/devtools/browser/ID` connection. Other hosts,
schemes, paths, userinfo, queries and fragments are rejected. WebSocket handshake
redirects are not followed. The imported transport accepts CDP commands, not
destinations. Raw guest sockets and DNS remain disabled. Ordinary HTTP grants do
not enable CDP. Configuration can set `cdp-endpoint` under a tool's table; CLI
`--allow-cdp` replaces it. Prefer transient CLI grants since session URLs change.

**The grant controls the entire Chrome instance behind that endpoint.** It is not
a per-tab security boundary. Use the dedicated launcher to isolate personal
profiles. SAT validates endpoint syntax, but cannot authenticate that a manually
supplied endpoint came from its launcher. The debugging port is not protected
against other native processes running as the user.

Chrome's own filesystem and network authority is separate from WASI's. CDP and
page JavaScript can cause Chrome to read files, access browser storage, or make
network requests without WASI filesystem/HTTP grants. This implementation does
not provide browser egress restrictions or filesystem confinement. A screenshot's
guest-side write requires a filesystem grant, but unrestricted CDP also has native
browser capabilities. Use explicitly authorized development sessions.

## Real-browser verification

```sh
python3 external_tools/browser/test.py --artifacts external_tools/browser/dist/test-results
# After building: add --skip-build
cargo test --locked --manifest-path sat/Cargo.toml
```

The suite serves `tests/app.html` locally and drives it through the installed
WASI component. It checks state across invocations, trusted input, Unicode and
multiline editing, contenteditable, delayed saves, shadow DOM, same-origin frames,
scrolling, coverage errors, JavaScript timeouts, viewport/full-page PNGs, failed
requests, exceptions, explicit tab selection, endpoint validation, denied writes,
and inability to attach to a target in another Chrome process. Host tests cover
exact handshake routing, event interleaving/bounds, host-assigned request IDs and
refusal to follow redirects.

Artifacts include the actual screenshot, snapshot, diagnostics and a report.
Only Linux has been verified locally; CI includes Linux, macOS and Windows Chrome
integration runs. Tests need permission to open local sockets and launch Chrome.
