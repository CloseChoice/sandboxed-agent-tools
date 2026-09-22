"""Build/register Brave in the local SAT demo registry, without making a search."""
from pathlib import Path
import os
import subprocess
import tomllib

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parent


def setup():
    env = dict(os.environ)
    env.setdefault("SAT_HOME", str(REPO / "sat/target/demo/registry"))
    registry = Path(env["SAT_HOME"]).resolve()
    env["SAT_HOME"] = str(registry)
    subprocess.run(["cargo", "build", "--locked", "--manifest-path", str(REPO / "sat/Cargo.toml")], check=True)
    subprocess.run(["cargo", "build", "--locked", "--manifest-path", str(REPO / "brave-search-example/Cargo.toml"),
                    "-p", "brave-search-guest", "--target", "wasm32-wasip2", "--release"], check=True)
    sat = REPO / "sat/target/debug" / ("sat.exe" if os.name == "nt" else "sat")
    target = ROOT / "target"
    target.mkdir(exist_ok=True)
    package = target / "brave-search.wasm"
    subprocess.run([str(sat), "pack", str(REPO / "brave-search-example/target/wasm32-wasip2/release/brave_search_guest.wasm"),
                    "--metadata", str(REPO / "sat/examples/brave.toml"),
                    "--policy", str(REPO / "sat/examples/brave-policy.toml"), "-o", str(package)], env=env, check=True)
    subprocess.run([str(sat), "install", str(package), "--replace"], env=env, check=True)
    credentials = registry / "credentials.toml"
    existing = credentials.read_text() if credentials.exists() else ""
    if "brave-search" not in tomllib.loads(existing):
        with credentials.open("a") as output:
            output.write("\n" + (ROOT / "credentials.example.toml").read_text())
    print(f"SAT_BIN={sat}\nSAT_HOME={registry}")
    print("Supply BRAVE_API_KEY to Pi's environment; the setup stores no secret and sends no searches.")


if __name__ == "__main__":
    setup()
