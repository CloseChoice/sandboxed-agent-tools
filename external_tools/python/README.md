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


## HTTP and HTTPS

The component includes a `urllib.request` transport backed by WASI HTTP.
Requests use the host's HTTP/TLS implementation and hostname policy, without
granting Python raw sockets or host networking.

```sh
sat python -c 'import urllib.request
with urllib.request.urlopen("https://example.com", timeout=10) as response:
    print(response.status)
    print(response.read(200).decode())
' -- --allow-http example.com
```

Registration defaults deny HTTP. `--allow-http example.com` permits that
hostname for this invocation. Every redirect is checked again by the host;
a redirect to an ungranted hostname fails. Requests do not need filesystem grants.

GET, HEAD, byte-body requests such as POST, headers, HTTP errors, and
`urlopen(..., timeout=seconds)` are supported. Request and response bodies
are buffered up to 8 MiB by the adapter. Streaming uploads/downloads, proxies,
custom Python TLS contexts, and socket-based third-party clients such as
requests/httpx are outside this profile. HTTPS certificate validation stays
in the host transport; Python does not need an SSL extension for this adapter.

Run the deterministic local-server checks, including denied hosts and redirects:

```sh
python3 external_tools/python/test_http.py --sat sat/target/release/sat
```
