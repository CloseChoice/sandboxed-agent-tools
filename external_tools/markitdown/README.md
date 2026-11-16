# MarkItDown PDF CLI for WASI

Reference: [Microsoft MarkItDown](https://github.com/microsoft/markitdown), about
195.3k GitHub stars on 2026-11-14, ahead of the other reviewed PDF-capable tools
(including Stirling-PDF, MinerU, Docling, and Marker).

This profile reuses the unmodified upstream 0.1.9 argument parser and PDF
converter at commit `ed178952a6672abdfd35b5f9e76a80b6cefc086e`. It adds
PDF-only orchestration with the same upstream output normalization. Text and
table extraction use pinned pdfminer.six and pdfplumber, not a substitute lopdf
formatter. The original `pdf-text` component is unchanged.

SAT runs this profile without fuel metering. The original upstream PDF/CLI
assertions pass for the supported local PDF profile; see the results below.

The full native `[pdf]` dependency closure and the branches excluded from this
WASI profile are documented in [DEPENDENCIES.md](DEPENDENCIES.md), with exact
edges and reference versions in [DEPENDENCIES.json](DEPENDENCIES.json).

## Build and interface

```sh
bash external_tools/markitdown/package.sh
sat register external_tools/markitdown/dist/markitdown.tar
sat markitdown --help
sat markitdown --version
sat markitdown /absolute/input.pdf -- --read /absolute/input.pdf
sat markitdown /absolute/input.pdf -o /absolute/output.md -- --read /absolute/input.pdf --write /absolute/output-directory
# stdin uses the upstream CLI and requires no input path grant:
cat input.pdf | sat markitdown -x pdf
```

Use the Codex SAT logging wrapper for local invocations. Upstream tool arguments
pass through unchanged; SAT permissions follow a separate `--`. Local filenames,
stdin, `-o`, `-x`, `-m`, `-c`, help/version, and `--keep-data-uris` retain the
upstream parser. Output is Markdown with the upstream print newline, without
SAT page headings or implicit preview truncation. Standard WASI grants enforce
reads/writes; the Python facade is not the security boundary.

The profile explicitly excludes non-PDF formats, OCR, remote URL fetching,
cloud conversion backends, third-party plugins, encrypted PDFs, and native
rendering/Ghostscript repair. Those options are not silently approximated.
The CLI parser still documents the original options; unsupported backends fail
explicitly. Inputs are capped at 16 MiB. Fuel metering is disabled by the host.
No host environment, native process execution or ambient network is inherited.

## Source and dependency provenance

`upstream/` contains the original CLI, PDF converter, base types, exceptions,
version and MIT license, with source hashes in its manifest. `pdf_profile.py`
replaces only format dispatch/orchestration for this PDF build. `app.py` exports
`run(args: list<string>) -> s32`, catches CLI exit statuses, and leaves stdio
and filesystem enforcement to SAT's WASI host.

The trusted builder runs natively and pins componentize-py 0.25.0,
pdfminer.six 20260107, pdfplumber 0.11.10, and charset-normalizer 3.5.2.
Only Python dependency sources are copied; host extension binaries are excluded.
Two port changes are explicit in `build.py`: unavailable cryptography imports
are removed and encrypted documents rejected, and Ghostscript repair rejects
instead of importing native subprocess support. Upstream text/table algorithms
and CLI parsing are unmodified. Generated builder/profile/component files live
under ignored `target/`; archives live under ignored `dist/`.

The build cache keys wrapper/WIT/vendor/build sources, pinned dependencies and
host Python version, and verifies the cached component digest before reuse.
There is no runtime pip installation or arbitrary PyPI compatibility.

## Original upstream tests

The upstream checkout must be at the exact commit above and unmodified:

```sh
/path/to/native-markitdown-venv/bin/python external_tools/markitdown/test_upstream.py \
  --upstream /path/to/markitdown-checkout \
  --python /path/to/native-markitdown-venv/bin/python \
  --output external_tools/markitdown/target/upstream-results
```

The native environment needs the pinned upstream package's `pdf` extra and
pytest. SAT must have this component registered in the selected `SAT_HOME`.
The driver runs the actual `test_pdf.py` and PDF cases from
`test_cli_vectors.py`. Assertions and fixtures remain unchanged and their
source hashes are verified before/after execution. A process-boundary adapter
provides API results and redirects original CLI subprocess commands through
SAT with explicit test grants. The API adapter removes exactly the one newline
added by the upstream CLI's `print`; it does not reformat or synthesize output.

Remote tests and internal Python implementation/lifecycle tests have explicit
skips for the SAT process boundary. Non-PDF CLI vectors are deselected on both
backends. The driver saves every failure and returns nonzero when either suite
fails. Failures are not marked xfail or counted as passes.

## Current test results (2026-11-14)

The [current upstream run](UPSTREAM_RESULTS.json) uses the original tests at the
pinned commit: native 48 passed / 5 remote skips; WASI 38 passed / 15 remote or
internal-only skips. All 38 matched conversion/CLI cases pass on both backends.
The extra ten WASI skips cover internal Python helper/lifecycle tests that have
no direct equivalent across the process boundary. Non-PDF CLI vectors are
deselected equally. This does not claim support for every MarkItDown format,
cloud integration, plugin, or upstream test outside the documented PDF profile.

The separate `test_cli.py` suite passed 28 native/WASI CLI and confinement
checks, including stdin, relative/absolute filenames, output files, parser
error statuses, and denied filesystem access. Native Python 3.12 and WASI Python
3.14 format argparse metavars differently in help; option names and usage match,
but literal help byte parity is not claimed. Other tested parser output and
conversion output match natively.

The [previous limited-fuel run](UPSTREAM_RESULTS_LIMITED.json) is preserved as
history: 25 cases exhausted the old fuel ceiling. With metering disabled those
same unchanged assertions pass. Legacy package `fuel` metadata is ignored.
