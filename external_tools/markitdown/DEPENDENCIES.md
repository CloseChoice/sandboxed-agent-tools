# Python dependency scope

The component is a restricted PDF text profile, not an installation of the full
upstream `markitdown[pdf]` dependency tree. Pure Python packages can run on WASI
CPython when their standard-library and system-service requirements are available.
Native extensions and bundled native libraries require WASI builds or replacements.
Bundling Python sources alone does not establish WASI compatibility.

## Native reference closure

Resolved from installed distribution metadata in the CPython 3.12 Linux reference
environment on 2026-11-14. Includes base and `[pdf]` dependencies, not `[all]`,
cloud/plugin extras, test tools or the component builder. This is an audit snapshot,
not a cross-platform dependency lock. See `DEPENDENCIES.json` for exact edges.

| Package | Reference version | Direct dependencies in this scope | Native artifacts |
| --- | --- | --- | --- |
| beautifulsoup4 | 4.15.0 | soupsieve, typing-extensions | no |
| certifi | 2026.10.23 | — | no |
| cffi | 2.1.1 | pycparser | yes |
| charset-normalizer | 3.5.2 | — | yes |
| click | 8.5.1 | — | no |
| cryptography | 50.0.2 | cffi | yes |
| defusedxml | 0.7.1 | — | no |
| flatbuffers | 25.12.19 | — | no |
| idna | 3.20 | — | no |
| magika | 1.0.3 | click, onnxruntime | no |
| markdownify | 1.2.3 | beautifulsoup4, six | no |
| markitdown | 0.1.9 | beautifulsoup4, certifi, charset-normalizer, defusedxml, magika, markdownify, pdfminer-six, pdfplumber, requests | no |
| numpy | 2.5.4 | — | yes |
| onnxruntime | 1.32.0 | flatbuffers, numpy, packaging, protobuf | yes |
| packaging | 26.3 | — | no |
| pdfminer-six | 20260107 | charset-normalizer, cryptography | no |
| pdfplumber | 0.11.10 | pdfminer-six, pillow, pypdfium2 | no |
| pillow | 13.0.0 | — | yes |
| protobuf | 7.36.2 | — | yes |
| pycparser | 3.12 | — | no |
| pypdfium2 | 5.15.2 | — | yes |
| requests | 2.34.2 | certifi, charset-normalizer, idna, urllib3 | no |
| six | 1.17.0 | — | no |
| soupsieve | 4.1 | — | no |
| typing-extensions | 4.16.0 | — | no |
| urllib3 | 2.8.0 | — | no |

## Native libraries below the Python package layer

The 26-package graph describes Python distributions, not the complete native
shared-library closure of their wheels. Supporting the excluded features can
also require porting or replacing OpenSSL and cryptography's native code,
libffi for CFFI, PDFium, Pillow's enabled image codecs, ONNX Runtime and its
kernels, and NumPy's native implementation (with BLAS/LAPACK where configured).
The exact native-library closure depends on the wheel/build configuration.
Those libraries do not become WASI-compatible merely because CPython is ported.
The current PDF text component avoids these branches rather than porting them.

## What the WASI component contains

- CPython 3.14 runtime supplied by `componentize-py`.
- Vendored MarkItDown 0.1.9 CLI/parser/converter with PDF-only orchestration.
- `pdfplumber==0.11.10`, `pdfminer.six==20260107`, and the pure Python code of `charset-normalizer==3.5.2`.
- Build-time tool: `componentize-py==0.25.0`; host venv/pip dependencies are not component runtime dependencies.

`build.py` installs with `--no-deps` and copies only the selected source packages.
It explicitly rejects encrypted PDFs before crypto initialization and disables
Ghostscript repair. Pillow/PDFium rendering and Magika/ONNX type detection are not
included. This does not port the excluded dependency branches; restoring those
features requires their own WASI implementations or suitable replacements.

The unchanged upstream tests cover the supported PDF text/CLI path; they do not
prove compatibility of the excluded APIs or full multi-format MarkItDown.
