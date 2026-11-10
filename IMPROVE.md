# Reducing token spend on software engineering tasks

These are proposed improvements, not implemented features or demonstrated
benchmark gains. The goal is to reduce token spend while preserving or
improving task resolution on workloads such as SWE-bench Lite.

SWE-bench Lite contains 300 tasks across 11 Python repositories, making
Python-focused navigation and test diagnostics useful early candidates.
See the [official SWE-bench Lite overview](https://www.swebench.com/lite.html).

## Existing foundation

SAT already provides:

- Ripgrep-based text search with bounded text or JSON output.
- Tree-sitter code retrieval: outlines, symbol reads, and text searches expanded
  to deduplicated enclosing functions.
- jaq-based JSON filtering and transformation.

These tools can help agents locate relevant code and read useful units of
source. Their effect on solve rate and token cost still needs measurement.

## Candidate improvements

| Priority | Capability | Expected token savings |
| --- | --- | --- |
| 1 | Test-failure extraction | Reduce long test logs to failing test names, assertions, exception messages, and relevant repository stack frames. Keep complete logs available separately. |
| 2 | Batched code retrieval | Search several terms together, deduplicate definitions, and return a bounded context bundle rather than repeatedly reading the same functions. |
| 3 | File discovery and repository map | Find likely implementation files, tests, and configuration without dumping directory trees or searching every file's contents. |
| 4 | Ranked lexical retrieval | Rank functions using issue text, identifiers, filenames, and docstrings when the agent does not know an exact search term. |
| 5 | References and inheritance lookup | Return likely callers, imports, subclasses, and overrides to reduce manual navigation. |
| 6 | Checked patch application and focused review | Apply small edits with expected-content checks, then return changed functions and a concise diff rather than entire files. |

### Test-failure extraction

This is the first tool to prioritize. Failed test runs can produce large
amounts of repeated traceback and captured output. A useful result might be:

```text
2 failed, 143 passed

FAILED tests/test_auth.py::test_refresh
AssertionError: expected expired token to be replaced

Repository frames:
  package/auth.py:84 — refresh()
  package/client.py:121 — request()

Full output: artifact:test-run-7
```

The extractor should preserve assertion differences, exception chains,
collection errors, and the process exit status. Taking only the last few lines
can hide the actual failure. Reduction should be deterministic and retain
links or offsets into the complete output so the agent can request more detail.

The test process would continue running in the benchmark's existing execution
environment. A WASI component would analyze its captured results; this does
not require porting arbitrary project test environments into WASI.

### Batched code retrieval

Extend the existing `code` component to accept several queries in one request.
For example, search `refresh_token`, `expired`, and `Authorization`, then return
each relevant function once.

Support progressive exploration:

```text
candidate files → signatures → selected definitions → related tests
```

Whole-function retrieval can increase token use when functions are enormous
or many definitions match. Provide an explicit context budget, previews, and
expansion controls. Distinguish complete definitions from excerpts and
size-limit results. Preserve source ranges and file hashes.

### File discovery and repository map

Provide bounded path search and a compact repository overview, with sensible
ignore rules. Help the agent identify source directories, test directories,
configuration, and likely files without repeatedly listing the entire tree.

Reuse the existing fuzzy matcher where appropriate. File discovery should
provide candidate paths to the retrieval tools without returning full file
contents by default.

### Ranked lexical retrieval

Use issue text and repository-local evidence to rank candidate definitions.
Start with lexical ranking, such as BM25 over extracted functions, filenames,
identifiers, and docstrings. This can support discovery before exact search
terms are known.

Keep any index reproducible and invalidate changed entries by file-content
hash. Compare its value against ordinary text search before adding embeddings.

### References and inheritance lookup

Start with syntax-based candidates: imports, identifier occurrences, calls,
base classes, and overrides. Include the evidence for each relationship.
Python's dynamic behavior makes it important to distinguish candidates from
compiler- or language-server-resolved references.

Defer full language-server indexing and comprehensive call graphs until the
simpler retrieval workflow demonstrates a need for them.

### Checked patch application and focused review

Support small, explicit edits with expected-content or file-hash checks to
detect stale source. Reject ambiguous replacements and provide a concise
review showing the diff and affected definitions.

Enforce write authority in the host and grant only the required files. Build
on existing patch conventions rather than recreating Git or introducing a
broad shell interface.

## Shared foundation: artifacts and pipelines

Host-managed artifacts and pipelines would make the existing tools more
valuable. A proposed workflow is:

```text
test output → stored artifact → failure extraction
            → code retrieval → bounded report
```

Keep raw logs and intermediate search results outside the model context.
Return selected evidence and artifact handles, with explicit operations to
expand a result or inspect the original data.

The host should orchestrate composition and enforce capabilities. Components
should receive access only to artifacts and source files explicitly passed to
them. Skills can encourage efficient workflows, but must not serve as the
permission boundary.

## Initial sequence

1. Add host-managed artifact handling and test-failure extraction.
2. Add batched, budgeted code retrieval that can consume extracted evidence.
3. Improve file discovery and measure ranked lexical retrieval.
4. Add references or checked editing when task evidence justifies them.

Initially postpone embeddings, full call graphs, and another general-purpose
interpreter. Prefer improvements that strengthen safety, portability,
discoverability, token efficiency, composition, or reproducibility.

## Evaluation

Use a fixed task set, model, and agent configuration. Compare the baseline
against incremental additions so their contributions can be distinguished.

Track:

- Resolved tasks and regressions.
- Total input and output tokens.
- Tool-call rounds.
- Repeated source reads and duplicate definitions returned.
- Test-log volume returned to the model.
- Wall-clock time and tool failures.

Keep complete logs outside the model context for inspection. Optimize for
lower token spend at equal or better solve rate, rather than tool count or
shorter output alone.
