"""Build and exercise the actual WASIp2 component through SAT, in a private registry."""
import json
import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
SAT = ROOT / "sat/target/debug" / ("sat.exe" if os.name == "nt" else "sat")


def main():
    with tempfile.TemporaryDirectory(prefix="sat-ripgrep-") as temporary:
        base = Path(temporary)
        env = dict(os.environ, SAT_HOME=str(base / "registry"),
                   RIPGREP_CONFIG_PATH=str(base / "secret"))
        subprocess.run(["bash", str(ROOT / "sat/ripgrep/build.sh")],
                       cwd=ROOT, env=env, check=True)
        inputs = base / "inputs"
        inputs.mkdir()
        (inputs / "a.txt").write_text("before\nHello rust\nafter\nhello WASI\na.b\naxb\n", encoding="utf-8")
        (inputs / "unicode.txt").write_text("café 日本語\n", encoding="utf-8")
        (inputs / ".hidden").write_text("hello hidden\n")
        (inputs / ".gitignore").write_text("ignored.txt\n")
        (inputs / "ignored.txt").write_text("hello ignored\n")
        # A subtree grant cannot see its parent's .gitignore. Local ignore
        # rules must prevent scanning generated trees and exhausting fuel.
        (inputs / ".ignore").write_text((ROOT / "sat/.ignore").read_text())
        (inputs / "binary").write_bytes(b"\0hello binary\n")
        nested = inputs / "nested"
        nested.mkdir()
        (nested / "b.txt").write_text("hello nested\n")
        secret = base / "secret"
        secret.write_text("TOP_SECRET\n")

        def invoke(*args, ok=True):
            p = subprocess.run([str(SAT), *map(str, args)], cwd=base, env=env,
                               text=True, encoding="utf-8", capture_output=True)
            assert (p.returncode == 0) == ok, (args, p.stdout, p.stderr)
            return p

        def search(pattern, *args, grant=True, ok=True, paths=("/data",)):
            flags = [f"--paths={p}" for p in paths]
            if grant:
                flags.append(f"--read={inputs}::/data")
            p = invoke("ripgrep", "--format=json", *flags, *args, "--", pattern, ok=ok)
            if not ok:
                assert str(base) not in p.stdout + p.stderr, p
                assert "TOP_SECRET" not in p.stdout + p.stderr, p
                return p
            return json.loads(p.stdout)

        assert "ripgrep" in invoke("search", "regex").stdout
        assert "--max-results" in invoke("ripgrep", "--help").stdout
        wit = invoke("inspect", "ripgrep", "--wit").stdout
        assert "export run:" in wit and "wasi:filesystem" in wit
        assert "wasi:sockets" not in wit and "wasi:http" not in wit
        assert "http = []" in invoke("inspect", "ripgrep", "--policy").stdout

        result = search("hello", "-i", "true")
        assert [r["text"] for r in result["results"]] == ["Hello rust", "hello WASI", "hello nested"], result
        assert not result["truncated"]
        assert all(r["path"].startswith("/data/") for r in result["results"])
        assert len(search("a.b", paths=("/data/a.txt",))["results"]) == 2
        assert len(search("a.b", "-F", "true", paths=("/data/a.txt",))["results"]) == 1
        assert search("^Hello rust$", paths=("/data/a.txt",))["results"][0]["line"] == 2
        assert search(r"hello\s+WASI", paths=("/data/a.txt",))["results"][0]["line"] == 4
        assert search("日本語")["results"][0]["text"] == "café 日本語"
        context = search("Hello", "-C", "1", paths=("/data/a.txt",))["results"]
        assert [(r["line"], r["matched"]) for r in context] == [(1, False), (2, True), (3, False)]
        assert any(r["path"].endswith(".hidden") for r in search("hello", "--hidden", "true")["results"])
        assert any(r["path"].endswith("ignored.txt") for r in search("hello", "--no-ignore", "true")["results"])
        limited = search("hello", "--max-results", "1")
        assert len(limited["results"]) == 1 and limited["truncated"]
        assert search("no-such-pattern") == {"results": [], "truncated": False}
        search("[", ok=False)
        search("hello", "--max-results", "0", ok=False)
        search("hello", "--context", "101", ok=False)
        search("hello", grant=False, ok=False)
        search("TOP_SECRET", paths=(str(secret),), ok=False)
        search("TOP_SECRET", paths=("/data/../secret",), ok=False)

        # An exact-file grant must not expose its parent or siblings.
        exact = invoke("ripgrep", "--format=json", f"--read={inputs / 'a.txt'}::/selected.txt",
                       "--paths=/selected.txt", "Hello")
        assert json.loads(exact.stdout)["results"][0]["path"] == "/selected.txt"
        invoke("ripgrep", f"--read={inputs / 'a.txt'}::/selected.txt",
               "--paths=/unicode.txt", "日本語", ok=False)
        if os.name != "nt":
            (inputs / "escape").symlink_to(secret)
            search("TOP_SECRET", paths=("/data/escape",), ok=False)
            assert not search("TOP_SECRET")["results"]

        # Result payloads are bounded even for very long matching lines.
        (inputs / "large.txt").write_text("x" * (1024 * 1024 + 1))
        assert search("x", paths=("/data/large.txt",)) == {"results": [], "truncated": True}
        (inputs / "large.txt").unlink()
        generated = inputs / "target" / "debug"
        generated.mkdir(parents=True)
        with (generated / "build-output.txt").open("wb") as output:
            for _ in range(1024):
                output.write(b"hello generated\n" * 1024)
        assert search("hello", "-i", "true") == result
        grant = f"--read={inputs}::/data"
        readable = invoke("ripgrep", grant, "--paths=/data/a.txt", "-C", "1", "Hello")
        assert readable.stdout == "/data/a.txt\n1-before\n2:Hello rust\n3-after\n", readable
        separated = invoke("ripgrep", grant, "--paths=/data/a.txt", "-C", "1", "Hello|axb")
        assert separated.stdout == "/data/a.txt\n1-before\n2:Hello rust\n3-after\n--\n5-a.b\n6:axb\n", separated
        plain = invoke("ripgrep", grant, "--paths=/data/a.txt", "Hello|axb")
        assert plain.stdout == "/data/a.txt\n2:Hello rust\n6:axb\n", plain
        grouped = invoke("ripgrep", grant, "--paths=/data", "hello", "-i", "true")
        assert "4:hello WASI\n\n/data/nested/b.txt\n1:hello nested\n" in grouped.stdout, grouped
        limited_text = invoke("ripgrep", grant, "--paths=/data", "hello", "--max-results=1")
        assert "[results truncated;" in limited_text.stdout
        assert invoke("ripgrep", grant, "--paths=/data", "no-such-pattern").stdout == "\n"
        assert "format must be text or json" in invoke(
            "ripgrep", "--format=invalid", "Hello", ok=False).stderr
        print("SAT ripgrep: text/JSON formatting, matching, recursion, ignore rules, context, limits, WASI imports, and filesystem isolation passed")


if __name__ == "__main__":
    main()
