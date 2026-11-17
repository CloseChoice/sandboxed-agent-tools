"""Integration checks for imported tools using SAT's archive and permission CLI."""
import base64
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
SAT = ROOT / 'sat/target/debug' / ('sat.exe' if os.name == 'nt' else 'sat')
LOGGER = Path.home() / '.codex/skills/sat/scripts/run_sat.py'
SAT_COMMAND = [sys.executable, str(LOGGER), '--'] if LOGGER.exists() else [str(SAT)]
TOOLS = ['code', 'jaq', 'ripgrep']


def check(name, build=True):
    if name == 'jaq':
        import runpy
        runpy.run_path(str(ROOT / 'external_tools/jaq/test.py'))['check'](build=build)
        return
    tool = ROOT / 'external_tools' / name
    command = 'pdftools' if name == 'pdf-tools' else name
    if build:
        subprocess.run(['bash', str(tool / 'package.sh')], check=True)
        subprocess.run(['cargo', 'build', '--locked', '--manifest-path', str(ROOT / 'sat/Cargo.toml')], check=True)
    with tempfile.TemporaryDirectory(prefix='sat-import-') as temporary:
        base = Path(temporary)
        env = dict(os.environ, SAT_HOME=str(base / 'registry'), SAT_TEST_SECRET='unrelated-host-secret', SAT_BINARY=str(SAT))

        def invoke(*args, input=None, ok=True):
            result = subprocess.run([*SAT_COMMAND, *map(str, args)], cwd=base, env=env,
                                    input=input, capture_output=True, timeout=180)
            assert (result.returncode == 0) == ok, (args, result.stdout, result.stderr)
            assert b'unrelated-host-secret' not in result.stdout + result.stderr
            return result

        invoke('register', tool / 'dist' / f'{command}.tar', input=b'y\n' * 6)
        help_text = json.loads(invoke(command, '--help', '--json').stdout)
        assert help_text['tool']['name'] == command
        source = base / 'source.py'
        source.write_text('def hello():\n    return "needle"\n')
        outside = base / 'secret.py'
        outside.write_text('def secret():\n    return "outside_secret"\n')
        original = source.read_bytes()
        if name == 'uutils':
            assert invoke(command, '--stdin', 'sort', input=b'b\na\n').stdout == b'a\nb\n'
            assert invoke(command, '--stdin', 'head', '-n', '1', input=b'a\nb\n').stdout == b'a\n'
            assert invoke(command, 'cat', source, '--', '--read', source).stdout == original
            invoke(command, 'cat', source, ok=False)
            invoke(command, 'cat', outside, '--', '--read', source, ok=False)
            invoke(command, 'unsupported-utility', ok=False)
            # A broad ancestor grant cannot expose trusted registry/cache state.
            registry = base / 'registry'
            invoke(command, 'cat', registry / 'tools/uutils/tool.toml', '--', '--read', base, ok=False)
            sentinel = registry / '.cache/protected-test'
            sentinel.parent.mkdir(exist_ok=True)
            sentinel.write_text('trusted host cache')
            invoke(command, '--input', 'poison', 'sort', '-o', sentinel, '--', '--write', base, ok=False)
            assert sentinel.read_text() == 'trusted host cache' 
        elif name in ['code', 'ripgrep']:
            args = ['outline'] if name == 'code' else ['needle']
            assert (b'hello' if name == 'code' else b'needle') in invoke(command, *args, '--paths', source, '--', '--read', source).stdout
            # Searchers may report an inaccessible root as an empty result.
            for denied in [source, outside]:
                grants = [] if denied == source else ['--', '--read', source]
                result = subprocess.run([*SAT_COMMAND, command, *args, '--paths', str(denied), *map(str, grants)], cwd=base, env=env, capture_output=True, timeout=180)
                assert b'def secret' not in result.stdout and b'def hello' not in result.stdout
                assert b'outside_secret' not in result.stdout and b'needle' not in result.stdout
            assert (b'hello' if name == 'code' else b'needle') in invoke(command, *args, '--paths', source, '--paths', source, '--', '--read', source).stdout
        else:
            image = base / 'pixel.png'
            image.write_bytes(base64.b64decode('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII='))
            output = base / 'result.pdf'
            output.touch()
            args = [command, '--convert', image, '--destination', output]
            invoke(*args, '--', '--read', image, '--write', output)
            assert output.read_bytes().startswith(b'%PDF-')
            saved = output.read_bytes()
            invoke(*args, '--', '--read', image, ok=False)
            assert output.read_bytes() == saved
            invoke(*args, '--', '--write', output, ok=False)
        if name == 'ripgrep':
            directory = base / 'search'
            directory.mkdir()
            (directory / 'a-readable.txt').write_text('needle before\n')
            (directory / 'z-readable.txt').write_text('needle after\n')
            (directory / 'denied.txt').write_text('needle denied secret\n')
            protected = base / 'protected.txt'
            protected.write_text('needle hard-linked secret\n')
            os.link(protected, directory / 'linked.txt')
            search = [command, 'needle', '--paths', directory]
            grants = ['--', '--read', directory, '--deny-read', '**/denied.txt']
            result = json.loads(invoke(*search, '--format', 'json', *grants).stdout)
            # Explicit deny-read entries are hidden during traversal; the hard link
            # is visible but its open is denied, so only that file is counted.
            assert result['skipped_paths'] == 1, result
            assert not result['truncated'], result
            assert {row['text'] for row in result['results']} == {'needle before', 'needle after'}, result
            text = invoke(*search, *grants).stdout
            assert b'[skipped 1 paths: permission denied]' in text, text
            assert b'secret' not in text, text
            assert protected.read_text() == 'needle hard-linked secret\n'
            # A denied traversal root must not prevent searching another root.
            roots = json.loads(invoke(command, 'needle', '--paths', protected,
                                      '--paths', directory / 'a-readable.txt',
                                      '--format', 'json', '--', '--read', directory).stdout)
            assert roots['skipped_paths'] == 1, roots
            assert [row['text'] for row in roots['results']] == ['needle before'], roots
        assert source.read_bytes() == original
        if hasattr(os, 'symlink') and name in ['uutils', 'code', 'ripgrep']:
            link = base / 'escape.py'
            link.symlink_to(outside)
            if name == 'uutils':
                args = [command, 'cat', link]
            else:
                args = [command, *(['outline'] if name == 'code' else ['outside_secret']), '--paths', link]
            result = subprocess.run([*SAT_COMMAND, *map(str, args), '--', '--read', str(source)], cwd=base, env=env, capture_output=True, timeout=180)
            assert b'outside_secret' not in result.stdout
    print(f'{name}: archive, execution, and confinement checks passed')


if __name__ == '__main__':
    import sys
    for name in TOOLS:
        check(name, build='--skip-build' not in sys.argv)
