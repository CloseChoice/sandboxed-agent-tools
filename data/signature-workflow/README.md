# PDF signature placement walkthrough

This folder is a reproducible, fictional example of placing a supplied signature
image into a PDF. The signature is a hand-drawn SVG squiggle for this demo; it
does not represent a real person or authorize this sample contract.

![Rendered sample with the dummy signature on Alex Morgan's signature line](signed-contract-preview.png)

## What I normally do

1. **Inspect both inputs.** Check that the PDF opens, how many pages it has, its
   page sizes and rotations, and whether it contains existing digital signature
   fields. Check the signature image's format, transparency, orientation, and
   visible margins. Keep the original files untouched.
2. **Resolve which line is intended.** Extract text and page coordinates first.
   Look for the signer's name and nearby labels such as “Signature.” Render the
   relevant page and inspect the actual line, signer block, and nearby text.
   Text extraction does not report the position of a drawn rule, so finding the
   line may also require inspecting PDF vector drawing operations or detecting a
   line in the rendered page. For scanned PDFs, OCR supplies text candidates,
   and visual inspection or image analysis is still needed to confirm the line.
3. **Ask when the target is ambiguous.** A page can have several signature
   lines, initials boxes, or signers. Do not guess which person or line the user
   intends. A locator should return page, rectangle, nearby label/name, and a
   confidence score so the chosen target can be reviewed.
4. **Choose a placement rectangle.** Convert the selected line into PDF page
   coordinates, accounting for page rotation and crop boxes. PDF coordinates
   usually use points (72 points per inch) from the lower-left; text tools often
   report top-left coordinates. Fit the image within the line, preserve its
   aspect ratio, and avoid covering labels or other content.
5. **Place and save a separate output.** Add the image as an overlay on a copy of
   the source PDF. Preserve the source and use a new output path unless the user
   explicitly asks to replace it.
6. **Verify the result.** Render the output page and check visually that the
   mark is on the intended line, at a reasonable size, and does not cover text.
   Recheck page count and that the output opens. If a certificate-based digital
   signature is also needed, apply it after all PDF edits; adding an image is
   only a visible mark and is not cryptographic signing.

## This sample, step by step

1. `dummy-contract.tex` defines a one-page A4 consulting agreement with two
   signature blocks. I compiled it to `dummy-contract.pdf` with `pdflatex`.
   The contract labels the second block “Consultant” and names Alex Morgan, so
   that is the unambiguous demo target.
2. `dummy-signature.svg` contains a synthetic blue pen stroke. I rasterized it
   to transparent `dummy-signature.png` with ImageMagick. Keeping transparency
   lets the signature sit over the rule without a white rectangle.
3. I inspected the PDF with `pdfinfo` and `pdftotext -bbox-layout`, then rendered
   the page with `pdftoppm`. The page is 595.276 × 841.89 points. Text extraction
   locates the Consultant block's “Signature” label around x=315.8 points and
   y=710.9 points from the top. The line itself is a drawn rule, not text, so I
   confirmed its position in the rendered page. This demo's overlay starts at
   x=316, y=135 points from the lower-left, with a width of 170 points; its
   aspect ratio determines the height.
4. `place-signature.tex` uses `pdfpages` to include the original page and a
   foreground TikZ overlay to place the PNG. I compiled it as
   `signed-contract.pdf`.
5. I rendered the result to `signed-contract-preview.png` and visually checked
   that the mark sits over Alex Morgan's line, leaves the printed name/date
   readable, and does not touch the other signer's block. `pdfinfo` confirms the
   output remains a one-page A4 PDF.

Rebuild from the repository root (requires TeX Live, ImageMagick, and Poppler):

```sh
cd data/signature-workflow
convert -background none -density 150 dummy-signature.svg -resize 1200x330 dummy-signature.png
pdflatex -interaction=nonstopmode -halt-on-error dummy-contract.tex
pdflatex -interaction=nonstopmode -halt-on-error -jobname=signed-contract place-signature.tex
pdftoppm -f 1 -l 1 -png -r 120 -singlefile signed-contract.pdf signed-contract-preview
```

The placement in `place-signature.tex` is deliberately fixed for this known
sample. It is not automatic signature-line detection. SAT's current PDF
component converts and merges documents but does not locate signature lines or
place image overlays. A future locator and placement tool should expose
coordinates and selected candidates explicitly, and keep the final placement
reviewable.
