# SAT 0.1.0 release preparation

The development checkpoint is preserved on `new-architecture`. The
`release/0.1.0` branch publishes only the tools selected in
[external_tools/release-tools.toml](external_tools/release-tools.toml):

- Python: sandboxed CPython 3.14 with the documented standard-library profile.
- jaq: the upstream jq-compatible CLI.
- code: Tree-sitter retrieval for Python, Rust, JavaScript, and TypeScript/TSX.
- MarkItDown: the local PDF conversion profile, without OCR or remote backends.
- ripgrep: text search using upstream ripgrep libraries through SAT's CLI.

The `backup-all-tools` branch preserves the experimental tool sources.

Validate versions with `python3 scripts/release.py`. Build the selected packages
with each tool's `package.sh`; the release workflow derives its matrix from
the same selection file and rejects missing or extra archives.

Before publishing, require the release workflow and host checks to pass.
Review the committed sources and version, then create an annotated `v0.1.0`
tag and push it explicitly. A branch push or manual workflow run produces
Actions artifacts only. No tag or publication is part of this preparation.

## Native SAT downloads

The workflow builds and tests the Linux x86_64 SAT executable on Ubuntu 24.04.
It packages the executable and uses the extracted archive to register and run
the portable jaq component.

The release includes `sat-<version>-x86_64-unknown-linux-gnu.tar.gz`, the five
tool archives, `install.sh`, and `SHA256SUMS`. Publishing requires both the host
job and package validation to pass.
Branch runs assemble the same complete release bundle, verify checksums, and
upload it as the sat-release Actions artifact. Only tag runs publish that bundle
as a GitHub release.

Run packaging checks locally with:
`python3 -m unittest discover -s scripts -p 'test_*.py'`.
