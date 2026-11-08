"""Real WASI component tests, with native jq parity checks when jq is available."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[2]
PROFILE = os.environ.get("SAT_BUILD_PROFILE", "release")
SAT = ROOT / f"sat/target/{PROFILE}" / ("sat.exe" if os.name == "nt" else "sat")


def main():
    with tempfile.TemporaryDirectory(prefix="sat-jaq-") as temporary:
        base = Path(temporary)
        env = dict(os.environ, SAT_HOME=str(base / "registry"), SAT_TEST_SECRET="must-not-leak")

        def invoke(*args, input=None, ok=True):
            payload = input.encode() if isinstance(input, str) else input
            result = subprocess.run([str(SAT), *map(str, args)], cwd=base, env=env,
                                    input=payload, capture_output=True)
            assert (result.returncode == 0) == ok, (args, result.stdout, result.stderr)
            return result

        if "--skip-build" in sys.argv:
            invoke("install", ROOT / "sat/target/jaq.wasm", "--replace")
        else:
            subprocess.run(["bash", str(ROOT / "sat/jaq/build.sh")], cwd=ROOT, env=env, check=True)
        assert "jaq" in invoke("search", "JSON queries").stdout.decode()
        assert b"--stdin" in invoke("jaq", "--help", "--stdin", input=b"\xff").stdout
        wit = invoke("inspect", "jaq", "--wit").stdout.decode()
        assert "wasi:filesystem" in wit and "export run:" in wit
        assert "wasi:sockets" not in wit and "wasi:http" not in wit

        comparisons = [
            (".items[] | select(.active) | .name", '{"items":[{"name":"é","active":true},{"name":"no","active":false}]}', [], []),
            ("map(. * 2) | add", "[1,2,3]", [], []),
            ("reduce .[] as $x (0; . + $x)", "[1,2,3]", [], []),
            (".users |= map(.id += 1)", '{"users":[{"id":1}]}', [], []),
            ("sort_by(.x) | group_by(.x)", '[{"x":2},{"x":1},{"x":2}]', [], []),
            ("paths(scalars)", '{"a":[1,2]}', ["-c", "true"], ["-c"]),
            ('getpath(["a",0]), setpath(["b"];2)', '{"a":[1]}', [], []),
            ('.[] | @base64', '["hello","world"]', ["-r", "true"], ["-r"]),
            ('test("^a"), gsub("a";"b")', '"abc"', [], []),
            ('try error("test") catch .', "null", [], []),
            ('def twice: . * 2; twice', "3", [], []),
            (r'"hello \($name)"', "null", ["--arg=name=world"], ["--arg", "name", "world"]),
            ('$x | add', "null", ["--argjson=x=[1,2]"], ["--argjson", "x", "[1,2]"]),
            (".", '{"z":1,"a":{"c":3,"b":2}}', ["-S", "true"], ["-S"]),
            (".", '{"a":[1,2]}', ["-c", "true"], ["-c"]),
            (".", '{"a":[1,2]}', ["--indent=4"], ["--indent", "4"]),
            (".", '{"a":[1,2]}', ["--tab=true"], ["--tab"]),
            (".", '"日本語 🐱"', ["-a", "true"], ["-a"]),
            (".[]", '["a","b"]', ["-j", "true"], ["-j"]),
            (".[]", '["a","b"]', ["--raw-output0=true"], ["--raw-output0"]),
            (".", "a\r\nb\n", ["-R", "true"], ["-R"]),
            (".", "a\nb\n", ["-R", "true", "-s", "true"], ["-R", "-s"]),
            ("add", "1 2 3", ["-s", "true"], ["-s"]),
            ("[., inputs]", "1 2 3", [], []),
            ("[inputs]", "1 2 3", ["-n", "true"], ["-n"]),
            ("empty", "null", [], []),
            ("halt", "null", [], []),
            ("nan", "null", [], []),
        ]
        parity = 0
        for filter, text, flags, jqflags in comparisons:
            result = invoke("jaq", filter, "--stdin", *flags, input=text)
            if shutil.which("jq"):
                jq = subprocess.run(["jq", *jqflags, filter], input=text.encode(), capture_output=True)
                # jq versions before 1.7 lack --raw-output0.
                if jq.returncode and "--raw-output0" in jqflags:
                    assert result.stdout == b"a\0b\0"
                    continue
                assert jq.returncode == 0, jq.stderr
                assert result.stdout == jq.stdout, (filter, flags, result.stdout, jq.stdout)
                parity += 1

        assert invoke("jaq", "range(3)", "-n", "true").stdout == b"0\n1\n2\n"
        assert invoke("jaq", "--input=42").stdout == b"42\n"
        assert invoke("jaq", "$ARGS", "-n", "true", "--args=one", "--args=two", "--arg=x=yes", "-c", "true").stdout == b'{"named":{"x":"yes"},"positional":["one","two"]}\n'
        assert json.loads(invoke("jaq", "$ARGS.positional", "-n", "true", "--jsonargs=1", "--jsonargs=true").stdout) == [1, True]
        assert b"must-not-leak" not in invoke("jaq", "env, $ENV", "-n", "true").stdout

        inputs = base / "inputs"
        inputs.mkdir()
        (inputs / "one.json").write_text('1\n2\n')
        (inputs / "two.json").write_text('3\n')
        (inputs / "message.txt").write_text('hello\n')
        (inputs / "filter.jq").write_text('map(. * 2)')
        secret = base / "secret.json"
        secret.write_text('"SECRET_CONTENT"')
        grant = f"--read={inputs}::/data"
        assert invoke("jaq", "[.,inputs]", grant, "--files=/data/one.json", "--files=/data/two.json", "-c", "true").stdout == b"[1,2,3]\n"
        assert invoke("jaq", "input_filename", grant, "--files=/data/one.json", "--files=/data/two.json").stdout == b'"/data/one.json"\n"/data/one.json"\n"/data/two.json"\n'
        assert invoke("jaq", "$data|add", "-n", "true", grant, "--slurpfile=data=/data/one.json").stdout == b"3\n"
        assert invoke("jaq", "$text", "-n", "true", grant, "--rawfile=text=/data/message.txt", "-r", "true").stdout == b"hello\n\n"
        assert invoke("jaq", grant, "--from-file=/data/filter.jq", "--input=[1,2]", "-c", "true").stdout == b"[2,4]\n"
        assert invoke("jaq", f"--read={inputs / 'one.json'}::/selected.json", "--files=/selected.json").stdout == b"1\n2\n"
        for args in [
            ["--files=/data/one.json"],
            [grant, "--files=/data/../secret.json"],
            [grant, f"--files={secret}"],
            ["-n", "true", grant, "--rawfile=text=/data/../secret.json"],
        ]:
            denied = invoke("jaq", *args, ok=False)
            assert str(base).encode() not in denied.stdout + denied.stderr
            assert b"SECRET_CONTENT" not in denied.stdout + denied.stderr
        if os.name != "nt":
            (inputs / "escape.json").symlink_to(secret)
            invoke("jaq", grant, "--files=/data/escape.json", ok=False)
        invoke("jaq", f"--read={inputs / 'one.json'}::/selected.json", "--files=/two.json", ok=False)
        invoke("jaq", 'include "/data/filter"; .', grant, "--input=1", ok=False)
        invoke("jaq", "--stdin", "--input=1", input="2", ok=False)
        invoke("jaq", "--stdin", input=b"\xff", ok=False)
        overflow = invoke("jaq", "--stdin", input=b" " * (8 * 1024 * 1024 + 1), ok=False)
        assert b"stdin exceeds 8 MiB" in overflow.stderr
        invoke("jaq", "--input=not-json", ok=False)
        invoke("jaq", "[", "--input=1", ok=False)
        invoke("jaq", "--input=1", "--files=/data/one.json", grant, ok=False)
        invoke("jaq", "--input=1", "--indent=9", ok=False)
        invoke("jaq", "--input=1", "--arg=bad-name=value", ok=False)
        invoke("jaq", "--input=1", "--argjson=x=not-json", ok=False)
        invoke("jaq", "--input=1", "--argjson=x=1 2", ok=False)
        invoke("jaq", r'"a\u0000b"', "-n", "true", "--raw-output0=true", ok=False)
        invoke("jaq", "false", "-n", "true", "-e", "true", ok=False)
        invoke("jaq", "empty", "-n", "true", "-e", "true", ok=False)
        assert invoke("jaq", "true", "-n", "true", "-e", "true").stdout == b"true\n"

        # Package metadata cannot point the stdin bridge at a bool, a positional
        # parameter, or a parameter with a default.
        original = (ROOT / "sat/jaq/tool.toml").read_text()
        guest = ROOT / "sat/jaq/target/wasm32-wasip2/release/sat_jaq_guest.wasm"
        for replacement in ['stdin = "raw-input"', 'stdin = "filter"', 'stdin = "missing"']:
            metadata = base / "invalid.toml"
            metadata.write_text(original.replace('stdin = "input"', replacement))
            invoke("pack", guest, "--metadata", metadata, "-o", base / "invalid.wasm", ok=False)
        print(f"SAT jaq: {parity} native jq parity cases; streams, variables, files, stdin bounds, exact output, metadata validation, and WASI confinement passed")


if __name__ == "__main__":
    main()
