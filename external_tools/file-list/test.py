"""Offline integration and boundary checks; run with python3 test.py."""
import os
import json
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parent


def cargo(*args):
    subprocess.run(["cargo", *args], cwd=ROOT, check=True)


cargo("build", "--locked", "-p", "file-list-host")
host = ROOT / "target/debug" / ("file-list-host.exe" if os.name == "nt" else "file-list-host")
component = ROOT / "target/wasm32-wasip2/debug/file_list_component.wasm"

try:
    cargo("build", "--locked", "-p", "file-list-component", "--target", "wasm32-wasip2",
          "--features", "security-probe")
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        directory = root / "listing"
        directory.mkdir()
        (directory / "hello.txt").write_text("unchanged", encoding="utf-8")
        (root / "message.txt").write_text("Selected text.\n", encoding="utf-8")
        (root / "secret.txt").write_text("UNGRANTED SECRET", encoding="utf-8")
        try:
            (directory / "escape").symlink_to(root / "secret.txt")
        except OSError:
            print("Symlink check unavailable on this platform; other checks still run.")
        config = root / "tool.toml"
        config.write_text(
            f"component = {json.dumps(component.as_posix())}\n"
            '[permissions]\ndirectory = "listing"\nfile = "message.txt"\n',
            encoding="utf-8",
        )
        result = subprocess.run([str(host), str(config)], cwd=ROOT,
                                capture_output=True, text=True)
        assert result.returncode == 0, result.stderr
        assert "hello.txt" in result.stdout
        assert result.stdout.endswith("content of file message.txt:\nSelected text.\n")
        assert "UNGRANTED SECRET" not in result.stdout
        assert str(root) not in result.stdout
        assert (directory / "hello.txt").read_text() == "unchanged"
        assert not (directory / "new.txt").exists()
        print("Listing, selected-file read, and sandbox boundary checks passed.")
finally:
    # Leave the normal, minimal component as the build output.
    cargo("build", "--locked", "-p", "file-list-component", "--target", "wasm32-wasip2")
