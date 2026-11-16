"""Check installable tar archives against the tool sources in this checkout."""
import argparse
import os
import subprocess
import sys
import tempfile
from pathlib import Path
import tarfile
import tomllib

ROOT = Path(__file__).resolve().parent
COMPONENT_HEADER = b'\0asm\x0d\0\x01\0'
PACKAGE_FILES = {'tool.wasm', 'tool.toml', 'defaults.toml'}


def tool_directories():
    tools = {}
    for script in sorted(ROOT.glob('*/package.sh')):
        directory = script.parent
        name = tomllib.loads((directory / 'tool.toml').read_text())['name']
        if name in tools:
            raise ValueError(f'duplicate tool name: {name}')
        tools[name] = directory
    return tools


def check_archive(path, tools):
    directory = tools.get(path.stem)
    if directory is None or path.suffix != '.tar':
        raise ValueError(f'unknown tool archive: {path.name}')
    with tarfile.open(path, 'r:') as archive:
        members = archive.getmembers()
        if len(members) != len(PACKAGE_FILES) or {m.name for m in members} != PACKAGE_FILES:
            raise ValueError(f'{path.name}: expected exactly {sorted(PACKAGE_FILES)}')
        if not all(member.isfile() for member in members):
            raise ValueError(f'{path.name}: package entries must be regular files')
        for name in ['tool.toml', 'defaults.toml']:
            data = archive.extractfile(name).read()
            tomllib.loads(data.decode('utf-8'))
            if data != (directory / name).read_bytes():
                raise ValueError(f'{path.name}: {name} differs from the release source')
        if archive.extractfile('tool.wasm').read(8) != COMPONENT_HEADER:
            raise ValueError(f'{path.name}: tool.wasm is not a WebAssembly component')
    print(f'Verified {path.name}', flush=True)


def check_registration(sat, archives):
    # Separate test registry/project, empty grants, and no provider API calls.
    with tempfile.TemporaryDirectory(prefix='sat-release-check-') as temporary:
        project = Path(temporary)
        env = dict(os.environ, SAT_HOME=str(project / 'registry'), SAT_BINARY=str(sat))
        logger = Path.home() / '.codex/skills/sat/scripts/run_sat.py'
        prefix = [sys.executable, str(logger), '--'] if logger.exists() else [str(sat)]
        for archive in archives:
            print(f'Registering and compiling {archive.name} with SAT...', flush=True)
            commands = [
                ([*prefix, 'register', str(archive.resolve())], b'[]\n' * 5 + b'unset\n'),
                ([*prefix, archive.stem, '--help'], b''),
            ]
            for command, input_bytes in commands:
                result = subprocess.run(command, cwd=project, env=env, input=input_bytes,
                                        capture_output=True, timeout=240)
                if result.returncode != 0:
                    raise ValueError(f'{archive.name}: SAT validation failed: {result.stderr.decode(errors="replace")}')
            print(f'SAT registered {archive.name} and displayed tool help', flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--complete', action='store_true', help='Require exactly the selected release tools')
    parser.add_argument('--sat', type=Path, help='Also register each archive and check its CLI help with this SAT binary')
    parser.add_argument('archives', nargs='+', type=Path)
    args = parser.parse_args()
    tools = tool_directories()
    if args.complete:
        selection = tomllib.loads((ROOT / 'release-tools.toml').read_text())['tools']
        if not isinstance(selection, list) or not selection or any(not isinstance(n, str) for n in selection):
            parser.error('release selection must be a nonempty list of tool names')
        if len(set(selection)) != len(selection) or any(name not in tools for name in selection):
            parser.error('release selection contains duplicate or unknown tools')
        expected = {f'{name}.tar' for name in selection}
        actual = {p.name for p in args.archives}
        if len(args.archives) != len(expected) or actual != expected:
            parser.error(f'incomplete release: missing={sorted(expected - actual)}, unexpected={sorted(actual - expected)}')
    for path in args.archives:
        check_archive(path, tools)
    if args.sat:
        check_registration(args.sat.resolve(), args.archives)


if __name__ == '__main__':
    main()
