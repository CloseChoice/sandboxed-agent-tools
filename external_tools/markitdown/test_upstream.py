"""Run the original pinned PDF and CLI PDF tests without changing assertions."""
import argparse
from collections import Counter
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys

COMMIT = 'ed178952a6672abdfd35b5f9e76a80b6cefc086e'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--upstream', required=True, type=Path)
    parser.add_argument('--python', type=Path, default=Path(sys.executable))
    parser.add_argument('--sat', type=Path, default=Path.home() / '.codex/skills/sat/scripts/run_sat.py')
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    root = args.upstream.resolve()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    assert subprocess.check_output(['git', '-C', str(root), 'rev-parse', 'HEAD'], text=True).strip() == COMMIT
    subprocess.run(['git', '-C', str(root), 'diff', '--exit-code', '--quiet', 'HEAD', '--', 'packages/markitdown'], check=True)
    package = root / 'packages/markitdown'
    files = [package / 'tests/test_pdf.py', package / 'tests/test_cli_vectors.py']
    hashes = {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in files}
    env = dict(os.environ, GITHUB_ACTIONS='1', SAT_MARKITDOWN_BINARY=str(args.sat.resolve()),
               PYTHONPATH=str(Path(__file__).resolve().parent))
    summaries = {}
    for backend in ['native', 'sat']:
        report = output / f'{backend}.json'
        command = [str(args.python.absolute()), '-m', 'pytest', *map(str, files), '-q',
                   '-p', 'upstream_pytest_plugin', '--tb=short']
        result = subprocess.run(command, cwd=package, env=dict(env, SAT_MARKITDOWN_BACKEND=backend,
            SAT_MARKITDOWN_RESULT=str(report)), capture_output=True, text=True, timeout=900)
        (output / f'{backend}.txt').write_text(result.stdout + result.stderr)
        if not report.exists():
            raise RuntimeError(result.stdout + result.stderr)
        report_data = json.loads(report.read_text())
        counts = dict(Counter(t['outcome'] for t in report_data['tests'].values()))
        summaries[backend] = {'exit_code': result.returncode, 'counts': counts}
        print(backend, summaries[backend], flush=True)
    assert hashes == {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in files}
    (output / 'summary.json').write_text(json.dumps({'commit': COMMIT, 'test_sha256': hashes,
        'suites': summaries, 'scope': 'Original PDF module plus original CLI vectors filtered to PDF inputs; explicit internal/remote skips'}, indent=2))
    return 1 if any(s['exit_code'] for s in summaries.values()) else 0


if __name__ == '__main__':
    raise SystemExit(main())
