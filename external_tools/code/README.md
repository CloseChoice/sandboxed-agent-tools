# code

```sh
bash external_tools/code/package.sh  # tar (default) or zip
sat register external_tools/code/dist/code.tar
sat code outline --paths /absolute/path/to/source.rs -- --read /absolute/path/to/source.rs
```

Defaults grant no filesystem or network access. Host permissions go after `--`.
Use absolute paths; the old `HOST::GUEST` mappings are not supported by this host.

Set `WASI_SDK_PATH` to an extracted WASI SDK before building the C grammars.

Run `python3 external_tools/code/test.py` to build and verify; use
`--skip-build` to test an existing archive.

`code` embeds Tree-sitter's C runtime and the Python, Rust, JavaScript/JSX,
TypeScript, and TSX grammars. It supports `search`, `outline`, `read-symbol`, and
`read-enclosing`. Repeated `--paths` select granted files or directories.
Search modes are literal text and regex; extraction supports full definitions,
signatures, and signatures with documentation. `--format json` includes source
ranges, hashes, hit locations, and parse status. Use `--max-results` for previews.

Install Rust's `wasm32-wasip2` target and set `WASI_SDK_PATH` to an extracted
[WASI SDK 34](https://github.com/WebAssembly/wasi-sdk/releases/tag/wasi-sdk-34).
Use the SDK matching your build platform; the component needs no SDK at runtime.
The package script uses SDK clang/llvm-ar only for C compilation. Rust supplies
the WASIp2 linker and libc. Cross-platform execution still needs CI verification.

All execution is bounded by the host fuel ceiling. Package metadata can lower
the budget but cannot increase it. No host environment or network authority is
inherited.
