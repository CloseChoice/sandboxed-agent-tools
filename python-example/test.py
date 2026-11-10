"""Checks the real WASI interpreter through SAT, including sandbox boundaries."""
from pathlib import Path
import json
import socket
import subprocess
import tempfile
import time

from build import build


def main():
    sat, env = build()
    env["SAT_PYTHON_SECRET"] = "must-not-be-inherited"

    def invoke(*args, ok=True, cwd=None):
        p = subprocess.run([str(sat), "python", *args], env=env,
                           capture_output=True, text=True, timeout=60, cwd=cwd)
        assert (p.returncode == 0) == ok, (args, p.stdout, p.stderr)
        return p.stdout if ok else p.stderr

    def run(code, ok=True, *flags, cwd=None):
        return invoke("-c", code, *flags, ok=ok, cwd=cwd)

    help_text = subprocess.check_output([str(sat), "python", "--help"], env=env, text=True)
    assert "-c, --code" in help_text and "script" in help_text
    assert "required" in invoke(ok=False)
    assert "cannot be used" in invoke("missing.py", "-c", "print(1)", ok=False)
    assert run("").strip() == ""
    help_json = json.loads(subprocess.check_output([str(sat), "python", "--help", "--json"], env=env, text=True))
    assert help_json["tool"]["options"]["code"]["short"] == "c"
    assert run("print(1 + 2)").strip() == "3"
    assert run("import sys; print(sys.platform)").strip() == "wasi"
    assert run("import math, statistics, json; print(json.dumps([math.sqrt(81), statistics.mean([2,4])]))").strip() == "[9.0, 3]"
    assert run("print('out'); import sys; print('err', file=sys.stderr)").strip() == "out\nerr"
    assert "ValueError: example" in run("raise ValueError('example')", False)
    assert "SystemExit: 2" in run("raise SystemExit(2)", False)
    run("raise SystemExit(0)")
    started = time.monotonic()
    assert "tool exceeded 30 seconds" in run("while True: pass", False).lower()
    assert 29 <= time.monotonic() - started < 45, "CPU-bound guest deadline was not enforced"
    assert "tool exceeded 30 seconds" in run("import time; time.sleep(60)", False).lower()

    with tempfile.TemporaryDirectory(prefix="sat-python-test-") as temp:
        secret = Path(temp) / "secret.txt"
        secret.write_text("private host content")
        destination = Path(temp) / "should-not-exist.txt"
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            listener.listen()
            port = listener.getsockname()[1]
            code = f'''
import os, socket
assert "SAT_PYTHON_SECRET" not in os.environ
for path, mode in [({str(secret)!r}, "r"), ({str(destination)!r}, "w")]:
    try:
        open(path, mode)
    except OSError:
        pass
    else:
        raise AssertionError("host file access unexpectedly succeeded")
s = None
try:
    s = socket.socket()
    s.connect(("127.0.0.1", {port}))
except OSError:
    pass
else:
    raise AssertionError("network access unexpectedly succeeded")
finally:
    if s is not None:
        s.close()
print("sandbox checks passed")
'''
            assert run(code).strip() == "sandbox checks passed"
            assert not destination.exists()

        work = Path(temp) / "work"
        work.mkdir()
        script = work / "script.py"
        script.write_text('import sys\nprint(__name__, __file__, sys.argv)\n')
        invoke("script.py", ok=False, cwd=work)
        assert invoke("script.py", "--read", "script.py", cwd=work).strip() == "__main__ script.py ['script.py']"
        assert invoke("--read", "script.py", "script.py", cwd=work).strip() == "__main__ script.py ['script.py']"
        assert invoke("alias.py", "--read", "script.py::alias.py", cwd=work).strip() == "__main__ alias.py ['alias.py']"
        invoke("script.py", "--read", "script.py::alias.py", ok=False, cwd=work)
        run('open("alias.py", "w")', False, "--read", "script.py::alias.py", cwd=work)
        assert not (work / "alias.py").exists()
        script.write_bytes(b'# coding: latin-1\nprint("caf\xe9")\n')
        assert invoke("script.py", "--read", "script.py", cwd=work).strip() == "café"
        script.write_text('raise ValueError("from script")\n')
        error = invoke("script.py", "--read", "script.py", ok=False, cwd=work)
        assert 'script.py' in error and "ValueError: from script" in error
        source = work / "input.txt"
        source.write_text("input")
        script.write_text('print(open("input.txt").read())\n')
        invoke("script.py", "--read", "script.py", ok=False, cwd=work)
        assert invoke("script.py", "--read", ".", cwd=work).strip() == "input"
        sibling = work / "sibling.txt"
        sibling.write_text("private")
        target = work / "output.txt"
        target.write_text("old")
        assert run('print(open("input.txt").read())', True, "--read", "input.txt", cwd=work).strip() == "input"
        assert run('import os; print(os.stat("input.txt").st_size)', True, "--read", "input.txt", cwd=work).strip() == "5"
        for code in [
            'open("input.txt", "w").write("bad")',
            'open("sibling.txt").read()',
            'open("input.txt/../sibling.txt").read()',
            'import os; os.listdir(".")',
            'import os; os.unlink("input.txt")',
        ]:
            run(code, False, "--read", "input.txt", cwd=work)
        assert run('print(open("../secret.txt").read()); print(open("input.txt").read())',
                   True, "--read", "../secret.txt", "--read", "input.txt", cwd=work).strip() == "private host content\ninput"
        assert source.read_text() == "input"
        run('open("output.txt", "w").write("new")', True, "--read-write", "output.txt", cwd=work)
        run('open("output.txt", "a").write("!")', True, "--read-write", "output.txt", cwd=work)
        assert target.read_text() == "new!"
        for code in ['open("sibling.txt", "w")', 'open("new.txt", "w")',
                     'import os; os.rename("output.txt", "sibling.txt")']:
            run(code, False, "--read-write", "output.txt", cwd=work)
        assert sibling.read_text() == "private"
        assert run('print(open("renamed/input.txt").read())', True, "--read", ".::renamed", cwd=work).strip() == "input"
        run('open("renamed/blocked.txt", "w")', False, "--read", ".::renamed", cwd=work)
        run('open("renamed/../secret.txt").read()', False, "--read", ".::renamed", cwd=work)
        run('open("alias.txt", "w").write("aliased")', True, "--read-write", "output.txt::alias.txt", cwd=work)
        assert target.read_text() == "aliased" and not (work / "alias.txt").exists()
        run('open("renamed/created.txt", "w").write("directory alias")', True, "--read-write", ".::renamed", cwd=work)
        assert (work / "created.txt").read_text() == "directory alias"
        for grant in ["::alias", "input.txt::", "input.txt::alias::extra"]:
            run('print(1)', False, "--read", grant, cwd=work)
        run('print(1)', False, "--read", "input.txt::same", "--read", "sibling.txt::same", cwd=work)
        run('open("new.txt", "w")', False, "--read", ".", cwd=work)
        run('open("../secret.txt").read()', False, "--read", ".", cwd=work)
        run('import os; os.unlink("input.txt")', False, "--read", ".", cwd=work)
        assert run('print(open("../secret.txt").read()); open("new.txt", "w").write("created")',
                   True, "--read", "..", "--read-write", ".", cwd=work).strip() == "private host content"
        assert (work / "new.txt").read_text() == "created"
        run('open("../secret.txt", "w")', False, "--read", "..", "--read-write", ".", cwd=work)
        assert secret.read_text() == "private host content"
        assert run('print(open("../work/input.txt").read())', True,
                   "--read", "..", cwd=work).strip() == "input"
        assert run(f'print(open({str(source)!r}).read())', True, "--read", str(source), cwd=work).strip() == "input"
        run('print(1)', False, "--read", "missing", cwd=work)
        if __import__("os").name != "nt":
            (work / "escape").symlink_to(secret)
            run('open("escape").read()', False, "--read", ".", cwd=work)
            run('open("escape", "w")', False, "--read-write", ".", cwd=work)
        # A separator is optional; SAT grants and generated tool options coexist.
        assert run('print(open("input.txt").read())', True, "--read", "input.txt", "--", cwd=work).strip() == "input"

    start = time.monotonic()
    run("print('warm cache')")
    print(f"PASS: Python code/scripts/encoding/argument validation/imports/errors, -c help, CPU/host-call execution deadlines, file/directory grants and isolation, read-only enforcement, network/environment denial. Warm invocation: {time.monotonic()-start:.2f}s")


if __name__ == "__main__":
    main()
