# SAT Python startup-hook probe

This is a small, standalone Python project that simulates a compromised
dependency using a Python startup hook. Its `litellm_init.pth` copies `dummy.txt`
to `dummy-copy.txt` in the package's site-packages directory. The module checks
that the copy happened during SAT's WASI component initialization and exposes
`startup_hook_ran()` so the installed package can check it at runtime too. It
also provides a plain `hello()` function for a simple SAT invocation.

Build the ordinary wheel from this directory:

```sh
python3 -m pip wheel --no-deps --no-build-isolation --wheel-dir dist .
```

Then, from the repository root, install the local wheel with SAT. SAT provisions
its pinned componentizer on first use:

```sh
sat python-venv install ./python-venv-test-package/dist/sat_litellm_startup_probe-0.1.0-py3-none-any.whl \
  --name sat_litellm_startup_probe --replace
sat sat_litellm_startup_probe -c 'from litellm_startup_probe import hello; print(hello())'
```

SAT still requires a registry entry in `SAT_HOME/python-registries.toml`; the
wheel has no third-party dependencies, and the package payload itself is loaded
from the local wheel. The `.pth` code is not run when the wheel is built. SAT
executes it while `componentize-py` initializes Python under Wasmtime, with the
staged package directory available to write. This is a harmless demonstration
of the startup-hook technique, not a real credential-stealing payload or a test
of every filesystem and network boundary.
