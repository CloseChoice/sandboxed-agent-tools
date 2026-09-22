# Random number component

A WASI P2 component that returns a random `u64` through `run()`.
The interface is in [`wit/world.wit`](wit/world.wit).

Build and run from this directory:

```sh
cargo build --locked --target wasm32-wasip2
wasmtime run --invoke 'run()' target/wasm32-wasip2/debug/random_number_component.wasm
```

Wasmtime prints the returned number. Rust's standard library adds supporting
WASI imports beyond the random interface declared in the WIT file.
