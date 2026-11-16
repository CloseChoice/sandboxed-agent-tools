"""Check native/WASI CLI behavior and sandbox boundaries using a small PDF."""
import argparse
import os
import re
from pathlib import Path
import subprocess
import tempfile



def without_onnx_pci_warning(stderr):
    # Native ONNX imports inspect runner hardware, even for --help. This specific
    # diagnostic is not emitted by MarkItDown's CLI and has no WASI counterpart.
    # Keep every other byte so actual CLI errors and other warnings still fail.
    warning = re.compile(
        rb'(?:\x1b\[[0-9;]*m)?[0-9:. -]+ '
        rb'\[W:onnxruntime:Default, device_discovery\.cc:\d+ GetPciBusId\] '
        rb'Skipping pci_bus_id for PCI path at "[^"\r\n]+" because filename '
        rb'"[^"\r\n]+" did not match expected pattern of [^\r\n]+'
        rb'(?:\x1b\[m)?\r?\n?'
    )
    return b''.join(line for line in stderr.splitlines(keepends=True)
                    if not warning.fullmatch(line))


def small_pdf():
    content = b'BT /F1 12 Tf 72 720 Td (Hello from MarkItDown) Tj ET'
    objects = [b'<< /Type /Catalog /Pages 2 0 R >>',
        b'<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
        b'<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>',
        b'<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>',
        b'<< /Length ' + str(len(content)).encode() + b' >>\nstream\n' + content + b'\nendstream']
    data = bytearray(b'%PDF-1.4\n')
    offsets = [0]
    for n, obj in enumerate(objects, 1):
        offsets.append(len(data))
        data.extend(f'{n} 0 obj\n'.encode() + obj + b'\nendobj\n')
    xref = len(data)
    data.extend(b'xref\n0 6\n0000000000 65535 f \n')
    for offset in offsets[1:]:
        data.extend(f'{offset:010d} 00000 n \n'.encode())
    data.extend(f'trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n'.encode())
    return data


def check(python, sat):
    calls = 0
    with tempfile.TemporaryDirectory(prefix='sat-markitdown-cli-') as temporary:
        base = Path(temporary)
        source = base / 'sample.pdf'
        source.write_bytes(small_pdf())
        env = dict(os.environ, SAT_MARKITDOWN_SECRET='unrelated-environment-secret')

        def invoke(args, *, backend='sat', grants=(), data=None):
            nonlocal calls
            command = [str(python), '-m', 'markitdown', *map(str, args)] if backend == 'native' else [str(sat), 'markitdown', *map(str, args)]
            if backend == 'sat' and grants:
                command += ['--', *map(str, grants)]
            p = subprocess.run(command, cwd=base, env=env, input=data, capture_output=True, timeout=180)
            assert b'unrelated-environment-secret' not in p.stdout + p.stderr
            calls += 1
            return p

        for args in [['--help'], ['--version'], ['-m', 'invalid'], ['-c', 'not-a-codec'], ['--unknown'], ['--list-plugins']]:
            native, wasi = invoke(args, backend='native', data=b''), invoke(args, data=b'')
            assert native.returncode == wasi.returncode, (args, native, wasi)
            if args == ['--help']:
                # argparse's metavar layout changed between Python 3.12/3.14.
                # Compare the public options and syntax; do not claim byte parity.
                flags = lambda text: set(re.findall(rb'(?<!\w)--?[A-Za-z][A-Za-z0-9_-]*', text))
                assert flags(native.stdout) == flags(wasi.stdout)
                assert b'markitdown <OPTIONAL: FILENAME>' in wasi.stdout
                if native.stdout != wasi.stdout:
                    print('Help layout differs across Python 3.12/3.14; option names and usage match')
            else:
                assert native.stdout == wasi.stdout, (args, native.stdout, wasi.stdout)
            # The logger appends an instrumentation report after child stderr.
            child_stderr = wasi.stderr.split(b'SAT FAILED (exit ', 1)[0]
            assert without_onnx_pci_warning(native.stderr) == child_stderr, (args, native.stderr, child_stderr)
        for args, data, grants in [([source], None, ['--read', source]),
                                   (['sample.pdf'], None, ['--read', source]),
                                   ([], source.read_bytes(), []),
                                   (['-x', 'pdf'], source.read_bytes(), []),
                                   (['-m', 'application/pdf', '-c', 'utf-8', source], None, ['--read', source])]:
            native = invoke(args, backend='native', data=data)
            wasi = invoke(args, data=data, grants=grants)
            assert native.returncode == wasi.returncode == 0, (args, native.stderr, wasi.stderr)
            assert native.stdout == wasi.stdout and b'Hello from MarkItDown' in wasi.stdout
        native_output = base / 'native.md'
        wasi_output = base / 'wasi.md'
        wasi_output.touch()
        assert invoke([source, '-o', native_output], backend='native').returncode == 0
        p = invoke([source, '-o', wasi_output], grants=['--read', source, '--write', wasi_output])
        assert p.returncode == 0 and p.stdout == b'', p.stderr
        assert wasi_output.read_bytes() == native_output.read_bytes()
        assert invoke([source]).returncode != 0
        saved = wasi_output.read_bytes()
        assert invoke([source, '-o', wasi_output], grants=['--read', source]).returncode != 0
        assert wasi_output.read_bytes() == saved
        secret = base / 'secret.pdf'
        secret.write_bytes(source.read_bytes())
        assert invoke([secret], grants=['--read', source]).returncode != 0
        if os.name != 'nt':
            link = base / 'escape.pdf'
            link.symlink_to(secret)
            assert invoke([link], grants=['--read', source]).returncode != 0
        assert source.read_bytes() == small_pdf()
    print(f'markitdown: {calls} native/WASI CLI and confinement checks passed')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--python', type=Path, required=True)
    parser.add_argument('--sat', type=Path, default=Path.home() / '.codex/skills/sat/scripts/run_sat.py')
    args = parser.parse_args()
    check(args.python.absolute(), args.sat.resolve())
