from pathlib import Path
import base64
import os
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
SAT = ROOT / "sat/target/debug" / ("sat.exe" if os.name == "nt" else "sat")


def run(args, cwd, env, success=True):
    result = subprocess.run([str(SAT), *map(str, args)], cwd=cwd, env=env,
                            text=True, capture_output=True)
    if success and result.returncode:
        raise RuntimeError(result.stderr or result.stdout)
    if not success and result.returncode == 0:
        raise RuntimeError("expected SAT invocation to fail")
    return result


def main():
    with tempfile.TemporaryDirectory(prefix="sat-pdftools-") as temporary:
        base = Path(temporary)
        env = dict(os.environ, SAT_HOME=str(base / "registry"))
        subprocess.run(["bash", str(ROOT / "sat/pdf-tools/build.sh")],
                       cwd=ROOT, env=env, check=True)
        inputs = base / "inputs"
        pdfs = base / "pdfs"
        output = base / "output"
        for folder in (inputs, pdfs, output):
            folder.mkdir()
        # Self-created 2x2 RGB fixtures; no personal images or Pillow needed.
        fixtures = {
            "images.png": (
                "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAIAAAD91JpzAAAAFklEQVR4nGP0rFzLwMDAxMDAwMDAAAAQ"
                "+gFzUuu6fAAAAABJRU5ErkJggg=="
            ),
            "images.jpeg": (
                "/9j/4AAQSkZJRgABAQAAAQABAAD/2wBDAAgGBgcGBQgHBwcJCQgKDBQNDAsLDBkSEw8UHRofHh0aHBwg"
                "JC4nICIsIxwcKDcpLDAxNDQ0Hyc5PTgyPC4zNDL/2wBDAQkJCQwLDBgNDRgyIRwhMjIyMjIyMjIyMjIy"
                "MjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjL/wAARCAACAAIDASIAAhEBAxEB/8QA"
                "HwAAAQUBAQEBAQEAAAAAAAAAAAECAwQFBgcICQoL/8QAtRAAAgEDAwIEAwUFBAQAAAF9AQIDAAQRBRIh"
                "MUEGE1FhByJxFDKBkaEII0KxwRVS0fAkM2JyggkKFhcYGRolJicoKSo0NTY3ODk6Q0RFRkdISUpTVFVW"
                "V1hZWmNkZWZnaGlqc3R1dnd4eXqDhIWGh4iJipKTlJWWl5iZmqKjpKWmp6ipqrKztLW2t7i5usLDxMXG"
                "x8jJytLT1NXW19jZ2uHi4+Tl5ufo6erx8vP09fb3+Pn6/8QAHwEAAwEBAQEBAQEBAQAAAAAAAAECAwQF"
                "BgcICQoL/8QAtREAAgECBAQDBAcFBAQAAQJ3AAECAxEEBSExBhJBUQdhcRMiMoEIFEKRobHBCSMzUvAV"
                "YnLRChYkNOEl8RcYGRomJygpKjU2Nzg5OkNERUZHSElKU1RVVldYWVpjZGVmZ2hpanN0dXZ3eHl6goOE"
                "hYaHiImKkpOUlZaXmJmaoqOkpaanqKmqsrO0tba3uLm6wsPExcbHyMnK0tPU1dbX2Nna4uPk5ebn6Onq"
                "8vP09fb3+Pn6/9oADAMBAAIRAxEAPwChRRRXvHjH/9k="
            ),
        }
        for name, encoded in fixtures.items():
            (inputs / name).write_bytes(base64.b64decode(encoded))

        combined = run(["pdftools", "--convert=.",
                        f"--destination={output / 'all-images.pdf'}", "--read=."],
                       cwd=inputs, env=env)
        assert "2 pages" in combined.stdout, combined.stdout

        repeated = run(["pdftools", f"--convert={inputs / 'images.png'}",
                        f"--convert={inputs / 'images.jpeg'}",
                        f"--destination={output / 'ordered.pdf'}",
                        f"--read={inputs}"], cwd=base, env=env)
        assert "2 pages" in repeated.stdout, repeated.stdout

        for filename in ("images.png", "images.jpeg"):
            run(["pdftools", f"--convert={inputs / filename}",
                 f"--destination={output / (filename + '.pdf')}",
                 f"--read={inputs / filename}"], cwd=base, env=env)
        shutil.copy(output / "images.png.pdf", pdfs / "a.pdf")
        shutil.copy(output / "images.jpeg.pdf", pdfs / "b.pdf")

        for _ in range(2):
            merged = run(["pdftools", f"--merge={pdfs}",
                          f"--destination={output / 'merged.pdf'}",
                          f"--read={pdfs}"], cwd=base, env=env)
            assert "2 pages" in merged.stdout, merged.stdout
            assert (output / "merged.pdf").read_bytes().startswith(b"%PDF-")

        denied = run(["pdftools", f"--convert={inputs / 'images.png'}",
                      f"--destination={output / 'denied.pdf'}"],
                     cwd=base, env=env, success=False)
        assert not (output / "denied.pdf").exists()
        print("SAT PDF component: conversion, merge, exact destination write, and denied ungranted read passed")


if __name__ == "__main__":
    main()
