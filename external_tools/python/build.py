"""Build the pinned WASI CPython component; execution remains inside SAT."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import venv

ROOT = Path(__file__).resolve().parent
BUILDER = "0.25.0"
TARGET = ROOT / "target"


def build():
    TARGET.mkdir(exist_ok=True)
    builder = TARGET / "builder"
    executables = builder / ("Scripts" if os.name == "nt" else "bin")
    python = executables / ("python.exe" if os.name == "nt" else "python")
    if not python.exists():
        venv.EnvBuilder(with_pip=True).create(builder)
    version = subprocess.run(
        [str(python), "-c", "import importlib.metadata; print(importlib.metadata.version('componentize-py'))"],
        capture_output=True, text=True, check=False,
    )
    if version.returncode or version.stdout.strip() != BUILDER:
        subprocess.run([str(python), "-m", "pip", "install", f"componentize-py=={BUILDER}"], check=True)
    spec = {
        "builder": f"componentize-py=={BUILDER}",
        "host_python": subprocess.check_output([str(python), "--version"], text=True).strip(),
        "sources": {name: (ROOT / name).read_text() for name in ["app.py", "wit/world.wit"]},
    }
    fingerprint = hashlib.sha256(json.dumps(spec, sort_keys=True).encode()).hexdigest()
    output = TARGET / "python.wasm"
    record = TARGET / "build.json"
    try:
        cached = json.loads(record.read_text())
    except (OSError, ValueError):
        cached = {}
    if (output.exists() and cached.get("spec") == fingerprint
            and cached.get("sha256") == hashlib.sha256(output.read_bytes()).hexdigest()):
        print("Reusing verified Python component.", flush=True)
        return output
    componentize = executables / ("componentize-py.exe" if os.name == "nt" else "componentize-py")
    pending = TARGET / "python.pending.wasm"
    try:
        subprocess.run([
            str(componentize), "-d", "wit", "-w", "python-tool",
            "componentize", "-p", ".", "app", "-o", str(pending),
        ], cwd=ROOT, env=dict(os.environ, VIRTUAL_ENV=str(builder)), check=True)
        pending.replace(output)
    finally:
        pending.unlink(missing_ok=True)
    record.write_text(json.dumps({
        "spec": fingerprint, "sha256": hashlib.sha256(output.read_bytes()).hexdigest(),
    }))
    return output


if __name__ == "__main__":
    build()
