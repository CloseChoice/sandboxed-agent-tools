# SAT dashboard

A local dashboard for SAT tool usage, built with Svelte 5 and Vite. The Rust host
embeds the production assets, so users do not need Node.js to open it.

```sh
# Use the same SAT_HOME as your installed tools.
export SAT_HOME="$PWD/sat/target/demo/registry"
export PATH="$PWD/sat/target/release:$PATH"

sat dashboard enable
sat uutils --input $'first\nsecond\n' head -n 1
sat dashboard --port 7331
```

`sat dashboard` serves at `http://127.0.0.1:7331` and opens your browser.
`--port PORT` selects a port; `--port 0` chooses an available port. Use `--no-open`
for headless use. Ctrl+C stops the server; it does not change recording preferences.
The server binds only to IPv4 loopback, serves read-only endpoints, and rejects
foreign Host/Origin headers. Assets and data are served locally with no CDN or
analytics requests.

## Recording controls

```sh
sat dashboard enable   # Opt in for future tool invocations.
sat dashboard disable  # Stop new records; retain history.
sat dashboard reset    # Delete the SQLite database and its journal/sidecar files.
```

Recording is **off by default**. Opening the dashboard does not enable it.
The setting is scoped to `SAT_HOME`, in `dashboard.enabled`; history is stored in
`SAT_HOME/executions.sqlite3`. Reset preserves the recording setting: when enabled,
the next invocation creates a fresh database. When disabled, reset leaves history
deleted until you opt in again. These commands work while the dashboard is open;
the page refreshes every ten seconds, or with the Refresh button.

History covers tool invocations (`sat python …`, `sat ripgrep …`, etc.), including
failed loads, missing tools, and help requests. SAT management commands such as
`list`, `install`, and `dashboard` are excluded. Each record contains:

- Tool name and complete argument array, preserving argument boundaries and flags.
- Start time (Unix milliseconds), elapsed wall time, success/failure, and exit code.
- The error message on failure; an unfinished status if no completion was recorded.

Wall time includes tool loading and execution. A process that is killed before
completion leaves an unfinished record; this is not counted as a success or failure.
Disabling stops new records; already-started invocations can still finish their
existing records. Reset coordinates with readers and writers across SAT processes;
an old invocation cannot update a new row after reset.

Arguments may contain paths, inline source, or secrets, so enabling records those
values exactly as requested. Piped stdin, environment variables, and command
stdout/stderr are not stored. The host does not automatically grant the history
database to guest tools. History failures are reported on stderr without changing
the underlying tool's exit status.

## Dashboard

The interactive pie chart shows execution counts by tool. Click a slice or legend
entry to see that tool's invocations. The execution table supports text search,
status and tool filters, and pagination through the complete history. Open an
execution to see every argument, its duration, exit code, and any error. The copied
command uses POSIX shell quoting; the numbered argument list is authoritative on
all platforms.

## Build

Requires Node.js 20.19+ or 22.12+ for the frontend, and Rust for the host:

```sh
cd sat/dashboard
npm ci
npm run build
cd ../..
cargo build --locked --release --manifest-path sat/Cargo.toml
```

The generated `assets/` are included in source control so a Rust-only build works.
After changing the Svelte sources, rebuild them before rebuilding SAT. SQLite is
bundled with the host; no system SQLite installation is needed.

For frontend development, run `sat dashboard --no-open` on port 7331, then
`npm run dev` in this directory. Vite proxies `/api` to the local SAT server.

## Verification

```sh
cargo test --locked --manifest-path sat/Cargo.toml
npm test --prefix sat/dashboard
python3 sat/dashboard/test.py --skip-build
# Also exercise the actual UI in Chromium:
npx --prefix sat/dashboard playwright install chromium
python3 sat/dashboard/test.py --skip-build --browser
```

The integration suite uses a temporary SAT_HOME. It checks opt-in/off/reset,
unchanged command output and exit status, exact arguments, concurrent invocations,
filtering/pagination, loopback serving, cross-origin/Host rejection, and unavailable
filesystem routes. Browser checks cover pie drilldown, filters, full argument
inspection, safe rendering of stored strings, keyboard dismissal, and mobile layout.
Preview screenshots are written to `sat/target/dashboard-preview/`.
