"""Build and register three local demos; no network requests are made by the tools."""
from pathlib import Path
import os
import subprocess

ROOT = Path(__file__).resolve().parent.parent
SAT = ROOT / "sat/target/debug" / ("sat.exe" if os.name == "nt" else "sat")


def prepare(destination):
    destination.mkdir(parents=True, exist_ok=True)
    for manifest, target in [
        ("sat/Cargo.toml", None),
        ("random-number-example/Cargo.toml", "wasm32-wasip2"),
        ("http-get-example/Cargo.toml", "wasm32-wasip2"),
        ("sat/examples/files-guest/Cargo.toml", "wasm32-wasip2"),
    ]:
        cmd = ["cargo", "build", "--locked", "--manifest-path", str(ROOT / manifest)]
        if target:
            cmd += ["--target", target, "--release"]
        subprocess.run(cmd, cwd=ROOT, check=True)
    env = dict(os.environ, SAT_HOME=str(destination / "registry"))
    for name, component, metadata, policy in [
        ("random", "random-number-example/target/wasm32-wasip2/release/random_number_component.wasm", "random", None),
        ("httpbin", "http-get-example/target/wasm32-wasip2/release/http_get_component.wasm", "http", "http-policy"),
        ("files", "sat/examples/files-guest/target/wasm32-wasip2/release/sat_files_guest.wasm", "files", "files-policy"),
    ]:
        output = destination / f"{name}.wasm"
        cmd = [str(SAT), "pack", str(ROOT / component), "--metadata", str(ROOT / f"sat/examples/{metadata}.toml"), "-o", str(output)]
        if policy:
            cmd += ["--policy", str(ROOT / f"sat/examples/{policy}.toml")]
        subprocess.run(cmd, check=True, env=env)
        subprocess.run([str(SAT), "install", str(output), "--replace"], check=True, env=env)
    documents = destination / "documents"
    documents.mkdir(exist_ok=True)
    (documents / "hello.txt").write_text("Hello from the granted documents folder.\n")
    (destination / "input.txt").write_text("Hello from the selected input file.\n")
    (destination / "output.txt").touch()
    return env


if __name__ == "__main__":
    env = prepare(ROOT / "sat/target/demo")
    print(f"\nSAT binary: {SAT}\nSet SAT_HOME to: {env['SAT_HOME']}")
    print("Then try: sat list, sat random, sat httpbin --help, sat files --help")
