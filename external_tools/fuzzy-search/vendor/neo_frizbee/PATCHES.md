# Local patch to neo_frizbee 0.11.0

Source: https://static.crates.io/crates/neo_frizbee/neo_frizbee-0.11.0.crate

Archive SHA-256: `7a2f6120a8da26bea3587731072111062c5d8c51ca3a3a75a716bd8b735d5882`

Upstream VCS commit: `7c189f518dbf01aaba5cddcf5bdf7a801784983c`.
All upstream sources and the MIT license are retained.

Only `src/prefilter/algo/unicode.rs` is modified. The original zero-typo
Unicode loop reuses a byte window when consecutive query characters change
UTF-8 width. For example, `café` incorrectly fails to match `📁/café.rs`,
on both native and Wasm backends.

The replacement scans Unicode scalar boundaries, accepting the original or
upstream's same-width case-flipped character. It returns the first possible
start and last possible end so the existing Smith–Waterman scorer can still
choose an alignment. This also avoids the original filter's case-flipping
mask issue. The old reverse-scan helper remains, marked as unused.
Scoring and typo-budget 1/2 code are unchanged. This scalar filter may be
slower than a repaired SIMD filter; correctness is the first milestone.

Regression tests in the wrapper cover mixed character widths, case, emoji
prefixes, and chunk-boundary positions. Separately, the wrapper interprets
0.11.0's alignment indices as **bytes**, matching upstream's Unicode alignment
tests; the upstream public `MatchIndices` docstring still calls them characters.
