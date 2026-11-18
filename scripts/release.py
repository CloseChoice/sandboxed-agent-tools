"""Validate the SAT version and the explicitly selected release tools."""
import argparse
import json
from pathlib import Path
import re
import tomllib

ROOT = Path(__file__).resolve().parents[1]


def read(path):
    return tomllib.loads(path.read_text())


def release_tools(root=ROOT):
    names = read(root / "external_tools/release-tools.toml")["tools"]
    if not isinstance(names, list) or not names:
        raise ValueError("release tools must be a nonempty list")
    if any(not isinstance(n, str) or not re.fullmatch(r"[a-z][a-z0-9-]*", n) for n in names):
        raise ValueError("invalid release tool directory")
    if len(set(names)) != len(names):
        raise ValueError("duplicate release tools")
    return names


def check_package(directory):
    package = read(directory / "Cargo.toml")["package"]
    entries = read(directory / "Cargo.lock")["package"]
    if not any(p["name"] == package["name"] and p["version"] == package["version"]
               and "source" not in p for p in entries):
        raise ValueError(f"{directory.name}: Cargo.lock does not match Cargo.toml")
    return package["version"]


def tagged_version(version, tag):
    expected = f"v{version}"
    if not tag or tag == expected:
        return version
    prefix = f"{expected}-"
    identifier = r"(?:0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*)"
    prerelease = rf"{identifier}(?:\.{identifier})*"
    if (re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", version)
            and tag.startswith(prefix)
            and re.fullmatch(prerelease, tag[len(prefix):])):
        return tag[1:]
    raise ValueError(f"release tag must be {expected} or a matching prerelease, got {tag}")


def validate(root=ROOT, tag=None):
    version = check_package(root / "sat")
    if tag is not None:
        if not tag:
            raise ValueError("release tag must be nonempty")
        tagged_version(version, tag)
    names = release_tools(root)
    identities = set()
    for name in names:
        directory = root / "external_tools" / name
        metadata = read(directory / "tool.toml")
        if metadata["name"] != name or metadata["name"] in identities:
            raise ValueError(f"{name}: release identity must match its directory")
        identities.add(metadata["name"])
        for required in ["package.sh", "defaults.toml"]:
            if not (directory / required).is_file():
                raise ValueError(f"{name}: missing {required}")
        read(directory / "defaults.toml")
        if (directory / "Cargo.toml").exists():
            if check_package(directory) != metadata["version"]:
                raise ValueError(f"{name}: tool.toml version differs from Cargo.toml")
    return version, names


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check-tag")
    parser.add_argument("--matrix", action="store_true", help="Print selected tool directories as JSON")
    args = parser.parse_args()
    try:
        version, names = validate(tag=args.check_tag)
    except (OSError, ValueError, KeyError, TypeError) as error:
        parser.error(str(error))
    print(json.dumps(names) if args.matrix else f"SAT {version}: {', '.join(names)}")


if __name__ == "__main__":
    main()
