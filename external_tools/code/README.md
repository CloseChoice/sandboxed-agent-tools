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

SAT runs with fuel metering disabled. Legacy package `fuel` values are
accepted for compatibility and ignored. No host environment or network authority is
inherited.

## Path selectors

`--path` and `--paths` accept the same repeatable selectors; values from both
options are combined (union). Quote patterns to prevent shell expansion.

```sh
sat code outline --path '/project/src/**/*.ts' --path '/project/lib/**/*.tsx' --exclude '**/*.test.ts' -- --read /project/src --read /project/lib
sat code outline --path 'regex:/project/src::(^|/)[^/]+\.(ts|tsx)$' --exclude node_modules -- --read /project/src
```

- Literal file/directory: directories are recursive. Existing paths are literal
  even if their names contain glob metacharacters. Use `glob:` to force glob syntax.
- Globs: `*` matches within one path segment, `**` crosses directories, `?`
  matches one character, `[ab]` matches a character class, `{ts,tsx}` alternates.
  The literal directory prefix before the first wildcard is the traversal root.
- Regex: `regex:ROOT::EXPR` explicitly bounds traversal to ROOT. EXPR searches
  root-relative paths using Rust regex syntax; use `^` and `$` for a full match.
- Repeatable `--exclude` accepts globs or `regex:EXPR`. Exclusions override all
  inclusions and match full paths or paths relative to any traversal root.
  Slash-free globs also match basenames, so `--exclude node_modules` prunes that
  directory. Use `**/*.test.ts` to exclude matching filenames at any depth.
- Matching uses `/` separators and is case sensitive. Ignore-file/hidden-file
  defaults remain enabled; directory symlinks are not followed. Adding a selector
  does not override ignore rules. At most 128 selectors and 128 exclusions are
  accepted, each at most 4096 bytes. No matches returns an empty result.
- Use absolute selector roots in SAT. Patterns grant no permissions: explicitly
  grant traversal roots with `--read` after `--`. Traversal outside those grants
  is denied. `read-enclosing` still requires a single literal file.
