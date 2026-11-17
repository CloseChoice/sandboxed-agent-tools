# SAT 0.1.0 release preparation

The development checkpoint is preserved on `new-architecture`. The
`release/0.1.0` branch publishes only the tools selected in
[external_tools/release-tools.toml](external_tools/release-tools.toml):

- Python: sandboxed CPython 3.14 with the documented standard-library profile.
- jaq: the upstream jq-compatible CLI.
- code: Tree-sitter retrieval for Python, Rust, JavaScript, and TypeScript/TSX.
- MarkItDown: the local PDF conversion profile, without OCR or remote backends.
- ripgrep: text search using upstream ripgrep libraries through SAT's CLI.

Other tool sources remain available for development and are excluded from
the release build matrix, complete archive checks, and published assets.

Validate versions with `python3 scripts/release.py`. Build the selected packages
with each tool's `package.sh`; the release workflow derives its matrix from
the same selection file and rejects missing or extra archives.

Before publishing, require the release workflow and host checks to pass.
Review the committed sources and version, then create an annotated `v0.1.0`
tag and push it explicitly. A branch push or manual workflow run produces
Actions artifacts only. No tag or publication is part of this preparation.

## Native SAT downloads

The workflow also builds and tests native SAT executables on:

- Linux x86_64 (Ubuntu 24.04 / GNU libc).
- macOS ARM64 (Apple Silicon).
- macOS x86_64 (Intel).
- Windows x86_64 (MSVC).

The platform matrix and packaging checks live in `scripts/host_release.py`.
Each job runs the host tests, packages the executable, extracts that archive,
and uses the extracted executable to register and run the same portable jaq
component. This checks the artifact users download, including executable modes.

Unix hosts use `sat-<version>-<target>.tar.gz`; Windows uses
`sat-<version>-<target>.zip` containing `sat.exe`. Extract the matching
archive and place the executable on PATH. The five tool archives remain portable
across these hosts. The release checksum file covers host and tool archives.
Publishing requires both package validation and all native host jobs to pass.
Branch runs assemble the same complete release bundle, verify checksums, and
upload it as the sat-release Actions artifact. Only tag runs publish that bundle
as a GitHub release.

Run packaging checks locally with:
`python3 -m unittest discover -s scripts -p 'test_*.py'`.
