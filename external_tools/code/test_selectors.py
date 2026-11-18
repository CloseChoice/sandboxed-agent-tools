"""WASI selector and confinement checks; every SAT command uses the logging skill."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

TOOL = Path(__file__).resolve().parent
ROOT = TOOL.parents[1]
WRAPPER = Path.home() / '.codex/skills/sat/scripts/run_sat.py'


def check(sat=None):
    with tempfile.TemporaryDirectory(prefix='sat-code-selectors-') as tmp:
        base = Path(tmp)
        project = base / 'project'
        src = project / 'src'
        lib = project / 'lib'
        src.mkdir(parents=True)
        lib.mkdir()
        (src / 'nested').mkdir()
        (src / 'node_modules').mkdir()
        for relative, symbol in [
            ('a.ts', 'alpha'), ('a.test.ts', 'testAlpha'),
            ('nested/b.ts', 'beta'), ('nested/c.tsx', 'component'),
            ('node_modules/dep.ts', 'dependency'), ('ignored.ts', 'ignored'),
            ('.hidden.ts', 'hidden'),
        ]:
            (src / relative).write_text(f'function {symbol}() {{ return 1; }}\n')
        (src / '.gitignore').write_text('ignored.ts\nnode_modules/\n')
        (lib / 'extra.ts').write_text('function extra() {}\n')
        secret = base / 'secret.ts'
        secret.write_text('function secretMarker() {}\n')
        (src / 'escape.ts').symlink_to(secret)
        (src / 'escape-dir').symlink_to(base, target_is_directory=True)
        env = dict(os.environ, SAT_HOME=str(base / 'registry'))
        if sat is not None:
            env['SAT_BINARY'] = str(sat)
        calls = 0
        failures = []

        def invoke(*args, ok=True, stdin=None):
            nonlocal calls
            calls += 1
            result = subprocess.run(
                [*([sys.executable, str(WRAPPER), '--'] if WRAPPER.exists() else [str(sat)]), *map(str, args)],
                input=stdin, capture_output=True, env=env, cwd=base, timeout=180,
            )
            if result.returncode:
                failures.append({'args': list(map(str, args)), 'exit': result.returncode,
                                 'diagnostics': result.stderr.decode(errors='replace')[-1500:]})
            assert (result.returncode == 0) == ok, (args, result.stdout, result.stderr)
            assert b'secretMarker' not in result.stdout, (args, result.stdout)
            return result

        invoke('register', TOOL / 'dist/code.tar', stdin=b'[]\n[]\n[]\n[]\n[]\nunset\n')

        def symbols(*args, grants=None):
            result = invoke('code', 'outline', *args, '--format', 'json',
                            '--', '--read', grants or src)
            data = json.loads(result.stdout)
            return {item['symbol'] for item in data['results']}

        # Do not place an unreadable symlink in successful traversal fixtures.
        (src / 'escape.ts').unlink()
        expected = {'alpha', 'testAlpha', 'beta', 'component'}
        assert symbols('--path', src) == expected
        assert symbols('--path', f'{src}/**/*.ts') == expected - {'component'}
        assert symbols('--path', f'glob:{src}/*.[jt]s') == {'alpha', 'testAlpha'}
        assert symbols('--path', f'{src}/**/*.{{ts,tsx}}', '--exclude', '**/*.test.ts') == expected - {'testAlpha'}
        assert symbols('--path', rf'regex:{src}::(^|/)b\.ts$') == {'beta'}
        assert symbols('--path', src, '--exclude', r'regex:(^|/)a(\.test)?\.ts$') == {'beta', 'component'}
        assert symbols('--path', src, '--exclude', 'nested') == {'alpha', 'testAlpha'}
        assert symbols('--path', f'{src}/**/*.rb') == set()
        assert symbols('--path', src / 'a.ts') == {'alpha'}
        assert symbols('--path', src, '--path', lib, grants=project) == expected | {'extra'}
        assert symbols('--path', src / 'node_modules', grants=project) == {'dependency'}
        assert symbols('--path', src, '--path', src / 'node_modules', '--exclude', 'node_modules', grants=project) == expected
        # Invalid selectors and denied reads must fail, without leaking content.
        invoke('code', 'outline', '--path', f'regex:{src}::[', '--', '--read', src, ok=False)
        invoke('code', 'outline', '--path', f'{base}/**/*.ts', '--', '--read', src / 'a.ts', ok=False)
        invoke('code', 'outline', '--path', secret, ok=False)
        (src / 'escape.ts').symlink_to(secret)
        invoke('code', 'outline', '--path', src / 'escape.ts', '--', '--read', src, ok=False)
        assert secret.read_text() == 'function secretMarker() {}\n'
        print(json.dumps({'sat_calls': calls, 'expected_denials': failures}, indent=2))
        print('code: selector, ignore-rule and confinement checks passed')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--sat', type=Path, default=ROOT / 'sat/target/debug/sat')
    check(parser.parse_args().sat.resolve())
