# ripgrep

Sandboxed text search using upstream ripgrep libraries.

```sh
bash external_tools/ripgrep/package.sh  # tar (default) or zip
sat register external_tools/ripgrep/dist/ripgrep.tar
sat ripgrep 'pattern' --path /absolute/path/to/source -- --read /absolute/path/to/source
```

Defaults grant no filesystem or network access. Host permissions go after `--`.
Use absolute paths; the old `HOST::GUEST` mappings are not supported by this host.

Run `python3 external_tools/ripgrep/test.py` to build and verify; use
`--skip-build` to test an existing archive.

`ripgrep` uses upstream grep-regex, grep-searcher, and ignore crates inside
WASI. It searches granted paths recursively, supports regular expressions,
literal strings, case-insensitive matching, gitignore filtering, hidden files,
and context lines. Pass one root per `--path`; repeat the flag for multiple roots:
`--path /project/src --path /project/tests`. Use absolute paths explicitly;
no host working directory is inherited. `--format json` returns structured
results. `--max-results` bounds returned lines, including context.

Files whose reads are denied, including hard-linked files rejected by SAT, and
entries that cannot be traversed are skipped while the search continues.
Text output reports the skipped count; JSON includes `skipped_paths`. A nonzero
count means some paths were not searched; a skipped directory can contain many
files. Entries hidden by the host policy are not included in this count. Other
search-processing errors and ignore-rule parsing errors still fail the call.

SAT runs with fuel metering disabled. Legacy package `fuel` values are
accepted for compatibility and ignored. No host environment or network authority is
inherited.

## Upstream credits and license

Built on [ripgrep](https://github.com/BurntSushi/ripgrep) by Andrew Gallant
(BurntSushi) and its contributors, using the upstream `grep-regex`,
`grep-searcher`, and `ignore` crates. These crates offer MIT or Unlicense;
SAT uses them under MIT and retains the notice in
[THIRD_PARTY_NOTICES.txt](THIRD_PARTY_NOTICES.txt).
This is SAT's search interface over those libraries, rather than the complete
upstream `rg` CLI.
