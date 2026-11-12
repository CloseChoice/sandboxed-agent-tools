"""Compare the sandboxed CLI with the same upstream jaq source built natively."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[2]
TOOL = Path(__file__).resolve().parent
SAT = ROOT / 'sat/target/debug' / ('sat.exe' if os.name == 'nt' else 'sat')
NATIVE = TOOL / 'target/release' / ('jaq-native.exe' if os.name == 'nt' else 'jaq-native')


def check(build=True):
    if build:
        subprocess.run(['bash', str(TOOL / 'package.sh')], check=True)
        subprocess.run(['cargo', 'build', '--locked', '--manifest-path', str(ROOT / 'sat/Cargo.toml')], check=True)
        subprocess.run(['cargo', 'build', '--locked', '--release', '--manifest-path', str(TOOL / 'Cargo.toml'), '--bin', 'jaq-native'], check=True)
    with tempfile.TemporaryDirectory(prefix='sat-jaq-cli-') as temporary:
        base = Path(temporary)
        env = dict(os.environ, SAT_HOME=str(TOOL / 'target/test-registry'), SAT_TEST_SECRET='host-secret')
        def sat(args, data=b'', grants=()):
            return subprocess.run([str(SAT), 'jaq', *args, *(['--', *grants] if grants else [])],
                                  input=data, cwd=base, env=env, capture_output=True, timeout=180)
        registered = subprocess.run([str(SAT), 'register', '--force', str(TOOL / 'dist/jaq.tar')],
                                    input=b'y\n' * 6, cwd=base, env=env, capture_output=True, timeout=180)
        assert registered.returncode == 0, registered.stderr
        print('jaq: registered and compiled', flush=True)
        def parity(args, data=b'', grants=()):
            native = subprocess.run([str(NATIVE), *args], input=data, cwd=base,
                                    env={'PWD': str(base)}, capture_output=True, timeout=30)
            guest = sat(args, data, grants)
            assert guest.returncode == native.returncode, (args, guest, native)
            assert guest.stdout == native.stdout, (args, guest.stdout, native.stdout, guest.stderr)
            # Path-independent errors and help/version must match byte-for-byte too.
            assert guest.stderr == native.stderr, (args, guest.stderr, native.stderr)
            return guest
        for args, data in [
            (['--help'], b''), (['--version'], b''),
            (['.'], b'{"a":1}'), (['-r', '.name'], b'{"name":"Ada"}'),
            (['-nc', '[range(3)]'], b''),
            (['-nr', '--arg', 'name', 'Ada', '$name'], b''),
            (['-n', '--argjson', 'n', '42', '$n + 1'], b''),
            (['-Rsc', '.'], b'a\nb\n'), (['-j', '.[]'], b'["a","b"]'),
            (['--raw-output0', '.[]'], b'["a","b"]'),
            (['-Sc', '.'], b'{"b":1,"a":2}'),
            (['--indent', '4', '.'], b'{"a":1}'),
            (['-C', '.'], b'{"a":true}'),
            (['-n', '--args', '$ARGS.positional', 'a', 'b'], b''),
            (['--from', 'yaml', '-c', '.'], b'name: Ada\n'),
            (['--from', 'toml', '-c', '.'], b'name = "Ada"\n'),
            (['--to', 'yaml', '.'], b'{"name":"Ada"}'),
            (['--to', 'cbor', '.'], b'{"a":1}'),
            (['--from', 'cbor', '.'], b'\xa1\x61a\x01'),
            (['-e', 'false'], b'null'), (['-e', 'empty'], b'null'),
            (['Ada'], b'null'), (['.'], b'{invalid'),
            (['--unknown'], b''), (['halt_error(7)'], b'"failure"'),
            (['env'], b'null'),
        ]:
            parity(args, data)
        data_file = base / 'users.json'
        data_file.write_text('{"name":"Ada"}')
        file_grant = ['--read', str(data_file)]
        parity(['-r', '.name', 'users.json'], grants=file_grant)
        parity(['-r', '.name', str(data_file)], grants=file_grant)
        parity(['-c', '.', 'users.json', 'users.json'], grants=file_grant)
        program = base / 'filter.jq'; program.write_text('.name')
        parity(['-rf', 'filter.jq', 'users.json'], grants=['--read', str(base)])
        (base / 'mod.jq').write_text('def answer: 42;')
        parity(['-n', '-L', '.', 'include "mod"; answer'], grants=['--read', str(base)])
        parity(['-n', '--slurpfile', 'rows', 'users.json', '$rows'], grants=file_grant)
        parity(['-nr', '--rawfile', 'text', 'users.json', '$text'], grants=file_grant)
        dash_file = base / '-input.json'; dash_file.write_text('{"name":"Ada"}')
        parity(['-r', '.name', '--', '-input.json'], grants=['--read', str(dash_file)])
        # CLI compatibility must not grant filesystem authority.
        denied = sat(['.name', 'users.json'])
        assert denied.returncode != 0 and b'Ada' not in denied.stdout
        outside = base / 'secret.json'; outside.write_text('{"secret":"private-value"}')
        denied = sat(['.', str(outside)], grants=file_grant)
        assert denied.returncode != 0 and b'private-value' not in denied.stdout
        link = base / 'escape.json'
        if hasattr(os, 'symlink'):
            link.symlink_to(outside)
            denied = sat(['.', str(link)], grants=file_grant)
            assert denied.returncode != 0 and b'private-value' not in denied.stdout
        assert data_file.read_text() == '{"name":"Ada"}'
        readonly = sat(['-i', '.name = "Grace"', 'users.json'], grants=file_grant)
        assert readonly.returncode != 0 and data_file.read_text() == '{"name":"Ada"}'
        # Same CLI and content for atomic in-place updates under a directory grant.
        edited = sat(['-i', '.name = "Grace"', 'users.json'], grants=['--read', str(base), '--write', str(base)])
        assert edited.returncode == 0, edited.stderr
        assert edited.stdout == b'' and edited.stderr == b''
        assert 'Grace' in data_file.read_text()
        # Neither a sibling write nor a symlink may expand authority.
        denied = sat(['-i', '.secret = "changed"', str(outside)], grants=['--read', str(outside), '--write', str(data_file)])
        assert denied.returncode != 0 and outside.read_text() == '{"secret":"private-value"}'
        denied = sat(['-i', '.secret = "changed"', 'secret.json'], grants=['--read', str(base), '--write', str(base), '--deny-write', 'secret.json'])
        assert denied.returncode != 0 and outside.read_text() == '{"secret":"private-value"}'
        print('jaq: upstream CLI parity and confinement checks passed', flush=True)


if __name__ == '__main__':
    check(build='--skip-build' not in sys.argv)
