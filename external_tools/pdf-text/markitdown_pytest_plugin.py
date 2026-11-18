"""Run unchanged upstream PDF conversion assertions with a SAT result adapter."""
import inspect
import json
import os
from pathlib import Path
import subprocess
from types import SimpleNamespace

import pytest

RESULTS = {}


class SatAdapter:
    def __init__(self, *args, **kwargs):
        if args or kwargs:
            raise NotImplementedError('SAT pdf-text does not implement MarkItDown constructor options')

    def convert(self, source, **kwargs):
        if kwargs:
            raise NotImplementedError(f'SAT pdf-text does not implement conversion options: {sorted(kwargs)}')
        path = Path(source).resolve()
        command = [os.environ['SAT_PDF_BENCH_SAT'], 'pdf-text', str(path),
                   '--format', 'json', '--max-pages', '200', '--max-bytes', '1048576',
                   '--', '--read', str(path)]
        result = subprocess.run(command, capture_output=True, text=True, timeout=180)
        assert result.returncode == 0, f'SAT exited {result.returncode}: {result.stderr}'
        data = json.loads(result.stdout)
        assert not data['truncated'], 'SAT output is truncated; cannot count this as upstream conformance'
        # Presentation adaptation only: remove JSON/page headings, preserving all
        # extracted text. Never create Markdown, normalize whitespace or weaken assertions.
        text = ''.join(page['text'] for page in data['pages'])
        return SimpleNamespace(text_content=text, markdown=text)


def pytest_collection_modifyitems(config, items):
    if os.environ.get('SAT_PDF_BENCH_BACKEND') != 'sat':
        return
    for item in items:
        body = inspect.getsource(item.obj)
        conversion = 'markitdown' in item.fixturenames or ('MarkItDown(' in body and '.convert(' in body)
        internal = 'pdf_activity' in item.fixturenames or 'cleanup' in item.name.lower()
        remote = 'remote' in item.name.lower()
        if not conversion or internal or remote:
            item.add_marker(pytest.mark.skip(reason='MarkItDown internal unit/lifecycle or remote test; no equivalent local SAT conversion API'))
        else:
            item.module.MarkItDown = SatAdapter


def pytest_runtest_logreport(report):
    if report.when == 'call' or (report.when in ('setup', 'teardown') and report.outcome != 'passed'):
        RESULTS[report.nodeid] = {
            'outcome': report.outcome,
            'phase': report.when,
            'seconds': report.duration,
            'detail': str(report.longrepr) if report.outcome != 'passed' else '',
        }


def pytest_sessionfinish(session, exitstatus):
    destination = os.environ.get('SAT_PDF_BENCH_RESULT')
    if destination:
        Path(destination).write_text(json.dumps({'exit_code': int(exitstatus), 'tests': RESULTS}, indent=2))
