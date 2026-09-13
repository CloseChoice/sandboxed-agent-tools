"""Exercise the real WASIp2 component and Wasmtime filesystem boundary."""
from pathlib import Path
import subprocess
import tempfile

WASM = Path(__file__).resolve().parents[1] / "dist/file-search.wasm"


with tempfile.TemporaryDirectory(prefix="file-search-test-") as tmp:
    root = Path(tmp)
    allowed = root / "allowed"
    allowed.mkdir()
    (allowed / "a.txt").write_text("first\nneedle here\nNeedle\nneedle again\n")
    (allowed / "unicode.txt").write_bytes("café\r\nneedle café".encode())
    (root / "secret.txt").write_text("needle secret\n")
    (allowed / "escape.txt").symlink_to(root / "secret.txt")
    (allowed / "context.txt").write_text(
        "one\ntwo\nhit three\nfour\nhit five\nsix\nseven\neight\nnine\nhit ten\n"
    )

    def run(query, *paths, grant=True):
        command = ["wasmtime", "run"]
        if grant:
            command += ["--dir", f"{allowed}::/data"]
        return subprocess.run(
            command + [str(WASM), query, *paths],
            capture_output=True, text=True, timeout=30,
        )

    result = run("needle", "/data/a.txt", "/data/unicode.txt")
    assert result.returncode == 0, result.stderr
    assert result.stdout == (
        "/data/a.txt:2:needle here\n/data/a.txt:4:needle again\n"
        "/data/unicode.txt:2:needle café\n"
    ), result.stdout
    result = run("absent", "/data/a.txt")
    assert result.returncode == 1 and not result.stdout, result
    result = run("café", "/data/unicode.txt")
    assert result.returncode == 0 and result.stdout.count("\n") == 2, result
    for path, grant in [
        ("/data/a.txt", False),
        (str(root / "secret.txt"), True),
        ("/data/../secret.txt", True),
        ("/data/escape.txt", True),
        ("/data/missing.txt", True),
    ]:
        result = run("needle", path, grant=grant)
        assert result.returncode == 1, (path, result)
        assert not result.stdout and result.stderr, (path, result)
    result = run("needle")
    assert result.returncode == 1 and result.stderr, result

    # Compare real output with grep, including overlap, adjacent groups, file
    # boundaries, start/end clipping, zero counts, and per-side overrides.
    for options in [
        ["-C", "1"], ["-B", "2"], ["-A", "2"],
        ["-C0"], ["-C2"], ["-C20"], ["-A0", "-C1"],
        ["-C1", "-B0"], ["-B1", "-A2"], ["-A1", "-A0"],
    ]:
        paths = ["context.txt", "a.txt"]
        expected = subprocess.run(
            ["grep", "-H", "-n", "-F", *options, "hit", *[str(allowed / p) for p in paths]],
            capture_output=True, text=True, check=True,
        ).stdout.replace(str(allowed), "/data")
        result = run(*options, "hit", *[f"/data/{p}" for p in paths])
        assert result.returncode == 0 and result.stdout == expected, (options, result, expected)
    result = run("-C1", "needle", "/data/a.txt", "/data/unicode.txt")
    assert "--\n/data/unicode.txt-1-café" in result.stdout, result
    result = run("-C3", "absent", "/data/a.txt")
    assert result.returncode == 1 and not result.stdout and not result.stderr, result
    for options in [["-C"], ["-A", "-1"], ["-B", "abc"], ["-C999999999999999999999999"], ["-Z"]]:
        result = run(*options)
        assert result.returncode == 1 and result.stderr and not result.stdout, result
    (allowed / "dash.txt").write_text("before\n--help\nafter\n")
    result = run("-C1", "--", "--help", "/data/dash.txt")
    assert result.returncode == 0 and result.stdout == (
        "/data/dash.txt-1-before\n/data/dash.txt:2:--help\n/data/dash.txt-3-after\n"
    ), result

print("CLI tests passed: matches, grep-compatible context, Unicode, exit codes, and denied filesystem access")
