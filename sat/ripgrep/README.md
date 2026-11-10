# Ripgrep WASI component

`ripgrep` wraps ripgrep's `grep-regex`, `grep-searcher`, and `ignore` Rust
libraries in a WASIp2 component with generated WIT bindings. Searching and
directory traversal happen inside Wasm; no native `rg` binary is invoked.
SAT returns the result through WIT without inheriting guest stdout or the host
environment. The component uses the host's existing filesystem sandbox.

Build and install from the repository root (Rust with `wasm32-wasip2`):

```sh
bash sat/ripgrep/build.sh
export SAT_HOME="${SAT_HOME:-$PWD/sat/target/demo/registry}"
sat/target/debug/sat ripgrep 'fn\s+run' --paths /src --read ./sat/src::/src
sat/target/debug/sat ripgrep 'hello' --paths /src --read ./sat/src::/src -i true -C 2
sat/target/debug/sat ripgrep 'literal.text' --paths /src --read ./sat/src::/src -F true --max-results 5
```

The script installs into `SAT_HOME` (default `sat/target/demo/registry`). It
produces `sat/target/ripgrep.wasm`, with metadata and an explicit deny-HTTP policy.
Use an existing `sat` on PATH instead of `sat/target/debug/sat` if desired.
Run `sat ripgrep --help` for generated help. Boolean options take `true` or
`false`, following SAT's CLI convention. Repeat `--paths` and `--read` for
multiple roots. Without `--paths`, search starts at the guest working directory;
no files are accessible without a grant. `--read` can grant an exact file or a
directory and its descendants. Guest aliases such as `/src` keep host paths out
of results. Pass `--` before a pattern beginning with `-`.

By default, results use file headings and numbered lines, like `rg --heading -n`:

```text
/src/main.rs
11-// Entry point
12:fn run() {
13-    // ...
```

Matches use `:`, context lines use `-`, and disjoint context groups use `--`.
Files are separated by blank lines. Output is plain text without ANSI colors.
A message marks truncated results. For structured results, add `--format json`:

```json
{"results":[{"path":"/src/main.rs","line":12,"text":"fn run() {","matched":true}],"truncated":false}
```

Line numbers start at 1. Context lines have `matched: false`. UTF-8 is preserved;
invalid UTF-8 is replaced for presentation. An empty match set succeeds.
Errors fail the invocation without returning partial results or OS error paths.
Both formats return through `run: func(...) -> result<string, string>`.
SAT's separate `--json` flag wraps the returned WIT string in JSON; use
`--format json` on its own for the structured search object shown above.

Search is single-threaded and line-oriented. Recursion respects `.ignore` and
`.gitignore` even without a `.git` directory; hidden entries are skipped unless
`--hidden true`. `--no-ignore true` disables ignore files. Parent/global ignore
configuration is disabled. Recursive traversal skips symlinks and special files;
explicit symlink file inputs remain subject to WASI confinement. NUL bytes stop
searching binary inputs; preceding matches may already have been collected.
This provides a useful subset of ripgrep, with no PCRE2, subprocess preprocessors,
compressed-file search, or full `rg` flag compatibility.

Ignore files outside a filesystem grant are inaccessible. For example, granting
only `./sat` hides the repository root's `.gitignore`; `sat/.ignore` therefore
contains `target/` so searches skip build output, including nested crate builds.
For other projects, put appropriate rules in an `.ignore` file inside the granted
directory, or grant and search the repository root so its `.gitignore` is visible.
The execution deadline can still stop large searches; narrow `--paths` when needed.

This repository has `TemporaryDirectory` matches in its Python tests (and no
`Temporary` matches in `sat/src`). From the repository root, this returns matches:

```sh
SAT_HOME="$PWD/sat/target/demo/registry" ./sat/target/debug/sat ripgrep 'Temporary' \
  --paths /src --read ./sat::/src -C 2
```

Limits: 4 KiB patterns, 128 input roots, 100 context lines, 10,000 returned lines,
1 MiB of returned text plus repeated path bytes (before JSON escaping), and a
16 MiB search buffer. `--max-results` defaults to 100 and counts context lines.
`truncated` is conservatively true whenever an output limit stops the search,
including when the last returned line happens to be the last match. Context
can therefore be cut short. SAT also enforces its guest memory limit and a
30-second execution deadline using epoch interruption. Legacy fuel metadata
is ignored.

Verify matching, limits, and actual WASI filesystem confinement:

```sh
python3 sat/ripgrep/test.py
```

The test uses a temporary registry and does not require a native ripgrep.
