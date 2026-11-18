# MarkItDown comparison

The reference was selected by GitHub stars among six reviewed PDF-capable CLI
projects on 2026-11-14: Microsoft MarkItDown (195.3k), MinerU (83,642), Docling
(70.6k), Marker (40.9k), PyMuPDF (11.1k), and pdfcpu (8.9k). This is a documented
candidate comparison, not a claim to have exhaustively ranked every GitHub repo.
Counts are a dated snapshot, not a criterion for correctness.

Reference: [microsoft/markitdown](https://github.com/microsoft/markitdown), commit
[`ed178952a6672abdfd35b5f9e76a80b6cefc086e`](https://github.com/microsoft/markitdown/tree/ed178952a6672abdfd35b5f9e76a80b6cefc086e),
version 0.1.9, with its `pdf` extra. Native here means the host Python CLI;
MarkItDown is not a standalone compiled binary.

## Observed results

| Check | Native MarkItDown | SAT pdf-text |
| --- | --- | --- |
| Same 26 public PDF conversion tests | 26 passed | 9 passed, 17 failed |
| Full upstream PDF test module | 36 passed, 1 remote test skipped | 9 passed, 17 failed, 11 skipped |
| CLI execution on the same 10 PDFs | 10 succeeded | 10 succeeded, no truncation |
| Exact extracted-text agreement | Reference | 0 of 10 |

SAT skips include the remote test and ten MarkItDown-specific helper/lifecycle
tests. Their skips are recorded with reasons, never counted as passes.
Failures include missing Markdown table structure, numbering/whitespace
differences, and missing form content. The current tool is not MarkItDown
compatible; passing the SAT security suite does not establish upstream parity.

The first local timing sample used three measured invocations per PDF after one
warmup. The median of the ten per-file medians was approximately 439 ms for
native MarkItDown and 60 ms for SAT. The private registry's initial registration
took approximately 7.05 seconds. These are local observations, not equivalent-work
speedups: different engines and output quality are involved. SAT timings include
the required logging wrapper and process startup. OS caches were not cleared.
This benchmark does not isolate WASI overhead, measure memory, or establish
performance on large PDFs or other operating systems.

The dated [result snapshot](benchmarks/markitdown-2026-11-14.json) records
individual outcomes, fixture digests, sample timings, provenance, and limitations.

## Reproduce

From the repository root, after building and registering `pdf-text`:

```sh
git clone https://github.com/microsoft/markitdown.git external_tools/pdf-text/target/markitdown
git -C external_tools/pdf-text/target/markitdown checkout ed178952a6672abdfd35b5f9e76a80b6cefc086e
python3 -m venv external_tools/pdf-text/target/benchmark-venv
external_tools/pdf-text/target/benchmark-venv/bin/python -m pip install \
  'external_tools/pdf-text/target/markitdown/packages/markitdown[pdf]' pytest
external_tools/pdf-text/target/benchmark-venv/bin/python \
  external_tools/pdf-text/benchmark_markitdown.py \
  --upstream external_tools/pdf-text/target/markitdown \
  --python external_tools/pdf-text/target/benchmark-venv/bin/python \
  --output external_tools/pdf-text/target/markitdown-results --runs 3
```

On Windows, use the virtual environment's `Scripts/python.exe`. The driver
requires Git and host Python with subprocess support. Its default `--sat` is
the installed Codex SAT logging wrapper; pass another executable explicitly
where that integration is unavailable. Every local SAT invocation, including
test adapters and registration, goes through that wrapper.

The source commit is pinned, and tracked modifications to its package are
rejected. Dependency resolution is not locked by the commands above: each run
saves `native-requirements.txt`, and the dated snapshot includes the actual
resolved versions. Reuse those versions when reproducing the original run.

The driver executes the original `tests/test_pdf.py` assertions without editing
them. A pytest plugin substitutes only the converter for public local conversion
tests: SAT JSON page text is joined without Markdown synthesis or whitespace
normalization. Truncated or unsuccessful SAT output fails the assertion adapter.
Internal Python helper/lifecycle tests and remote conversions have no matching
local SAT API and are explicitly skipped in the SAT run. The native run retains
the full module, with remote access disabled.

Raw pytest reports, individual stdout/stderr, CLI help snapshots, dependency
versions, input hashes and timings are written under `--output`. Success of the
benchmark command means the comparison completed, not that the tools conform.
Use `--require-parity` to return nonzero when matched tests fail or the native
baseline suite fails. It currently fails, as the snapshot demonstrates.

## CLI compatibility

MarkItDown accepts a file or stdin and has options such as `-o` for an output
file. SAT's current `pdf-text` takes a PDF path, adds page headings or JSON,
defaults to bounded previews, and requires separate filesystem grants. Its flags,
defaults, output format, and errors do not preserve the upstream CLI.

This comparison deliberately does not add aliases or weaken expectations to
claim compatibility. Measuring native-versus-WASI runtime overhead would require
a second comparison that runs the same extraction implementation and options on
both targets. Porting MarkItDown itself would be a separate project.
