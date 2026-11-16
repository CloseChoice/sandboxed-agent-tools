# Original upstream Tesseract tests

The complete Tesseract 5.5.4 source and tests are a Git submodule at
`external_tools/tesseract/upstream/tesseract`, pinned to
`7e548e6eff172f38f97664afe7f7ce40db36c8d6`. Its nested submodules supply the
original test images/data and GoogleTest:

| Input | Pinned revision |
| --- | --- |
| Tesseract | `7e548e6eff172f38f97664afe7f7ce40db36c8d6` |
| `tesseract-ocr/test` | `232ff181c66516116ec0e84c4963f70de15050fd` |
| GoogleTest | `7d76a231b0e29caf86e68d1df858308cd53b2a66` |
| Fonts/model assets used by upstream CI (`egorpugin/tessdata`) | `fce038c79cbab4c45bac56b6f8d4e6bc6a9d9cfd` |

```sh
git submodule update --init --recursive external_tools/tesseract/upstream/tesseract
```

Use that command after cloning and after checking out a new parent revision.
Do not use `--remote` for reproducible comparisons. Gitlinks pin the source and
nested submodules; the test-asset revision is pinned by the runner.

## Full native baseline

```sh
python3 external_tools/tesseract/upstream_suite.py --init
```

The runner inventories all **83** original `unittest/*.cc` files, builds the
full native engine with legacy OCR, training, Pango and ICU enabled, and requires
all 83 test executables to appear in CTest. It runs them serially because the
original helpers share temporary filenames. Each executable can contain many
GoogleTest cases. Counts in reports mean test cases, not source files or the
number of individual EXPECT/ASSERT statements.

On Ubuntu, install CMake, Ninja, a C++20 compiler, pkg-config, libpng-dev,
libjpeg-dev, libtiff-dev, libicu-dev and libpango1.0-dev. Python 3.12+ handles
trusted build orchestration. The locked Leptonica source is fetched automatically
if absent; no system Tesseract/Leptonica installation is needed. Model, font and
language fixtures are fetched at the upstream CI asset revision.

Inputs live in the pinned submodules. A disposable source snapshot under
`target/upstream-suite/source` receives the upstream CI fonts and CMake's expected
language-data layout. No tracked original test source or assertion is edited;
hashes are recorded. The second CMake configure pass makes Pango detection visible
to the upstream test-directory configuration. All model families (`tessdata`,
`tessdata_best`, `tessdata_fast`) are prepared before execution.

Default wall timeout is 600 seconds **per executable**, configurable with
`--timeout`. That is a test-runner timeout, not SAT fuel metering. Output limits
keep failing training tests from producing unbounded JUnit/report payloads.
Skips, disabled cases, failures, crashes and missing XML reports must be read
separately; a successful test executable is not proof that every case ran.
Upstream's `DISABLED_` cases are present in the checkout and reported separately.
Use `--include-disabled` to opt into those cases; they can require more models or
features and are not part of the standard upstream baseline. The runner clears
an inherited `GTEST_FILTER` so it cannot silently narrow a full run.

Results: `target/upstream-suite/summary.json`, `inventory.json`, `ctest.xml`,
`gtest/*.xml`, `build.log`, and `ctest.log`. The output directory can be changed
with `--output`. Generated reports for a repeated run replace previous reports;
use a separate output directory to retain distinct runs. The full native suite
tests upstream's broader feature set; it is not a WASI component pass count.

## Original tests inside WASI

```sh
export WASI_SDK_PATH=/absolute/wasi-sdk-34.0-x86_64-linux
bash external_tools/tesseract/package.sh tar
python3 external_tools/tesseract/upstream_wasi.py --sat sat/target/release/sat
```

This compiles **30 original core test files** into separate test-only WASI
components, linked to the same `libtesseract.a` and Leptonica build used by the
production OCR component. GoogleTest sources and test assertions are unchanged.
The original upstream file helper is linked as test support; its visibility
annotations are empty for static linking. The compiler includes the production
generated configuration to keep engine types/layout consistent.

The harness exports the test entrypoint, preserves static test registrations with
whole-archive linking, sets an absolute temporary-directory pointer in the
original helper, and disables debug-image writes. It supplies only the test-data
read grant and controlled XML/temp write grants. Empty test runs are rejected.
Test components are registered in an isolated registry under `target/upstream-wasi`,
not installed as normal tools in the user's registry. Local SAT calls use the
installed Codex logging wrapper; CI records commands and diagnostics per test.

The old pinned GoogleTest is configured for single-threaded WASI, without C++
exceptions, death tests or stream redirection. These configuration differences
are part of the coverage limitation, not equivalent replacement assertions.

Every upstream source file remains in the inventory. Exclusions have reasons in
the WASI report: legacy engine, training/ICU, Pango/fonts, image/model/lifecycle or
thread APIs needing additional adapters, extra Unicode test sources, and two
files whose original temporary-directory helper uses C++ try/catch. In particular,
`recodebeam_test` needs ICU training helpers, while `lstm_nullchar_test` and
`lstm_recoder_test` need exceptions. `--include-exception-tests` reproduces the
latter build blockers. `--only rect_test` selects a smaller core run.

Results: `target/upstream-wasi/summary.json`, `xml/*.xml`, and per-file logs. A
missing XML report, zero registered tests, compilation failure, timeout, runtime
trap or failed assertion cannot count as a pass. If the full native report is
present, the runner pairs matching test-case IDs across native and WASI results.
For runs started in parallel, refresh just the pairing after native completion:

```sh
python3 external_tools/tesseract/upstream_wasi.py --sat sat/target/release/sat --compare-only
```

This is original core unit-test coverage of the engine, not full upstream CLI
compatibility. The separate `test.py` suite continues to check the shipped CLI
and sandbox boundaries.

## CI and recorded results

Local Linux run on 2026-11-14:

| Backend | Original executables | Passed cases | Skipped | Upstream-disabled |
| --- | ---: | ---: | ---: | ---: |
| Full native baseline | 83 | 423 | 9 | 494 |
| WASI core subset | 30 | 144 | 9 | 0 |

All 144 executed WASI passes also pass natively with identical original test-source
hashes. Three other paired cases pass natively but skip on WASI; six skip on both.
The WASI report identifies all 53 source files outside this core pass and their
reasons. A full native pass is not a full production-WASI pass.

`.github/workflows/tesseract-upstream.yml` is a manual workflow with independent
native-full and WASI-core jobs. It initializes the recursive submodules and uploads
both raw and summarized results even on failure. It does not depend on the separate
release workflow's version-check script.

`UPSTREAM_NATIVE_RESULTS.json` and `UPSTREAM_WASI_RESULTS.json` are local result
snapshots. `WASI_DISCOVERY_RESULTS.json` and `WASI_RECOVERY_RESULTS.json` retain
the initial porting-discovery failures so unsupported dependencies are visible.
The initial native attempt had incomplete fixture preparation; its bounded
diagnostics and partial reports are retained under
`target/upstream-suite/attempts/incomplete-fixtures`, rather than counted as the
corrected baseline. The repeated encoding log from that invalid attempt was
reduced to a bounded sample with its original byte count recorded.

The CLI smoke benchmark remains in `TEST_RESULTS.json`. Unit-test runtime mostly
measures harness/startup cost for tiny tests; use matching OCR workloads/models
for useful native-vs-WASI performance comparisons.
