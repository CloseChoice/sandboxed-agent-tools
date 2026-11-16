# ripgrep

Imported from `feat/sat-component-mvp` into the external tool layout.

```sh
bash external_tools/ripgrep/package.sh  # tar (default) or zip
sat register external_tools/ripgrep/dist/ripgrep.tar
sat ripgrep 'pattern' --paths /absolute/path/to/source -- --read /absolute/path/to/source
```

Defaults grant no filesystem or network access. Host permissions go after `--`.
Use absolute paths; the old `HOST::GUEST` mappings are not supported by this host.

Run `python3 external_tools/ripgrep/test.py` to build and verify; use
`--skip-build` to test an existing archive.

`ripgrep` uses upstream grep-regex, grep-searcher, and ignore crates inside
WASI. It searches granted paths recursively, supports regular expressions,
literal strings, case-insensitive matching, gitignore filtering, hidden files,
and context lines. Repeated `--paths` select roots. Use absolute paths explicitly;
no host working directory is inherited. `--format json` returns structured
results. `--max-results` bounds returned lines, including context.

SAT runs with fuel metering disabled. Legacy package `fuel` values are
accepted for compatibility and ignored. No host environment or network authority is
inherited.
