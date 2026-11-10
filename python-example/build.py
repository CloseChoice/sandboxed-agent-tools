"""Build and optionally register the minimal WASI CPython tool."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import venv

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parent
BUILDER = "0.25.0"
TARGET = ROOT / "target"


def build(install=True, sat_home=None):
    profile = os.environ.get("SAT_BUILD_PROFILE", "release")
    if profile not in ("release", "debug"):
        raise ValueError("SAT_BUILD_PROFILE must be release or debug")
    TARGET.mkdir(exist_ok=True)
    builder = TARGET / "builder"
    executable_dir = builder / ("Scripts" if os.name == "nt" else "bin")
    python = executable_dir / ("python.exe" if os.name == "nt" else "python")
    if not python.exists():
        venv.EnvBuilder(with_pip=True).create(builder)
    version = subprocess.run(
        [str(python), "-c", "import importlib.metadata; print(importlib.metadata.version('componentize-py'))"],
        capture_output=True, text=True,
    )
    if version.returncode or version.stdout.strip() != BUILDER:
        subprocess.run([str(python), "-m", "pip", "install", f"componentize-py=={BUILDER}"], check=True)

    # Cache only an exact builder/source specification and verify the cached artifact.
    spec = {"builder": f"componentize-py=={BUILDER}", "sources": {
        name: (ROOT / name).read_text() for name in ["app.py", "wit/world.wit"]
    }}
    fingerprint = hashlib.sha256(json.dumps(spec, sort_keys=True).encode()).hexdigest()
    raw = TARGET / "python.raw.wasm"
    record = TARGET / "build.json"
    cached = json.loads(record.read_text()) if record.exists() else {}
    if not (raw.exists() and cached.get("spec") == fingerprint
            and cached.get("sha256") == hashlib.sha256(raw.read_bytes()).hexdigest()):
        componentize = executable_dir / ("componentize-py.exe" if os.name == "nt" else "componentize-py")
        subprocess.run([
            str(componentize), "-d", "wit", "-w", "python-tool",
            "componentize", "-p", ".", "app", "-o", str(raw),
        ], cwd=ROOT, env=dict(os.environ, VIRTUAL_ENV=str(builder)), check=True)
        record.write_text(json.dumps({"spec": fingerprint, "sha256": hashlib.sha256(raw.read_bytes()).hexdigest()}))
    else:
        print("Reusing cached Python component.", flush=True)

    command = ["cargo", "build", "--locked", "--manifest-path", str(REPO / "sat/Cargo.toml")]
    if profile == "release":
        command.append("--release")
    subprocess.run(command, cwd=REPO, check=True)
    sat = REPO / f"sat/target/{profile}" / ("sat.exe" if os.name == "nt" else "sat")
    # Use the same demo registry as sat/demo.py, unless the caller chooses another.
    env = dict(os.environ)
    if sat_home is not None:
        env["SAT_HOME"] = str(sat_home)
    else:
        env.setdefault("SAT_HOME", str(REPO / "sat/target/demo/registry"))
    # Private test registries get a private package too; only the raw component
    # cache is shared with ordinary builds.
    package = (Path(sat_home).parent / "python.wasm") if sat_home is not None else TARGET / "python.wasm"
    package.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run([str(sat), "pack", str(raw), "--metadata", str(ROOT / "tool.toml"),
                    "--policy", str(ROOT / "policy.toml"), "-o", str(package)], env=env, check=True)
    if install:
        subprocess.run([str(sat), "install", str(package), "--replace"], env=env, check=True)
        print(f"Installed python. SAT_HOME={env['SAT_HOME']}")
    return sat, env


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--no-install", action="store_true")
    args = parser.parse_args()
    build(install=not args.no_install)
