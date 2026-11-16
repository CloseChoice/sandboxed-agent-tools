# Sandboxed Python

CPython 3.14 in a WASI component, built from [app.py](app.py) and
[wit/world.wit](wit/world.wit) with pinned `componentize-py==0.25.0`.
The trusted host-side builder needs Python 3 with venv/pip and network access
on first use. It does not run submitted code on the host.

From the repository root:

```sh
bash external_tools/python/package.sh       # dist/python.tar
bash external_tools/python/package.sh zip   # dist/python.zip
sat/target/release/sat register external_tools/python/dist/python.tar
sat/target/release/sat python -c 'print(1 + 2)'
sat/target/release/sat python /absolute/script.py -- --read /absolute/script.py
```

Registration defaults grant no filesystem or network access. Use absolute
paths and explicit read/write grants after `--`. The runtime enforces these
permissions; Python output capture is only presentation. Output and tracebacks
are returned as text. Choose either `-c CODE` or a script path. Extra script
arguments and an interactive REPL are not supported.

The wrapper preloads a small standard-library profile for text, JSON, CSV, XML,
math, archives, and in-memory SQLite. XML includes ElementTree (including XInclude
and cElementTree), DOM/minidom/pulldom/DOMBuilder, SAX and Expat. `test_xml.py`
checks parsing, namespaces, Unicode, serialization, incremental parsing and
host-enforced file grants, including denied external-entity file reads. Imports are captured at build time;
untested lazy imports may be unavailable. There is no runtime pip, arbitrary
PyPI support, or native host extension loading. Custom package environments
are outside this package's current scope.

The builder caches by pinned componentizer version, host Python version, and
wrapper/WIT source content, and verifies the cached component digest. Changed
inputs or a damaged artifact trigger a rebuild. Metadata is repackaged on every
invocation. Generated files stay under ignored `target/` and `dist/` directories.

Run execution, standard-library and permission checks against an existing host:

```sh
python3 external_tools/python/test.py --sat sat/target/release/sat
```
