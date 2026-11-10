"""One functional test per portable stdlib module, executed by actual WASI CPython.

The host only builds the component, supplies fixture grants, and checks the
result. Imports, operations, and assertions under test all execute inside Wasm.
"""
from pathlib import Path
import subprocess
import tempfile
import textwrap
import unittest

from build import build


class PortableStdlibTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temporary = tempfile.TemporaryDirectory(prefix="sat-stdlib-tests-")
        cls.addClassCleanup(cls.temporary.cleanup)
        root = Path(cls.temporary.name)
        cls.fixtures = root / "fixtures"
        cls.fixtures.mkdir()
        (cls.fixtures / "input.txt").write_bytes("hello 日本語\n".encode("utf-8"))
        (cls.fixtures / "output.txt").write_bytes(b"")
        cls.sat, cls.env = build(sat_home=root / "registry")

    def execute(self, code, *, write=False):
        program = (
            "import sys\nassert sys.platform == 'wasi', sys.platform\n"
            + textwrap.dedent(code)
            + "\nprint('PASS')\n"
        )
        command = [str(self.sat), "python", "-c", program,
                   "--read", f"{self.fixtures}::/data"]
        if write:
            command += ["--read-write", f"{self.fixtures / 'output.txt'}::/output.txt"]
        result = subprocess.run(command, env=self.env, text=True, encoding="utf-8",
                                capture_output=True, timeout=60)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(result.stdout.strip(), "PASS")

    def test_collections(self):
        self.execute("""
            from collections import Counter
            assert Counter([1, 2, 3, 3]) == {1: 1, 2: 1, 3: 2}
        """)

    def test_itertools(self):
        self.execute("""
            from itertools import islice, count
            assert list(islice(count(10), 3)) == [10, 11, 12]
        """)

    def test_functools(self):
        self.execute("""
            from functools import lru_cache
            @lru_cache(maxsize=4)
            def square(value):
                return value * value
            assert square(3) == square(3) == 9
            assert square.cache_info().hits == 1
        """)

    def test_pathlib(self):
        self.execute("""
            from pathlib import Path
            assert Path('/data/input.txt').read_text(encoding='utf-8') == 'hello 日本語\\n'
            assert sorted(p.name for p in Path('/data').glob('*.txt')) == ['input.txt', 'output.txt']
        """)

    def test_os(self):
        self.execute("""
            import os
            assert os.stat('/data/input.txt').st_size == 16
            assert os.path.basename('/data/input.txt') == 'input.txt'
            assert 'input.txt' in os.listdir('/data')
        """)

    def test_json(self):
        self.execute("""
            import json
            value = {'message': '日本語', 'items': [1, True, None]}
            assert json.loads(json.dumps(value, ensure_ascii=False)) == value
        """)

    def test_re(self):
        self.execute("""
            import re
            pattern = re.compile(r'(?P<name>[a-z]+)=(?P<value>\\d+)')
            assert [m.groupdict() for m in pattern.finditer('x=12 y=3')] == [
                {'name': 'x', 'value': '12'}, {'name': 'y', 'value': '3'}]
        """)

    def test_datetime(self):
        self.execute("""
            from datetime import datetime, timedelta
            date = datetime.strptime('2026-11-08', '%Y-%m-%d')
            assert (date + timedelta(days=1)).isoformat() == '2026-11-09T00:00:00'
        """)

    def test_math(self):
        self.execute("""
            import math
            assert math.isclose(math.sqrt(81), 9.0)
            assert math.factorial(5) == 120
        """)

    def test_statistics(self):
        self.execute("""
            from statistics import mean, median
            assert mean([1, 2, 3, 6]) == 3
            assert median([1, 2, 3, 6]) == 2.5
        """)

    def test_decimal(self):
        self.execute("""
            from decimal import Decimal, localcontext
            with localcontext() as context:
                context.prec = 10
                assert Decimal('0.1') + Decimal('0.2') == Decimal('0.3')
        """)

    def test_fractions(self):
        self.execute("""
            from fractions import Fraction
            assert Fraction(1, 3) + Fraction(1, 6) == Fraction(1, 2)
        """)

    def test_hashlib(self):
        self.execute("""
            from hashlib import sha256
            assert sha256(b'abc').hexdigest() == 'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad'
        """)

    def test_csv(self):
        self.execute("""
            import csv
            from io import StringIO
            rows = list(csv.DictReader(StringIO('name,value\\n"a,b",2\\n')))
            assert rows == [{'name': 'a,b', 'value': '2'}]
        """)

    def test_tomllib(self):
        self.execute("""
            import tomllib
            data = tomllib.loads('[package]\\nname = "sat"\\nenabled = true\\n')
            assert data == {'package': {'name': 'sat', 'enabled': True}}
        """)

    def test_argparse(self):
        self.execute("""
            import argparse
            parser = argparse.ArgumentParser(prog='sat-example')
            parser.add_argument('--count', type=int, default=1)
            assert parser.parse_args(['--count', '3']).count == 3
            assert '--count' in parser.format_help()
        """)

    def test_shutil(self):
        self.execute("""
            import shutil
            from pathlib import Path
            shutil.copyfile('/data/input.txt', '/output.txt')
            assert Path('/output.txt').read_text(encoding='utf-8') == 'hello 日本語\\n'
        """, write=True)

    def test_tarfile(self):
        self.execute("""
            import tarfile
            from io import BytesIO
            payload = b'hello from tar'
            archive = BytesIO()
            with tarfile.open(fileobj=archive, mode='w:gz') as output:
                member = tarfile.TarInfo('message.txt')
                member.size = len(payload)
                output.addfile(member, BytesIO(payload))
            archive.seek(0)
            with tarfile.open(fileobj=archive, mode='r:gz') as source:
                assert source.getnames() == ['message.txt']
                assert source.extractfile('message.txt').read() == payload
        """)

    def test_zipfile(self):
        self.execute("""
            import zipfile
            from io import BytesIO
            archive = BytesIO()
            with zipfile.ZipFile(archive, 'w', compression=zipfile.ZIP_DEFLATED) as output:
                output.writestr('message.txt', 'hello from zip')
            archive.seek(0)
            with zipfile.ZipFile(archive) as source:
                assert source.namelist() == ['message.txt']
                assert source.read('message.txt') == b'hello from zip'
        """)

    def test_sqlite3(self):
        self.execute("""
            import sqlite3
            connection = sqlite3.connect(':memory:')
            try:
                connection.execute('create table items (value integer)')
                connection.executemany('insert into items values (?)', [(10,), (20,), (12,)])
                assert connection.execute('select sum(value) from items').fetchone() == (42,)
            finally:
                connection.close()
        """)


if __name__ == '__main__':
    unittest.main(verbosity=2)
