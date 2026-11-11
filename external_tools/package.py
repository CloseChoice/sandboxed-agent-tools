"""Create a deterministic SAT package from a built component and checked metadata."""
import io
from pathlib import Path
import sys
import tarfile
import tomllib
import zipfile


def package(tool_dir, component, archive_format):
    metadata = tomllib.loads((tool_dir / "tool.toml").read_text())
    tomllib.loads((tool_dir / "defaults.toml").read_text())
    files = {
        "tool.wasm": component.read_bytes(),
        "tool.toml": (tool_dir / "tool.toml").read_bytes(),
        "defaults.toml": (tool_dir / "defaults.toml").read_bytes(),
    }
    if not files["tool.wasm"].startswith(b"\0asm\x0d\0\x01\0"):
        raise ValueError("tool.wasm must be a WebAssembly component")
    destination = tool_dir / "dist" / (metadata["name"] + "." + archive_format)
    destination.parent.mkdir(exist_ok=True)
    if archive_format == "tar":
        with tarfile.open(destination, "w", format=tarfile.USTAR_FORMAT) as archive:
            for name, data in files.items():
                entry = tarfile.TarInfo(name)
                entry.size = len(data)
                entry.mode = 0o644
                archive.addfile(entry, io.BytesIO(data))
    elif archive_format == "zip":
        with zipfile.ZipFile(destination, "w", compression=zipfile.ZIP_DEFLATED) as archive:
            for name, data in files.items():
                entry = zipfile.ZipInfo(name, (1980, 1, 1, 0, 0, 0))
                entry.create_system = 3
                entry.external_attr = 0o100644 << 16
                entry.compress_type = zipfile.ZIP_DEFLATED
                archive.writestr(entry, data)
    else:
        raise ValueError("archive format must be tar or zip")
    print(destination)
    return destination


if __name__ == "__main__":
    package(Path(sys.argv[1]), Path(sys.argv[2]), sys.argv[3])
