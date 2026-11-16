# SAT 0.1.0 release preparation

The development checkpoint is preserved on `new-architecture`. The
`release/0.1.0` branch publishes only the tools selected in
[external_tools/release-tools.toml](external_tools/release-tools.toml):

- Python: sandboxed CPython 3.14 with the documented standard-library profile.
- jaq: the upstream jq-compatible CLI.
- code: Tree-sitter retrieval for Python, Rust, JavaScript, and TypeScript/TSX.
- MarkItDown: the local PDF conversion profile, without OCR or remote backends.

Other tool sources remain available for development and are excluded from
the release build matrix, complete archive checks, and published assets.

Validate versions with `python3 scripts/release.py`. Build the selected packages
with each tool's `package.sh`; the release workflow derives its matrix from
the same selection file and rejects missing or extra archives.

Before publishing, require the release workflow and host checks to pass.
Review the committed sources and version, then create an annotated `v0.1.0`
tag and push it explicitly. A branch push or manual workflow run produces
Actions artifacts only. No tag or publication is part of this preparation.
