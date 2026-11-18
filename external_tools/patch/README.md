# patch

Sandboxed unified-diff application with a GNU-style CLI subset, implemented in
Rust using `diffy` 0.5.2. This is not a GNU patch port or full CLI replacement.
No Python dependency is needed inside the component.

```sh
sat patch -p1 -d /absolute/project -i /absolute/change.diff --dry-run -- \
  --read /absolute/change.diff --read /absolute/project
sat patch -p1 -d /absolute/project -i /absolute/change.diff -- \
  --read /absolute/change.diff --write /absolute/project
```

Run the first command to preview; run the second to apply. Ordinary modifications
can use exact file grants instead of granting the whole directory. Create/delete
operations require a writable parent directory. Defaults grant no filesystem or
network access. In Codex, invoke SAT through the SAT skill's logging wrapper.

Supports standard unified hunks and Git text-diff headers, multiple distinct files,
creation/deletion through `/dev/null`, stdin or `-i FILE`, `-p NUM`, `-d DIR`,
`--dry-run`, `-R`, `-s`/`--quiet`/`--silent`, and explicit `ORIGINAL [PATCHFILE]`.
`-u`, `-t`, `--binary`, and `-F0` are accepted. Default strip behavior uses header
basenames; use `-p0` to retain paths or `-p1` for the usual `a/` and `b/` prefixes.
File contents are bytes, including non-UTF-8 text and CRLF. Existing file modes
are preserved by writing in place. Filenames must be UTF-8.

All context must match (offset matching is supported; fuzz is zero). All input
files and hunks are validated before writes. Conflicts return 1 without writing;
CLI, I/O and unsupported-format errors return 2. Success returns 0. This profile
never prompts, retries in reverse automatically, or writes backup/reject files.

Unsupported: context/normal/ed diffs, fuzzy context, rename/copy, binary patches,
Git executable/symlink/submodule modes, metadata-only changes, repeated targets,
implicit creation of directories, output redirection flags and most other GNU
options. Different old/new header names need an explicit `ORIGINAL` argument.
Absolute and traversal paths in patch headers are rejected before stripping;
symlink paths are rejected. Explicit user-selected paths still need host grants.

Limits: 16 MiB patch, 16 MiB per original/result file, 64 MiB aggregate patch +
originals + results, 256 targets. Memory use includes additional parser/application
overhead. There is no CPU fuel limit. Application is not a filesystem transaction:
write errors, a failed later create/delete, or concurrent filesystem changes can
leave partial results. Use dry-run and a recoverable working tree.

## Build and verification

```sh
bash external_tools/patch/package.sh tar
cargo test --locked --manifest-path external_tools/patch/Cargo.toml
python3 external_tools/patch/test.py --sat sat/target/release/sat
python3 external_tools/patch/test_upstream.py --sat sat/target/release/sat
```

Register `dist/patch.tar` before running the Python test drivers. The trusted host
drivers use subprocesses; local SAT calls go through the installed logging wrapper.
CI can invoke its own freshly built SAT directly when that skill is absent.

`TEST_RESULTS.json` records 37 integration checks: 18 successful differential
cases against GNU patch 2.7.6 and 19 boundary/profile checks, including expected
nonzero exits. Comparisons assert exit status and exact resulting files, not
identical diagnostic formatting. `test_upstream.py` additionally runs the original
GNU patch 2.8 `no-mode-change-git-diff` script and its original two assertions.
The launcher only supplies SAT permissions and target directory. Vendored source
hashes and origin are recorded in `upstream/manifest.json`; copyright notices are
retained. This does not claim the full GNU patch regression suite passes.

References: [GNU patch](https://www.gnu.org/software/patch/),
[diffy source](https://github.com/bmwill/diffy),
[diffy 0.5.2 API](https://docs.rs/diffy/0.5.2/diffy/).
