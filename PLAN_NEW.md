# Python environments with SAT

## Build flow

`sat python-venv install <requirement>... --name <tool>` downloads Python wheels
from a named registry, assembles a temporary Python import tree, and asks the
pinned `componentize-py` tool to produce a SAT-compatible Python component. The
result is validated and registered through SAT's existing local registry.

Package resolution and wheel download happen on the trusted host. The initial
implementation accepts package specifiers only and supports universal
`py3-none-any` wheels; direct URLs, pip options, source distributions and native
extensions are rejected/not supported. Credentials stay in the host process and
are never included in the guest environment.

`componentize-py` is itself a host executable in this design. It pre-initializes
the Python component by running the Python guest under Wasmtime. Only the staged
application and staged package site-packages are mapped into the guest. Imports
and Python `.pth` startup hooks execute in that guest, with no network or host
credentials. The final component file is written by the trusted componentizer
outside those mounted paths. The staging directory is unique, initially empty,
configurable via `SAT_BUILD_TMP`, and removed on both success and failure.

SAT checks the generated package with its existing component parser/runtime
loader before registration. This validation is still a host-side Wasmtime parse
and compilation boundary; generated code remains untrusted when installed or
run. Runtime filesystem and HTTP capabilities remain governed by SAT's existing
policy and per-invocation grants. Completed components are cached using the tool
name, sorted requirement strings, registry identity and downloaded wheel hashes.
SAT still downloads/resolves wheels before checking the build cache.

## Registry configuration

Store indexes in `SAT_HOME/python-registries.toml`:

```toml
[registries.pypi]
index_url = "https://pypi.org/simple"

[registries.company]
index_url_env = "SAT_PYPI_INDEX_URL"
```

An `index_url_env` points to an environment variable containing the complete
HTTPS URL, so credentials need not be stored in the file. Registry data and
credentials are used only for host-side resolution/download. Custom index
operators are trusted to serve package files referenced by their index pages.

## Limits and security properties

- Wheel contents are installed without running package code on the host.
- Import-time code and `.pth` hooks are run while componentize-py initializes the
  guest under Wasmtime. The guest sees only the temporary stage paths.
- No PEP 517 build backend is run in this initial version. Supporting source
  packages requires a separate in-WASI build frontend and toolchain.
- Package code can still exploit a Wasmtime/SAT vulnerability or exhaust resources
  inside its guest. Componentization has a configurable wall-clock process timeout
  (`SAT_PYTHON_BUILD_TIMEOUT_SECS`, default 600 seconds), but memory and disk
  quotas are not implemented yet.
- SAT validates the resulting component with the same loader used for packages;
  runtime Wasmtime parsing remains part of the trusted host surface.
- LiteLLM-style `.pth` startup payloads do not receive registry credentials,
  ambient host filesystem access, or network access during initialization.

## CLI and setup

Run `python3 python-example/build.py --no-install` once to provision the pinned
componentize-py toolchain, then configure the index and install:

```sh
sat python-venv install 'requests==2.32.3' --name python_requests
sat python_requests -c 'import requests; print(requests.__version__)'
```

Creating and testing a dedicated adversarial package remains a separate manual
step. That test should attempt writes outside the stage and network access from a
`.pth` or imported module, and confirm only in-stage effects succeed.
