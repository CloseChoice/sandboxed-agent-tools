# uutils for SAT

A WASI component calling the upstream [uutils coreutils](https://github.com/uutils/coreutils)
0.12.0 implementations. No host-native commands are executed.

Included commands: `head`, `tail`, `cat`, `wc`, `cut`, `sort`, `uniq`, `nl`, `ls`,
`basename`, and `dirname`. Their usual flags are passed through unchanged.
`top` is not part of coreutils; process monitoring would require a separate host
capability and is not included.

## Build and use

From the repository root, with Rust and the `wasm32-wasip2` target installed:

```sh
./sat/uutils/build.sh
export SAT_HOME="$PWD/sat/target/demo/registry"
export PATH="$PWD/sat/target/release:$PATH"

sat uutils --read ./sat/src::/src head -n 20 /src/main.rs
sat uutils --read ./sat/src::/src ls -lh /src
sat uutils --read ./sat/src/main.rs::/main.rs wc -l /main.rs
printf 'pear\napple\npear\n' | sat uutils --stdin sort -u
sat uutils --input $'one\ntwo\n' tail -n 1
sat uutils head --help
```

**Put SAT flags (`--read`, `--read-write`, `--stdin`, `--input`, `--json`)
before the utility name.** Everything after the utility name belongs to that
utility. A utility's `--` still separates its options from filenames, for example
`sat uutils --read ./data::/data cat -- /data/-file`.

The build script defaults to a release host; `SAT_BUILD_PROFILE=debug` selects a
debug host. `SAT_HOME` overrides the installation registry.

## Permissions and output

Only explicitly granted files/directories are visible. Reads need `--read`;
writing with `sort -o` needs `--read-write`. No host environment, network,
process table, or terminal is inherited. Stdin is empty unless `--stdin` or
`--input` is supplied; both accept UTF-8 text up to 8 MiB. Binary files can be
read through grants, and binary stdout is preserved.

SAT captures the upstream utilities' WASI stdout/stderr in private memory, then
presents them after successful component execution. Each stream must remain
below 8 MiB; reaching the bound fails rather than returning a truncated success.
The component retains the host's 64 MiB memory limit and 30-second deadline
enforced using epoch interruption. This adapter supports bounded, one-shot commands; interactive
use and continuous `tail -f` streaming are not supported.

By default, SAT writes the captured bytes unchanged and preserves the utility's
exit status. For structured output:

```sh
sat uutils --json --read ./sat/src::/src head -n 2 /src/main.rs
```

This returns `{"stdout":"…","stderr":"","exit_code":0}`. A stream that is not
valid UTF-8 is represented as an array of byte values instead of a string.
`--json --help` describes the SAT wrapper; `--json head --help` captures the
upstream utility help. Output capture is an adaptation for upstream stdio-based
code, not a new pipeline transport protocol.

## Tests

```sh
python3 sat/uutils/test.py
# After a build:
python3 sat/uutils/test.py --skip-build
```

Tests execute every utility inside the real WASI sandbox, compare text commands
with native utilities when installed, and check missing grants, path traversal,
symlink escape, read-only writes, exact-file grants, binary output, output bounds,
and exit statuses. The component imports no network interfaces.

uutils is MIT-licensed; its notice is included in `LICENSE.uutils`.
Dependency versions are pinned by `Cargo.lock`.
