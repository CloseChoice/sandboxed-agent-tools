"""Adapt unchanged upstream PDF API/CLI tests to the SAT process boundary."""
import inspect
import json
import os
from pathlib import Path
import subprocess
from types import SimpleNamespace

import pytest

RESULTS = {}
REAL_RUN = subprocess.run


def sat_run(upstream_args, **kwargs):
    assert upstream_args[1:3] == ['-m', 'markitdown'], upstream_args
    args = list(upstream_args[3:])
    grants = []
    for arg in args:
        path = Path(arg)
        if path.suffix.lower() == '.pdf' and path.is_file():
            grants += ['--read', str(path.resolve())]
    if '-o' in args or '--output' in args:
        flag = '-o' if '-o' in args else '--output'
        output = Path(args[args.index(flag) + 1]).resolve()
        grants += ['--write', str(output.parent)]
    command = [os.environ['SAT_MARKITDOWN_BINARY'], 'markitdown', *args]
    if grants:
        command += ['--', *grants]
    return REAL_RUN(command, **kwargs)


class SatAdapter:
    def __init__(self, *args, **kwargs):
        if args or kwargs:
            raise NotImplementedError('Only the local PDF test profile is supported')

    def convert(self, source, **kwargs):
        if kwargs:
            raise NotImplementedError(f'Unsupported options: {sorted(kwargs)}')
        path = Path(source).resolve()
        result = REAL_RUN([os.environ['SAT_MARKITDOWN_BINARY'], 'markitdown', str(path),
                          '--', '--read', str(path)], capture_output=True, text=True, timeout=180)
        assert result.returncode == 0, f'SAT exited {result.returncode}: {result.stderr}'
        # Upstream CLI adds exactly one print newline beyond the API result.
        text = result.stdout[:-1] if result.stdout.endswith('\n') else result.stdout
        return SimpleNamespace(text_content=text, markdown=text)


def pytest_collection_modifyitems(config, items):
    selected, deselected = [], []
    backend = os.environ['SAT_MARKITDOWN_BACKEND']
    for item in items:
        if item.path.name == 'test_cli_vectors.py':
            vector = getattr(item, 'callspec', None)
            vector = vector.params.get('test_vector') if vector else None
            if vector is None or not vector.filename.endswith('.pdf'):
                deselected.append(item)
                continue
            if backend == 'sat':
                item.module.subprocess = SimpleNamespace(run=sat_run)
        elif backend == 'sat':
            body = inspect.getsource(item.obj)
            conversion = 'markitdown' in item.fixturenames or ('MarkItDown(' in body and '.convert(' in body)
            internal = 'pdf_activity' in item.fixturenames or 'cleanup' in item.name.lower()
            if not conversion or internal or 'remote' in item.name.lower():
                item.add_marker(pytest.mark.skip(reason='Internal implementation/lifecycle or remote API, not a local process conversion test'))
            else:
                item.module.MarkItDown = SatAdapter
        selected.append(item)
    items[:] = selected
    config.hook.pytest_deselected(items=deselected)


def pytest_runtest_logreport(report):
    if report.when == 'call' or (report.when in ('setup', 'teardown') and report.outcome != 'passed'):
        RESULTS[report.nodeid] = {'outcome': report.outcome, 'phase': report.when,
                                  'seconds': report.duration,
                                  'detail': str(report.longrepr) if report.outcome != 'passed' else ''}


def pytest_sessionfinish(session, exitstatus):
    Path(os.environ['SAT_MARKITDOWN_RESULT']).write_text(json.dumps(
        {'exit_code': int(exitstatus), 'tests': RESULTS}, indent=2))
