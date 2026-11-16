# Tesseract OCR for SAT

Tesseract **5.5.4** and Leptonica **1.87.0**, compiled into one WASI component.
The pinned `tessdata_fast` English LSTM model is embedded. No Python, host OCR
executable, shell, network access or external model directory is needed at runtime.
This is an image OCR profile with a Tesseract-style CLI subset, not the full
upstream command-line program.

```sh
sat tesseract /absolute/scan.png stdout --psm 6 -- --read /absolute/scan.png
sat tesseract /absolute/scan.jpg stdout tsv -- --read /absolute/scan.jpg
sat tesseract /absolute/scan.png /absolute/output-directory/result hocr -- \
  --read /absolute/scan.png --write /absolute/output-directory
```

Output basenames receive `.txt`, `.tsv`
or `.hocr`; `stdout`/`-` writes directly to stdout. Use the SAT skill's logging
wrapper for Codex invocations. All default grants are empty.

Inputs: PNG, JPEG, PNM (including PGM/PPM) and BMP, or `stdin`/`-` for image bytes.
Images are decoded with pure Rust `image` codecs and converted to grayscale;
transparency is composited onto white. Tesseract and Leptonica do the recognition.
Native PNG/JPEG codecs are excluded to avoid setjmp/longjmp and Wasm exception
handling requirements. Leptonica uses zlib **1.4** internally; Rust PNG decoding
uses its own locked compression dependencies.

Supported options: `-l`, `--psm`, `--oem 1`, `--model`, help/version, and one
output config (`txt`, `tsv`, `hocr`). TSV includes the standard column header;
hOCR is an HTML document. Page segmentation modes 3–11 and 13 are supported.
Mode 6 is useful for a single text block; 7 for a single line.

Only English is embedded. For another language, pass `-l deu --model
/absolute/deu.traineddata` and explicitly grant that model file read access.
Use compatible upstream LSTM traineddata. Multilingual model combinations,
OSD, legacy engines, external config files, `-c` variables, training, searchable
PDF output, TIFF, multipage documents, remote URLs and PDF rasterization are
outside this profile. PDFs must be rasterized separately; embedded-text PDF
extraction and OCR are distinct operations.

## Boundaries and limits

The host enforces file grants. Ordinary stdout recognition needs only the input
file's read grant. An embedded model needs no filesystem grant. Explicit models
need their own read grant. Writing a new output needs a parent-directory write
grant; overwriting an existing output can use an exact-file write grant. Write
only to a controlled output location. The API's ancillary-file reader refuses
all requests, Leptonica debug image writes are disabled, and unsupported shell
and temporary-file helpers fail with `ENOSYS` in WASI.

Encoded image and external model: at most 32 MiB each. Image: at most 12,000 per
dimension and 20 million pixels. Decoder allocation budget: 128 MiB, separate
from grayscale buffers and OCR engine/model memory. OCR output: at most 16 MiB
(checked after the API produces it). These are not a total process-memory limit.
SAT fuel remains unlimited. Runtime errors return nonzero and diagnostics go to
stderr; successful OCR content goes to stdout or the selected output file.
Leptonica warning/info chatter is suppressed; errors remain visible.

## Build and test

Requires Rust with `wasm32-wasip2`, WASI SDK **34**, CMake, Ninja and Python 3.12+
on the trusted builder. Runtime execution requires only SAT.

```sh
export WASI_SDK_PATH=/absolute/wasi-sdk-34.0-x86_64-linux
bash external_tools/tesseract/package.sh tar
sat register external_tools/tesseract/dist/tesseract.tar
python3 external_tools/tesseract/build.py --native
python3 external_tools/tesseract/test.py --sat sat/target/release/sat \
  --output external_tools/tesseract/TEST_RESULTS.json
```

`sources.json` pins all C/C++ archives and the English model by URL and SHA-256.
`Cargo.lock` locks Rust transitive dependencies. The builder verifies source
downloads and cached archives on each invocation. CMake/Cargo handle incremental
compilation. Tesseract recognition sources are unmodified; optional services,
legacy engine, debug fonts, native SIMD and OpenMP are disabled in the WASI build.
Full dependency notices are retained in the repository and embedded in a custom
section of the distributed component. The standard SAT TAR/ZIP package has no
sidecar runtime dependency.

The integration suite checks recognition against the original upstream phototest
transcription, native/WASI text and structured output, four input formats plus
alpha PNG, stdin, explicit models, output files, limits and denied grants.
`fixtures/manifest.json` identifies upstream originals and derived images by
hash. The matching native upstream CLI is checked independently on PGM input.
TSV confidence values allow 0.1 tolerance for native/WASI numeric differences;
text and geometry must match. hOCR comparisons check recognized text, not byte
identical HTML formatting. This is our CLI integration suite. The [original upstream test guide](UPSTREAM_TESTS.md)
adds the complete pinned source/test submodules, a full native baseline, and
original core tests executed in test-only WASI components.

`TEST_RESULTS.json` records checks and a small same-source/model native-vs-WASI
CLI timing comparison, including process, host and local logging overhead. It is
a smoke benchmark, not a broad OCR accuracy/performance study.

Upstream: [Tesseract](https://github.com/tesseract-ocr/tesseract),
[Leptonica](https://github.com/DanBloomberg/leptonica),
[English model](https://github.com/tesseract-ocr/tessdata_fast),
[original test fixtures](https://github.com/tesseract-ocr/test).
