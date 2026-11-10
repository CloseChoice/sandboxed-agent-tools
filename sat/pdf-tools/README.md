# SAT PDF tools

`pdftools` is a WASI Preview 2 component registered with SAT. It converts PNG
and JPEG images to PDF, placing one image on each page, and concatenates PDFs
from a granted directory in filename order. It uses lopdf and the `image` crate;
the guest has no network access.

## Build and install

From the repository root (Rust with `wasm32-wasip2` installed):

```sh
bash sat/pdf-tools/build.sh
```

This builds the component and SAT host, packages the component with SAT
metadata, and installs/replaces the local `pdftools` registration. The default
registry is `sat/target/demo/registry`; set `SAT_HOME` to use another registry.

## Convert images to one PDF

Repeat `--convert` for multiple image files or directories. Inputs are processed
in argument order; each directory is read non-recursively and its PNG, JPG, and
JPEG files are appended in filename order. The destination is one PDF file that
can be new or already exist; its parent directory must exist.

```sh
mkdir -p sat/target/pdf-output
sat pdftools --convert ./data/first.png --convert ./data/photos \
  --destination ./sat/target/pdf-output/images.pdf \
  --read ./data
```

From the input folder:

```sh
cd data
sat pdftools --convert . --destination ../sat/target/pdf-output/images.pdf \
  --read .
```

For one input file, grant only that file:

```sh
sat pdftools --convert ./data/images.png \
  --destination ./sat/target/pdf-output/image.pdf --read ./data/images.png
```

## Merge PDFs

Merge every PDF in the selected directory, sorted by filename. Keep the output
outside the source directory so a previous result cannot become an input on a
later run.

```sh
sat pdftools --merge ./pdfs \
  --destination ./sat/target/pdf-output/merged.pdf --read ./pdfs
```

Encrypted PDFs are rejected. This MVP preserves page content and common
inherited page attributes, but does not preserve document-level outlines, forms,
or named destinations. JPEGs are copied into the PDF without recompression;
grayscale and RGB JPEGs are supported. PNG transparency is flattened onto white.
Images are limited to 5 megapixels each; decoded PNGs are limited to 4
megapixels total per operation. Input data is limited to 16 MiB total and 256
files per operation. SAT enforces a 30-second execution deadline using epoch
interruption; the package's legacy fuel metadata is ignored.

## Interface and permissions

The WIT interface imports a binary `destination.write(list<u8>)` capability and
exports `run(convert, merge)`. SAT exposes the import as `--destination PATH`.
The host opens that exact file for writing (creating it if needed), and the guest
receives no path or access to its parent folder. Input authority comes from
explicit `--read` grants. The component has no directory write access, and
`--read-write` is not needed. The parent of a new destination file must already
exist. Exactly one of `--convert` or `--merge` is required.

## Check

```sh
python3 sat/pdf-tools/test.py
```

The integration check builds and installs the actual component into a temporary
SAT registry, converts self-contained synthetic PNG and JPEG fixtures with both directory and
repeated-file arguments, merges generated PDFs, and confirms an ungranted input
cannot be read. Outputs use exact-file write grants.
