# uutils

Imported from `feat/sat-component-mvp` into the external tool layout.

```sh
bash external_tools/uutils/package.sh  # tar (default) or zip
sat register external_tools/uutils/dist/uutils.tar
printf 'b\na\n' | sat uutils --stdin sort
```

Defaults grant no filesystem or network access. Host permissions go after `--`.
Use absolute paths; the old `HOST::GUEST` mappings are not supported by this host.

Run `python3 external_tools/uutils/test.py` to build and verify; use
`--skip-build` to test an existing archive.

`uutils` embeds upstream Rust coreutils: head, tail, cat, wc, cut, sort, uniq,
nl, ls, basename, and dirname. Arguments after the utility name are passed as a
WIT string list. Host permissions still go after SAT's final `--` separator.

Use `--input` or `--stdin` before the utility name for input. No ambient host
stdin is inherited. The host captures guest stdout (16 MiB maximum) and stderr
(1 MiB), checks the returned exit status, and emits the captured output. This
stdio adapter is specific to the upstream coreutils interface; other tools
return their results through WIT. `--json` before the utility name encodes the
captured UTF-8 output as a JSON string. Unsupported commands fail explicitly.

All execution is bounded by the host fuel ceiling. Package metadata can lower
the budget but cannot increase it. No host environment or network authority is
inherited.
