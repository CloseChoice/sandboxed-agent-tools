# jaq 3.1.2

This package embeds the **upstream jaq CLI source**, parser, evaluator, help,
and data-format support in a WASIp2 component. Tool arguments pass through
unchanged; SAT does not generate a replacement jaq argument parser.

```sh
bash external_tools/jaq/package.sh
sat register external_tools/jaq/dist/jaq.tar
sat jaq --help
sat jaq --version
printf '%s\n' '{"name":"Ada"}' | sat jaq -r '.name'
sat jaq -nc --arg name Ada '{name: $name}'
sat jaq -r '.users[] | select(.active) | .name' \
  external_tools/jaq/examples/users.json -- --read "$PWD/external_tools/jaq/examples/users.json"
```

Use upstream syntax: positional files, clustered flags (`-nrc`), `--arg NAME
VALUE`, `--argjson NAME JSON`, `-f FILE`, `-L DIR`, and standard stdin. Boolean
flags take no `true` value. Upstream help/version and exit statuses are preserved.
The former SAT-only `--input`, `--files`, `--stdin`, and `NAME=VALUE` binding
syntax have been removed. JSON, YAML, TOML, XML, CBOR, raw text, and NUL-delimited
formats follow upstream jaq's supported `--from` / `--to` options.

Sandbox authority remains separate: append `-- --read ABSOLUTE_PATH` (and
`--write` when needed). Relative tool filenames resolve from the invocation's
working directory. The grants themselves use absolute paths or `$PWD/` syntax.
An upstream `--` is passed through unless it introduces SAT permission options.
No host environment variables or network authority are inherited. `PWD` is a
synthetic working directory and grants no filesystem authority. Input is read
from the invoking stdin stream; output is captured with host limits of 16 MiB
stdout and 1 MiB stderr. Tools cannot access the registry or native-code cache.

Registration compiles the component for the current host before reporting
success. Normal runs reuse the compiled cache.

See [upstream jaq documentation](https://github.com/01mf02/jaq) and
[vendored-source notes](upstream/README.sat.md) for provenance and WASI adapters.
Rustyline uses its upstream wasm terminal backend; host terminal/history access
is not inherited. Filesystem operations remain subject to WASI and host policy. `-i` requires a
writable directory grant for temporary files and atomic replacement. WASI has
no chmod operation; replacement-file modes follow host creation defaults.

Verify CLI behaviour against a native binary built from the same upstream source:

```sh
python3 external_tools/jaq/test.py
# After both binaries and the archive have been built:
python3 external_tools/jaq/test.py --skip-build
```

The checks compare output, diagnostics, and exit statuses for upstream examples,
flags, positional files, stdin, bindings, modules, formats, and error cases, then
verify denied reads and symlink isolation.

## Upstream credits and license

[jaq](https://github.com/01mf02/jaq) is developed by Michael Färber and its
contributors. The vendored source is used under its
[MIT license](upstream/LICENSE-MIT); the retained text is also distributed in
[THIRD_PARTY_NOTICES.txt](THIRD_PARTY_NOTICES.txt). SAT supplies the WASI adapter;
the upstream CLI and evaluator remain jaq's work.
