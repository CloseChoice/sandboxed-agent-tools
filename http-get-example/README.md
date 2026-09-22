# HTTP GET guest

A WASI P2 component using standard `wasi:http`. No custom Rust host.
Its WIT export is `run(method: string, url: string) -> result<string, string>`.

From this directory:

```sh
cargo build --locked -p http-get-component --target wasm32-wasip2 --release
wasmtime run -S http --invoke 'run("GET", "https://httpbin.org/get?hello=world")' target/wasm32-wasip2/release/http_get_component.wasm
```

Wasmtime prints the returned result containing httpbin's mirrored JSON.
Change the URL (including query parameters) inside `run(...)` as needed.
This guest supports only GET, reads at most 64 KiB and does not follow redirects.

`-S http` enables outbound WASI HTTP. The Wasmtime CLI does not provide a
GET-only or destination allow-list option. The guest's GET check is behavior,
not a security boundary: enforcing those restrictions on untrusted components
requires a custom host policy. No `inherit-network` flag is needed.
