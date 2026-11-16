"""Package, verify, and smoke-test native SAT release binaries."""
import argparse
import gzip
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import tomllib
import zipfile

ROOT = Path(__file__).resolve().parents[1]
TARGETS = [
    {"os": "ubuntu-24.04", "target": "x86_64-unknown-linux-gnu"},
    {"os": "macos-15", "target": "aarch64-apple-darwin"},
    {"os": "macos-15-intel", "target": "x86_64-apple-darwin"},
    {"os": "windows-2025", "target": "x86_64-pc-windows-msvc"},
]


def version():
    return tomllib.loads((ROOT / "sat/Cargo.toml").read_text())["package"]["version"]


def binary_name(target):
    if target not in {entry["target"] for entry in TARGETS}:
        raise ValueError(f"unsupported host target: {target}")
    return "sat.exe" if "windows" in target else "sat"


def archive_name(target):
    suffix = ".zip" if "windows" in target else ".tar.gz"
    return f"sat-{version()}-{target}{suffix}"


def check_binary(data, target):
    signatures = [b"MZ"] if "windows" in target else (
        [b"\xcf\xfa\xed\xfe", b"\xfe\xed\xfa\xcf", b"\xca\xfe\xba\xbe"]
        if "apple" in target else [b"\x7fELF"])
    if not any(data.startswith(signature) for signature in signatures):
        raise ValueError(f"unexpected executable format for {target}")


def pack(target, binary, output):
    name = binary_name(target)
    data = binary.read_bytes()
    check_binary(data, target)
    output.mkdir(parents=True, exist_ok=True)
    destination = output / archive_name(target)
    if "windows" in target:
        with zipfile.ZipFile(destination, "w", compression=zipfile.ZIP_DEFLATED) as archive:
            entry = zipfile.ZipInfo(name, (1980, 1, 1, 0, 0, 0))
            entry.create_system = 3
            entry.external_attr = 0o100755 << 16
            archive.writestr(entry, data, compress_type=zipfile.ZIP_DEFLATED)
    else:
        with destination.open("wb") as stream:
            with gzip.GzipFile(filename="", fileobj=stream, mode="wb", mtime=0) as compressed:
                with tarfile.open(fileobj=compressed, mode="w", format=tarfile.USTAR_FORMAT) as archive:
                    entry = tarfile.TarInfo(name)
                    entry.size = len(data)
                    entry.mode = 0o755
                    archive.addfile(entry, io.BytesIO(data))
    print(destination)
    return destination


def unpack_checked(path, target):
    name = binary_name(target)
    if path.name != archive_name(target):
        raise ValueError(f"unexpected host archive name: {path.name}")
    if "windows" in target:
        with zipfile.ZipFile(path) as archive:
            if archive.namelist() != [name] or archive.getinfo(name).is_dir():
                raise ValueError("host archive must contain only its executable")
            data = archive.read(name)
    else:
        with tarfile.open(path, "r:gz") as archive:
            members = archive.getmembers()
            if (len(members) != 1 or members[0].name != name
                    or not members[0].isfile() or members[0].mode != 0o755):
                raise ValueError("host archive must contain only its executable with mode 0755")
            data = archive.extractfile(members[0]).read()
    check_binary(data, target)
    return data


def verify(directory):
    expected = {archive_name(entry["target"]): entry["target"] for entry in TARGETS}
    actual = {p.name for p in directory.glob("sat-*") if p.is_file()}
    if actual != set(expected):
        raise ValueError(f"incomplete host release: missing={sorted(set(expected) - actual)}, "
                         f"unexpected={sorted(actual - set(expected))}")
    for name, target in expected.items():
        unpack_checked(directory / name, target)
        print(f"Verified {name}")


def smoke(path, target, tool_archive):
    data = unpack_checked(path, target)
    with tempfile.TemporaryDirectory(prefix="sat-host-smoke-") as temporary:
        project = Path(temporary)
        binary = project / binary_name(target)
        binary.write_bytes(data)
        binary.chmod(0o755)
        env = dict(os.environ, SAT_HOME=str(project / "registry"), SAT_BINARY=str(binary))
        logger = Path.home() / ".codex/skills/sat/scripts/run_sat.py"
        prefix = [sys.executable, str(logger), "--"] if logger.exists() else [str(binary)]

        def run(*args, data=b""):
            return subprocess.run([*prefix, *args], cwd=project, env=env, input=data,
                                  capture_output=True, check=True, timeout=240).stdout

        if run("--version").decode().strip() != f"sat {version()}":
            raise ValueError("packaged binary reports the wrong version")
        run("--help")
        run("register", str(tool_archive.resolve()), data=b"[]\n" * 5 + b"unset\n")
        if run("jaq", "-n", "1 + 2").strip() != b"3":
            raise ValueError("packaged host could not run the portable jaq component")
        print(f"Packaged SAT {version()} runs jaq on {target}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("matrix")
    build = commands.add_parser("pack")
    build.add_argument("--target", required=True)
    build.add_argument("--binary", type=Path, required=True)
    build.add_argument("--output", type=Path, default=ROOT / "dist/host")
    check = commands.add_parser("verify")
    check.add_argument("directory", type=Path)
    test = commands.add_parser("smoke")
    test.add_argument("archive", type=Path)
    test.add_argument("--target", required=True)
    test.add_argument("--tool-archive", type=Path, required=True)
    args = parser.parse_args()
    if args.command == "matrix":
        print(json.dumps(TARGETS))
    elif args.command == "pack":
        pack(args.target, args.binary, args.output)
    elif args.command == "verify":
        verify(args.directory)
    else:
        smoke(args.archive, args.target, args.tool_archive)


if __name__ == "__main__":
    main()
