"""Compare SAT pdf-text with a pinned, popular upstream; do not claim equivalent engines."""
import argparse
from collections import Counter
from datetime import datetime, timezone
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import platform
import statistics
import subprocess
import sys
import time

UPSTREAM_COMMIT = 'ed178952a6672abdfd35b5f9e76a80b6cefc086e'
UPSTREAM_URL = 'https://github.com/microsoft/markitdown'


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--upstream', type=Path, required=True)
    parser.add_argument('--python', type=Path, default=Path(sys.executable))
    parser.add_argument('--sat', type=Path, default=Path.home() / '.codex/skills/sat/scripts/run_sat.py')
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--runs', type=int, default=3)
    parser.add_argument('--require-parity', action='store_true')
    args = parser.parse_args()
    if not 1 <= args.runs <= 20:
        parser.error('--runs must be between 1 and 20')
    upstream, sat, output = [p.resolve() for p in (args.upstream, args.sat, args.output)]
    # Resolving a venv interpreter symlink loses the virtual environment.
    python = args.python.absolute()
    output.mkdir(parents=True, exist_ok=True)
    commit = subprocess.check_output(['git', '-C', str(upstream), 'rev-parse', 'HEAD'], text=True).strip()
    if commit != UPSTREAM_COMMIT:
        raise SystemExit(f'Expected pinned upstream {UPSTREAM_COMMIT}, got {commit}')
    subprocess.run(['git', '-C', str(upstream), 'diff', '--exit-code', '--quiet',
                    'HEAD', '--', 'packages/markitdown'], check=True)
    package = upstream / 'packages/markitdown'
    test_file = package / 'tests/test_pdf.py'
    fixture_dir = package / 'tests/test_files'
    plugin = Path(__file__).resolve().parent
    env = dict(os.environ, GITHUB_ACTIONS='1', SAT_PDF_BENCH_SAT=str(sat),
               PYTHONPATH=str(plugin) + os.pathsep + os.environ.get('PYTHONPATH', ''))
    result = {
        'created_utc': datetime.now(timezone.utc).isoformat(),
        'upstream': {'url': UPSTREAM_URL, 'commit': commit, 'test_sha256': digest(test_file)},
        'selection': {'criterion': 'GitHub stars among reviewed PDF-capable CLI projects',
                      'checked_date': '2026-11-14', 'github_displayed_stars': {
                          'microsoft/markitdown': '195.3k', 'opendatalab/MinerU': '83,642',
                          'docling-project/docling': '70.6k', 'datalab-to/marker': '40.9k',
                          'pymupdf/PyMuPDF': '11.1k', 'pdfcpu/pdfcpu': '8.9k'}},
        'environment': {'platform': platform.platform(), 'python': sys.version, 'runs': args.runs},
        'methodology': [
            'Different engines: native MarkItDown/pdfminer/pdfplumber vs SAT WASI lopdf; not a WASI overhead measurement.',
            'SAT is registered/compiled before timing; registration time is separate.',
            'Timing includes full CLI process startup. SAT timings also include the required Python logging wrapper.',
            'One untimed warmup per backend/file, followed by repeated sequential invocations; OS caches are not cleared.',
            'No upstream assertions are changed. The SAT adapter only joins JSON page text, rejecting truncated output.',
            'MarkItDown internal implementation tests and remote conversions are explicitly skipped for SAT.',
            'Plain-text extraction is not Markdown CLI compatibility; successful execution alone is not conformance.',
        ],
        'suites': {}, 'files': [],
    }
    freeze = subprocess.check_output([str(python), '-m', 'pip', 'freeze'], text=True)
    (output / 'native-requirements.txt').write_text(freeze)
    for backend in ['native', 'sat']:
        print(f'Running unchanged upstream PDF assertions: {backend}', flush=True)
        report = output / f'upstream-{backend}.json'
        command = [str(python), '-m', 'pytest', str(test_file), '-q', '-p', 'markitdown_pytest_plugin', '--tb=short']
        proc = subprocess.run(command, cwd=package, env=dict(env, SAT_PDF_BENCH_BACKEND=backend,
                              SAT_PDF_BENCH_RESULT=str(report)), capture_output=True, text=True, timeout=900)
        (output / f'upstream-{backend}.txt').write_text(proc.stdout + proc.stderr)
        if not report.exists():
            raise RuntimeError(f'{backend} upstream suite could not execute: {proc.stdout}\n{proc.stderr}')
        suite = json.loads(report.read_text())
        suite['counts'] = dict(Counter(t['outcome'] for t in suite['tests'].values()))
        result['suites'][backend] = suite
        print(f'{backend}: {suite["counts"]}', flush=True)

    native_tests = result['suites']['native']['tests']
    if digest(test_file) != result['upstream']['test_sha256']:
        raise RuntimeError('Upstream assertions changed during the comparison')
    comparable = {name: test for name, test in result['suites']['sat']['tests'].items() if test['outcome'] != 'skipped'}
    result['matched_conversion_tests'] = {
        'total': len(comparable),
        'native_passed': sum(native_tests.get(name, {}).get('outcome') == 'passed' for name in comparable),
        'sat_passed': sum(test['outcome'] == 'passed' for test in comparable.values()),
    }
    # Snapshot the two actual CLI interfaces as evidence; do not translate flags
    # and then call that upstream CLI compatibility.
    for label, command in [('native', [str(python), '-m', 'markitdown', '--help']), ('sat', [str(sat), 'pdf-text', '--help'])]:
        help_result = subprocess.run(command, capture_output=True, text=True, timeout=60)
        (output / f'{label}-help.txt').write_text(help_result.stdout + help_result.stderr)

    spec = importlib.util.spec_from_file_location('pdf_upstream_vectors', package / 'tests/_test_vectors.py')
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    vectors = {v.filename: v for v in module.GENERAL_TEST_VECTORS if v.filename.endswith('.pdf')}

    # Use a private registry so compilation/cache state is recorded explicitly.
    registry = output / 'registry'
    tool_dir = Path(__file__).resolve().parent
    archive = tool_dir / 'dist/pdf-text.tar'
    # This file can also run from the experiments staging area before installation.
    if not archive.exists():
        archive = Path('/home/tobias/programming/github/sandboxed-agent-tools/external_tools/pdf-text/dist/pdf-text.tar')
    timing_env = dict(os.environ, SAT_HOME=str(registry))
    result['registration_cache_preexisting'] = (registry / '.cache').exists()
    start = time.perf_counter()
    registration = subprocess.run([str(sat), 'register', '--force', str(archive)], cwd=output,
        env=timing_env, input=b'[]\n' * 5 + b'unset\n', capture_output=True, timeout=300)
    result['registration_ms'] = (time.perf_counter() - start) * 1000
    if registration.returncode:
        raise RuntimeError(registration.stderr.decode())
    result['sat_archive_sha256'] = digest(archive)
    for path in sorted(fixture_dir.glob('*.pdf')):
        print(f'Comparing CLI output and timing: {path.name}', flush=True)
        row = {'file': path.name, 'sha256': digest(path), 'bytes': path.stat().st_size}
        outputs = {}
        for backend, command in [
            ('native', [str(python), '-m', 'markitdown', str(path)]),
            ('sat', [str(sat), 'pdf-text', str(path), '--format', 'json', '--max-pages', '200',
                     '--max-bytes', '1048576', '--', '--read', str(path)]),
        ]:
            samples = []
            returns = []
            for iteration in range(args.runs + 1):
                started = time.perf_counter()
                proc = subprocess.run(command, cwd=output, env=timing_env if backend == 'sat' else os.environ,
                                      capture_output=True, timeout=180)
                elapsed = (time.perf_counter() - started) * 1000
                if iteration:
                    samples.append(elapsed)
                    returns.append(proc.returncode)
            stdout = proc.stdout.decode('utf-8', errors='replace')
            stderr = proc.stderr.decode('utf-8', errors='replace')
            (output / f'{path.stem}.{backend}.stdout').write_text(stdout)
            (output / f'{path.stem}.{backend}.stderr').write_text(stderr)
            text, truncated = stdout, False
            if backend == 'sat' and proc.returncode == 0:
                data = json.loads(stdout)
                text = ''.join(p['text'] for p in data['pages'])
                truncated = data['truncated']
            vector = vectors.get(path.name)
            missing = [s for s in vector.must_include if s not in text] if vector else []
            forbidden = [s for s in vector.must_not_include if s in text] if vector else []
            row[backend] = {'argv': command, 'exit_code': proc.returncode, 'measured_exit_codes': returns,
                'milliseconds': samples, 'median_ms': statistics.median(samples), 'truncated': truncated,
                'text_bytes': len(text.encode()), 'vector_missing': missing, 'vector_forbidden': forbidden,
                'vector_pass': (proc.returncode == 0 and not truncated and not missing and not forbidden) if vector else None}
            outputs[backend] = text
        row['exact_text_match'] = (row['native']['exit_code'] == row['sat']['exit_code'] == 0
                                   and not row['sat']['truncated'] and outputs['native'] == outputs['sat'])
        result['files'].append(row)
        (output / 'results.json').write_text(json.dumps(result, indent=2))
    print(json.dumps({'matched_conversion_tests': result['matched_conversion_tests'],
        'files': len(result['files']), 'exact_text_matches': sum(row['exact_text_match'] for row in result['files']),
        'results': str(output / 'results.json')}, indent=2), flush=True)
    failures = result['matched_conversion_tests']['total'] - result['matched_conversion_tests']['sat_passed']
    return 1 if args.require_parity and (failures or result['suites']['native']['exit_code']) else 0


if __name__ == '__main__':
    raise SystemExit(main())
