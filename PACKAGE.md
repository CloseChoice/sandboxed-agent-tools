# Python package compatibility shortlist

SAT's current `python-venv` builder downloads wheels tagged `py3-none-any` only.
It does not build source distributions or install platform-specific wheels. It
then imports each detected top-level package module inside the WASI build guest
so the modules are included in the generated component.

This requirement applies to the **entire resolved dependency tree**. The
requested package and every direct or transitive dependency must have a
compatible universal wheel. One dependency with only a Linux/macOS/Windows wheel
or only a source archive makes the install fail, even when the requested package
itself is pure Python.

The packages below are realistic first candidates because their distributions
and normal transitive dependencies are published as universal Python wheels. They
are **candidates, not yet verified SAT compatibility claims**: a package can
still fail if an import requires a Python standard-library module unavailable
in the WASI build, has unsupported WASI behavior, or has an optional dependency
that is not included.

## Good first candidates

| Package | Useful for | Why it looks compatible |
| --- | --- | --- |
| [`beautifulsoup4`](https://pypi.org/project/beautifulsoup4/) | HTML/XML parsing with Python's built-in `html.parser` | The package and default dependency `soupsieve` publish universal wheels. Avoid optional parser backends such as `lxml`, which needs native code. |
| [`rich`](https://pypi.org/project/rich/) | Terminal formatting, tables, syntax highlighting | Rich and its standard dependencies (`markdown-it-py`, `pygments`, `mdurl`) publish universal wheels. Terminal behavior may differ in a WASI process. |
| [`markdown-it-py`](https://pypi.org/project/markdown-it-py/) | CommonMark and Markdown parsing/rendering | Pure Python package with universal wheels; its normal dependency `mdurl` is also pure Python. |
| [`python-dateutil`](https://pypi.org/project/python-dateutil/) | Date parsing and timezone utilities | Pure Python package; its `six` dependency is also available as a universal wheel. |
| [`click`](https://pypi.org/project/click/) | Command-line argument parsing | Pure Python and broadly used. SAT supplies its own command-line interface, so Click is useful for Python code that builds a CLI internally rather than replacing SAT's CLI. |
| [`python-dotenv`](https://pypi.org/project/python-dotenv/) | Reading `.env`-style configuration files | Pure Python and distributed as a universal wheel. File access is still restricted by SAT's runtime grants. |

For example, a first compatibility trial could be:

```sh
sat python-venv install 'requests==2.34.2' --name python_requests
sat python_requests -c 'import requests; print(requests.__version__)'
```

The version is illustrative; choose a version available from the configured
registry and pin it for a reproducible environment. A successful install means
the selected wheel dependency tree resolved and componentized. It does not
certify the package as safe or every API as supported.

## Wheel-compatible, with current runtime limitations

[`requests`](https://pypi.org/project/requests/) and
[`httpx`](https://pypi.org/project/httpx/) have universal wheels, as do their
usual dependencies. However, HTTPS support through `requests` currently fails:
the generated component includes imported Python modules but does not currently
preserve package data files such as `certifi/cacert.pem`. Also, SAT's generated
Python component does not import SAT's `wasi:http` interface, and the runtime
denies raw TCP/UDP sockets to Python. These packages can be imported, but
outbound requests are not currently a supported use case. The missing CA bundle
is a separate issue from the network capability.

SAT's host can grant policy-limited HTTP to components that import
`wasi:http/outgoing-handler`; SAT checks each request against the package's
allowed HTTP endpoints and the invocation's `--allow-http` grant. The generated
Python component does not currently use that interface. Supporting Python HTTP
clients would require a bridge to the host-mediated interface (or a separate,
carefully scoped socket capability); adding an allowlist to the current Python
policy alone would not make `requests.get()` work.

Other packages can likewise depend on non-Python resources (certificate
bundles, templates, schemas, timezone databases, and similar files). Universal
wheel tags alone do not guarantee those resources will be available in the
component at runtime.

## Scientific and mathematical candidates

These are more realistic than NumPy for the current pure-Python-only path:

| Package | Useful for | Current assessment |
| --- | --- | --- |
| [`mpmath`](https://pypi.org/project/mpmath/) | Arbitrary-precision arithmetic and special functions | Strong candidate: publishes a universal wheel and has no required native extension. The project reports testing through CPython 3.14. |
| [`uncertainties`](https://pypi.org/project/uncertainties/) | Propagating measurement uncertainty through calculations | Strong candidate: universal wheel and no required third-party runtime dependency. Its NumPy integration requires NumPy and is unavailable in this MVP. |
| [`SymPy`](https://pypi.org/project/sympy/) | Symbolic algebra, calculus, equation solving | Candidate: universal wheel; its hard dependency, mpmath, also has a universal wheel. SymPy is large and has a broad, sometimes lazy import graph, so componentization may need further import coverage. |
| [`asteval`](https://pypi.org/project/asteval/) | Evaluating mathematical expressions in a Python-like syntax | Candidate: small universal wheel; try its core evaluator without optional scientific dependencies. It is a library feature, not a security boundary. |
| [`NetworkX`](https://pypi.org/project/networkx/) | Graph algorithms and network analysis | Not currently usable: its package import pulls in Python's `bz2` module, but this WASI Python build does not include the `_bz2` extension. The wheel downloads and installs, then componentization fails. |

For example, these are reasonable packages to try one at a time:

```sh
sat python-venv install 'mpmath==1.4.1' --name python_mpmath
sat python-venv install 'uncertainties==3.2.3' --name python_uncertainties
sat python-venv install 'sympy==1.14.0' --name python_sympy
```

These versions have universal wheels on PyPI, but they have **not been
validated as working SAT components**. The installed components use CPython
3.14 under WASI, and the data-file and import-coverage limitations above still
apply. NumPy-based ecosystems such as SciPy, pandas, scikit-learn, and
statsmodels remain unavailable through this installer because their required
dependency trees include native wheels.

## Not available through the current installer

These popular packages rely on compiled extensions or platform-specific wheels,
so the current `py3-none-any`-only resolver will not select their normal PyPI
artifacts:

- Scientific/data packages such as NumPy, pandas, SciPy, and PyTorch.
- Packages such as Pillow, cryptography, lxml, or orjson.
- Pydantic's `pydantic-core` dependency, which contains a compiled extension.

WASI-specific wheels are not accepted by the current resolver yet either. SAT
would need a WASI wheel tag/ABI selection path before those artifacts can be
installed. A pure-Python wrapper around an unavailable native dependency does
not make the dependency installable.

## How to evaluate another package

1. Check that the requested version and its full dependency tree have compatible
   `py3-none-any` wheels and support the embedded Python version (3.14).
2. Try `sat python-venv install '<package>==<version>' --name <tool_name>`.
   Wheel-only resolution rejects a missing compatible wheel without falling
   back to running a source build.
3. Run a small import and API call with the generated SAT tool.
4. Keep runtime filesystem, HTTP, and other capabilities limited to what the
   package actually needs.

During componentization, SAT imports detected top-level modules and executes
Python startup hooks from the wheels inside the WASI build guest. That catches
many missing-import and initialization problems during the build, but it does
not test every lazy import path or runtime operation.
