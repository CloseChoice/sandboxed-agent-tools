# pdf-tools

Imported from `feat/sat-component-mvp` into the external tool layout.
Build a deterministic archive and register it with the current host:

```sh
bash external_tools/pdf-tools/package.sh  # accepts tar (default) or zip
sat register external_tools/pdf-tools/dist/pdftools.tar
sat pdftools --convert /absolute/path/to/input.png --destination /absolute/path/to/output.pdf -- --read /absolute/path/to/input.png --write /absolute/path/to/output.pdf
```

Defaults grant no filesystem or network access. Host permissions go after `--`.
Paths must be absolute; this host does not support the old `HOST::GUEST` syntax.
For PDF output, create the destination file first and explicitly grant it write access.

Run `python3 external_tools/pdf-tools/test.py` to build and verify; use
`--skip-build` to test an existing archive.

The registered tool name is `pdftools`. Choose exactly one of repeated
`--convert` image paths/directories or `--merge` a directory of PDFs. Inputs in
a directory are processed in filename order. PNG and JPEG images are supported.
PDF bytes pass through the `destination` WIT import to a host-bound file;
`--destination` selects that file but grants no write authority by itself.

Input limits are 16 MiB per file, 256 files, and four million decoded image
pixels. Existing PDF content is merged using lopdf. This tool does not provide
OCR, text extraction, or cryptographic signing.

All execution is bounded by the host fuel ceiling. Package metadata can lower
the budget but cannot increase it. No host environment or network authority is
inherited.
