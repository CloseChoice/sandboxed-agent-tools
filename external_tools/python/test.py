"""Exercise the real component, including host-enforced capability boundaries."""
import argparse
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parent
LOGGER = Path.home() / ".codex/skills/sat/scripts/run_sat.py"


def check(sat):
    with tempfile.TemporaryDirectory(prefix="sat-python-") as temporary:
        project = Path(temporary)
        env = dict(os.environ, SAT_HOME=str(project / "registry"), SAT_PYTHON_SECRET="must-not-leak", SAT_BINARY=str(sat))

        def invoke(*args, ok=True, data=b""):
            prefix = [sys.executable, str(LOGGER), "--"] if LOGGER.exists() else [str(sat)]
            result = subprocess.run([*prefix, *map(str, args)], cwd=project, env=env,
                                    input=data, capture_output=True, timeout=180)
            assert (result.returncode == 0) == ok, (args, result.stdout, result.stderr)
            assert b"must-not-leak" not in result.stdout + result.stderr
            return result.stdout.decode()

        def run(code, *grants, ok=True):
            return invoke("python", "-c", code, *(["--", *grants] if grants else []), ok=ok)

        invoke("register", ROOT / "dist/python.tar", data=b"[]\n" * 5 + b"unset\n")
        assert "code" in invoke("python", "--help")
        invoke("python", ok=False)
        invoke("python", "script.py", "-c", "pass", ok=False)
        assert run("print(1 + 2)").strip() == "3"
        assert run("import sys; print(sys.platform)").strip() == "wasi"
        assert run("import sys; print(sys.version_info[:2])").strip() == "(3, 14)"
        run("raise ValueError('expected')", ok=False)
        run("raise SystemExit(2)", ok=False)
        run("raise SystemExit(0)")
        assert "True" in run("import os; print('SAT_PYTHON_SECRET' not in os.environ)")
        assert json.loads(invoke("python", "-c", "print('hello')", "--json")) == "hello\n"
        assert "stdlib passed" in run('''
import collections, csv, datetime, decimal, fractions, hashlib, io, json
import math, pathlib, re, sqlite3, statistics, tarfile, tomllib, zipfile
assert collections.Counter('aba')['a'] == 2
assert next(csv.reader(['"a,b",c'])) == ['a,b', 'c']
assert datetime.datetime.strptime('2026-11-14', '%Y-%m-%d').day == 14
assert decimal.Decimal('0.1') + decimal.Decimal('0.2') == decimal.Decimal('0.3')
assert fractions.Fraction(1, 2) + fractions.Fraction(1, 3) == fractions.Fraction(5, 6)
assert hashlib.sha256(b'abc').hexdigest().startswith('ba7816bf')
assert json.loads(json.dumps({'hello': 'world'}))['hello'] == 'world'
assert math.sqrt(81) == 9 and statistics.mean([2, 4]) == 3
assert re.findall(r'\\d+', 'x12') == ['12']
assert tomllib.loads('n = 3')['n'] == 3
db = sqlite3.connect(':memory:')
assert db.execute('select 1 + 2').fetchone()[0] == 3
for archive in ('zip', 'tar'):
    data = io.BytesIO()
    if archive == 'zip':
        with zipfile.ZipFile(data, 'w', compression=zipfile.ZIP_DEFLATED) as z:
            z.writestr('hello.txt', 'hello')
        data.seek(0)
        with zipfile.ZipFile(data) as z:
            assert z.read('hello.txt') == b'hello'
    else:
        with tarfile.open(fileobj=data, mode='w:gz') as t:
            entry = tarfile.TarInfo('hello.txt')
            entry.size = 5
            t.addfile(entry, io.BytesIO(b'hello'))
        data.seek(0)
        with tarfile.open(fileobj=data, mode='r:gz') as t:
            assert t.extractfile('hello.txt').read() == b'hello'
print('stdlib passed')
''')
        source = project / "input.txt"
        source.write_text("granted input")
        outside = project / "secret.txt"
        outside.write_text("ungranted input")
        destination = project / "output.txt"
        run(f"open({str(source)!r}).read()", ok=False)
        assert "granted input" in run(f"print(open({str(source)!r}).read())", "--read", source)
        run(f"open({str(outside)!r}).read()", "--read", source, ok=False)
        run(f"open({str(source)!r}, 'w').write('overwrite')", "--read", source, ok=False)
        run(f"open({str(destination)!r}, 'w').write('denied')", ok=False)
        destination.touch()
        run(f"open({str(destination)!r}, 'w').write('allowed')", "--write", destination)
        assert destination.read_text() == "allowed" and source.read_text() == "granted input"
        script = project / "script.py"
        script.write_text("print(__name__)\n")
        invoke("python", script, ok=False)
        assert "__main__" in invoke("python", script, "--", "--read", script)
        if os.name != "nt":
            link = project / "escape.txt"
            link.symlink_to(outside)
            run(f"open({str(link)!r}).read()", "--read", source, ok=False)
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            listener.listen()
            run(f"import socket; socket.create_connection(('127.0.0.1', {listener.getsockname()[1]}))", ok=False)
        subprocess.run([sys.executable, str(ROOT / "test_xml.py"), "--sat", str(sat)],
                       env=env, check=True)
    print("python: execution, stdlib/XML, file grants, read-only access, environment and network isolation passed")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sat", type=Path, default=ROOT.parents[1] / "sat/target/debug/sat")
    parser.add_argument("--skip-build", action="store_true")
    args = parser.parse_args()
    if not args.skip_build:
        subprocess.run(["bash", str(ROOT / "package.sh")], check=True)
    check(args.sat.resolve())
