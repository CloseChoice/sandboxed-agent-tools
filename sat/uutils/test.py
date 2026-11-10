"""Exercise upstream uutils inside SAT, including filesystem boundaries."""
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
    with tempfile.TemporaryDirectory(prefix="sat-uutils-") as temporary:
        base = Path(temporary)
        env = dict(os.environ, SAT_HOME=str(base / "registry"), SAT_TEST_SECRET="not-visible")

        def invoke(*args, input=None, ok=True):
            result = subprocess.run([str(SAT), *map(str, args)], cwd=base, env=env,
                                    input=input, capture_output=True, timeout=45)
            assert (result.returncode == 0) == ok, (args, result.stdout, result.stderr)
            assert str(base).encode() not in result.stdout + result.stderr, result
            return result

        if "--skip-build" in sys.argv:
            invoke("install", ROOT / "sat/target/uutils.wasm", "--replace")
        else:
            subprocess.run(["bash", str(ROOT / "sat/uutils/build.sh")], cwd=ROOT, env=env, check=True)
        metadata = (ROOT / "sat/uutils/tool.toml").read_text()
        invalid = base / "invalid.toml"
        invalid.write_text(metadata.replace("command_output = true", "command_output = true\nverbatim_output = true"))
        result = invoke("pack", ROOT / "sat/uutils/target/wasm32-wasip2/release/sat_uutils_guest.wasm",
                        "--metadata", invalid, "--output", base / "invalid.wasm", ok=False)
        assert b"conflicts with verbatim_output" in result.stderr
        invalid.write_text(metadata.replace('stdin = "input"', '').replace(
            '[options.input]', '[[arguments]]\nname = "input"'))
        result = invoke("pack", ROOT / "sat/uutils/target/wasm32-wasip2/release/sat_uutils_guest.wasm",
                        "--metadata", invalid, "--output", base / "invalid.wasm", ok=False)
        assert b"positional list must be last" in result.stderr
        assert b"uutils" in invoke("search", "coreutils").stdout
        wit = invoke("inspect", "uutils", "--wit").stdout
        assert b"export run:" in wit and b"wasi:filesystem" in wit
        assert b"wasi:sockets" not in wit and b"wasi:http" not in wit
        assert b"--read" in invoke("uutils", "--help").stdout
        assert json.loads(invoke("uutils", "--json", "--help").stdout)["tool"]["command_output"]

        cases = [
            (["head", "-n", "2"], b"c\na\nb\n", b"c\na\n"),
            (["tail", "-n", "1"], b"c\na\nb\n", b"b\n"),
            (["cat", "-n"], b"hi\n", b"     1\thi\n"),
            (["wc", "-l"], b"hi\nbye\n", b"2\n"),
            (["cut", "-d", ",", "-f", "2"], b"a,b\nc,d\n", b"b\nd\n"),
            (["sort"], b"b\na\nb\n", b"a\nb\nb\n"),
            (["uniq", "-c"], b"a\na\nb\n", b"      2 a\n      1 b\n"),
            (["nl", "-ba"], b"x\n\ny\n", b"     1\tx\n     2\t\n     3\ty\n"),
            (["basename", "/src/hello.rs", ".rs"], b"", b"hello\n"),
            (["dirname", "/src/hello.rs"], b"", b"/src\n"),
        ]
        parity = 0
        for args, source, expected in cases:
            actual = invoke("uutils", "--stdin", *args, input=source).stdout
            assert actual == expected, (args, actual, expected)
            if shutil.which(args[0]):
                native = subprocess.run(args, input=source, capture_output=True, env=dict(os.environ, LC_ALL="C"))
                assert native.returncode == 0 and native.stdout == actual, (args, native, actual)
                parity += 1
        assert invoke("uutils", "head").stdout == b""  # No ambient stdin.
        assert invoke("uutils", "--input=x\ny\n", "head", "-n1").stdout == b"x\n"
        assert b"Usage:" in invoke("uutils", "head", "--help").stdout
        assert json.loads(invoke("uutils", "--json", "head", "--help").stdout)["exit_code"] == 0
        assert invoke("uutils", "top", ok=False).returncode == 2
        assert invoke("uutils", "head", "--no-such-flag", ok=False).returncode == 1
        assert b"UTF-8" in invoke("uutils", "--stdin", "cat", input=b"\xff", ok=False).stderr

        data = base / "data"
        data.mkdir()
        (data / "lines").write_bytes(b"c\na\nb\n")
        (data / "binary").write_bytes(b"\x00\xff\n")
        secret = base / "secret"
        secret.write_bytes(b"SECRET_CONTENT")
        grant = f"--read={data}::/data"
        (data / "-file").write_bytes(b"literal filename\n")
        assert invoke("uutils", grant, "cat", "--", "/data/-file").stdout == b"literal filename\n"
        (data / "-file").unlink()
        invoke("uutils", "cat", grant, "/data/lines", ok=False)  # A guest flag cannot grant authority.
        assert invoke("uutils", grant, "head", "-n", "1", "/data/lines").stdout == b"c\n"
        assert invoke("uutils", grant, "ls", "-1", "/data").stdout == b"binary\nlines\n"
        assert invoke("uutils", grant, "cat", "/data/binary").stdout == b"\x00\xff\n"
        record = json.loads(invoke("uutils", "--json", grant, "cat", "/data/binary").stdout)
        assert record == {"stdout": [0, 255, 10], "stderr": "", "exit_code": 0}
        assert invoke("uutils", f"--read={data / 'lines'}::/only", "cat", "/only").stdout == b"c\na\nb\n"
        for path in ["/data/lines", "/secret", "/etc/passwd"]:
            assert b"SECRET_CONTENT" not in invoke("uutils", "cat", path, ok=False).stdout
        for path in ["/secret", "/data/../secret", "/etc/passwd", "/proc/self/environ"]:
            assert b"SECRET_CONTENT" not in invoke("uutils", grant, "cat", path, ok=False).stdout
        result = invoke("uutils", "--json", grant, "cat", "/missing", ok=False)
        record = json.loads(result.stdout)
        assert record["exit_code"] == result.returncode == 1 and record["stderr"]
        assert not result.stderr
        if os.name != "nt":
            (data / "escape").symlink_to(secret)
            invoke("uutils", grant, "cat", "/data/escape", ok=False)
        invoke("uutils", grant, "sort", "-o", "/data/output", "/data/lines", ok=False)
        assert not (data / "output").exists()
        invoke("uutils", f"--read-write={data}::/data", "sort", "-o", "/data/output", "/data/lines")
        assert (data / "output").read_bytes() == b"a\nb\nc\n"
        (data / "large").write_bytes(b"x" * (8 * 1024 * 1024 + 1))
        result = invoke("uutils", grant, "cat", "/data/large", ok=False)
        assert not result.stdout and b"8 MiB" in result.stderr, result
        print(f"uutils WASI tests passed; {parity} native parity checks")


if __name__ == "__main__":
    main()
