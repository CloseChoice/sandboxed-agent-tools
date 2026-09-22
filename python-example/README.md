# Sandboxed Python for SAT

A CPython 3.14.0 WASI component built with Bytecode Alliance's
[`componentize-py`](https://github.com/bytecodealliance/componentize-py) 0.25.0.
Python code executes inside Wasmtime, not the host's Python interpreter.

From the repository root:

```sh
python3 python-example/build.py
export SAT_HOME="$PWD/sat/target/demo/registry"
export PATH="$PWD/sat/target/debug:$PATH"

sat python --help
sat python -c 'print(1 + 2)'
sat python script.py --read script.py
sat python -c 'import math; print(math.sqrt(81))'
sat python -c 'import json; print(json.dumps({"hello": "wasi"}))'
```

The builder uses a project-local virtual environment and needs Python with `venv`
and pip, plus Rust. Only the build step needs native Python. It registers in the
existing demo registry unless `SAT_HOME` is set. Re-running the builder explicitly
replaces the installed `python` package. `--no-install` only builds/packages it.
The first Wasmtime compilation can take a few minutes with a debug SAT build;
subsequent invocations reuse compiled code from `SAT_HOME/.cache`.

The WIT export is `run(code: option<string>, script: option<string>) -> result<string, string>`. The embedded
`[options.code]` metadata adds `short = "c"`; SAT's generic CLI maps `-c` to `code`.
Python stdout/stderr are captured into the returned string. Exceptions produce
an error/nonzero exit, with a traceback. `--json` returns captured output as a JSON
string. Supply exactly one of `-c CODE` or a positional script path; SAT rejects
both/neither before execution. The script needs an explicit read grant. Script mode
sets `__name__ = "__main__"`, `__file__`, and `sys.argv = [script]`, and honors Python
source encoding declarations. Extra script arguments and an interactive REPL are
not implemented. The same import/package limitations below apply in both modes.

Filesystem access is denied by default; grant existing paths explicitly:

```sh
sat python --read input.txt -c 'print(open("input.txt").read())'
sat python --read .. --read-write . -- -c 'open("result.txt", "w").write("hello")'
```

A file grant exposes only that file; a directory grant includes its descendants.
Repeat flags for multiple paths. Optional `HOST::GUEST` mappings rename a grant,
e.g. `sat python alias.py --read script.py::alias.py`; the alias need not exist on disk. Relative filenames work from the invocation's
working-directory relationship, including `..`. A writable directory permits
creating files; a single-file grant requires an existing file and does not permit
rename/delete. See [SAT filesystem grants](../sat/README.md#standard-filesystem-grants).

No host environment or network access is inherited. SAT supplies a synthetic
`PWD` when paths are granted. The package policy continues to deny HTTP. `exec` and
normal Python builtins remain available; the sandbox boundary is Wasmtime and
the host's capability configuration, not Python monkey-patching. Output capture
is only presentation. Each invocation gets a new component instance.

## Imports and packages

This minimal build preloads: `collections`, `datetime`, `decimal`, `fractions`,
`functools`, `itertools`, `json`, `math`, `os`, `pathlib`, `random`, `re`, `socket`,
`statistics`, and `sys`, plus the wrapper's dependencies. Modules can be imported
by submitted code after preloading, but OS operations still require capabilities.
Not every standard-library module is included. `componentize-py` snapshots imports
at build time; there is no runtime pip or arbitrary PyPI compatibility here.

Later, build separate registered components with the same WIT interface:

```sh
sat python_numpy -c 'import numpy; print(numpy.arange(5))'
```

That command is a proposed future environment, not an included tool. To create it:

1. Resolve pinned packages in a trusted external builder.
2. Supply pure-Python dependencies or native extensions actually built for the
   chosen WASI CPython ABI; host Linux/macOS/Windows wheels will not work.
3. Import the supported packages during componentization so they are included.
4. Embed metadata with a distinct name such as `python_numpy`, package policy,
   and install the resulting self-contained component through SAT.

Cache future environments by a normalized specification: interpreter and builder
versions, WASI target, sorted pinned dependencies/artifact hashes, and wrapper
source. Equivalent specs reuse builds; changed specs produce a new component.
This builder already caches its exact builder/source specification and verifies
its cached Wasm digest. SAT separately verifies the final installed package hash.

## Checks

```sh
python3 python-example/test.py
```

Checks code/script execution, input selection, source encoding, arithmetic/imports, CLI help, exceptions, file/directory grants, sibling and symlink isolation, read-only enforcement, environment
isolation, and inability to connect to a local listener. No external website is
needed. The build step may download the pinned builder and Rust dependencies.
