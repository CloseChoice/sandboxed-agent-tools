# PDF text extraction

`pdf-text` extracts embedded text inside a WASI component using pinned
[`lopdf`](https://docs.rs/lopdf/0.46.0/lopdf/struct.Document.html). It returns
page-labelled text or JSON without requiring an output-file grant.

```sh
bash external_tools/pdf-text/package.sh           # dist/pdf-text.tar
sat register external_tools/pdf-text/dist/pdf-text.tar
sat pdf-text /absolute/report.pdf -- --read /absolute/report.pdf
sat pdf-text /absolute/report.pdf --max-pages 2 --format json -- --read /absolute/report.pdf
sat pdf-text /absolute/report.pdf --start-page 3 -- --read /absolute/report.pdf
```

Follow your agent integration's logging wrapper when invoking SAT. Tool options
go before `--`; host grants go after it. Defaults grant no filesystem or network
access. A path argument alone grants nothing. Input files are opened for reading
only. The guest does not invoke native PDF programs, run PDF JavaScript, or fetch
external resources.

JSON output (`--format json`) has `total_pages`, `pages`, `next_page`, and
`truncated`. Each page has its one-based `number`, UTF-8 `text`, and `truncated`
flag. The default preview covers at most 20 pages and 65,536 text bytes.
`--max-pages` accepts 1–200; `--max-bytes` accepts 1–1,048,576. Byte limits apply
to extracted text, excluding JSON escaping and presentation headings. Truncation
never splits a UTF-8 character. When a page is clipped, `next_page` points to
that same page: increase the byte budget to read it fully. Otherwise it points
to the next unread page, or is null at the end of the document.

Pages with no extractable text have an empty text field; readable output adds
an explicit notice. Scanned PDFs need OCR, which this tool does not implement.
Reading order, spacing, tables, unusual fonts, and complex layouts depend on the
PDF encoding and lopdf's extraction support. This is text extraction, not a
layout-preserving converter. Unsupported content fails explicitly when the
parser reports an error; an empty result does not prove a page is blank.

Inputs are limited to 16 MiB and 10,000 pages. Parser decompression is limited
to 8 MiB, and page-content/font-map decompression during extraction to 2 MiB.
Encrypted documents are rejected. SAT does not meter fuel; these input,
output, page-count and decompression limits still apply.
Errors omit input paths and parser internals.

Builds need the repository Rust toolchain and `wasm32-wasip2`; no WASI SDK is
needed. `package.sh zip` produces a ZIP with the same three package files.
The existing `pdftools` component continues to handle image conversion and merging.

```sh
python3 external_tools/pdf-text/test.py --skip-build
```

The integration driver runs on native Python because it launches SAT processes;
its default SAT executable is the global logging wrapper. Tests exercise the
real WASI component with generated multi-page, compressed, Unicode, empty,
oversized, and malformed inputs, plus denied reads and symlink/registry isolation.

## Upstream comparison

This is a custom lopdf CLI, not a MarkItDown-compatible port. The pinned
[MarkItDown comparison](MARKITDOWN_BENCHMARK.md) reruns upstream PDF conversion
assertions against both engines and records local CLI timings. In the initial
run, native MarkItDown passed all 26 matched tests; SAT passed 9 and failed 17.
The original security/integration checks cover different guarantees.
Use the documented benchmark's `--require-parity` option to gate on conformance.
