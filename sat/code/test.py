"""Verify real Tree-sitter WASIp2 grammars, retrieval metadata, and SAT confinement."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[2]
SAT = ROOT / "sat/target/debug" / ("sat.exe" if os.name == "nt" else "sat")


def main():
    with tempfile.TemporaryDirectory(prefix="sat-code-") as temporary:
        base = Path(temporary)
        env = dict(os.environ, SAT_HOME=str(base / "registry"), SAT_TEST_SECRET="do-not-inherit")

        def invoke(*args, ok=True):
            result = subprocess.run([str(SAT), *map(str, args)], cwd=base,
                                    env=env, text=True, encoding="utf-8", capture_output=True)
            assert (result.returncode == 0) == ok, (args, result.stdout, result.stderr)
            return result

        if "--skip-build" in sys.argv:
            invoke("install", ROOT / "sat/target/code.wasm", "--replace")
        else:
            subprocess.run(["bash", str(ROOT / "sat/code/build.sh")], cwd=ROOT, env=env, check=True)

        inputs = base / "inputs"
        inputs.mkdir()
        fixtures = {
            "auth.py": '''# module level refresh_token
class Auth:
    # Refresh credentials.
    @cached
    async def refresh(self):
        """Renew a token."""
        token = "refresh_token 日本語"
        return token + "refresh_token"

    def outer(self):
        def inner():
            return "nested_hit"
        return inner()
''',
            "auth.rs": '''struct Client;
impl Client {
    /// Refresh documentation.
    #[inline]
    fn refresh(&self) -> &str {
        "refresh_token"
    }
}
''',
            "auth.js": '''class Client {
  refresh() { return "refresh_token"; }
}
export const refresh = async (client) => {
  return client.refresh_token;
};
''',
            "auth.ts": '''class Client {
  refresh(): string { return "refresh_token"; }
}
const refresh = (client: Client): string => client.refresh();
''',
            "auth.tsx": '''export const Widget = () => <span>refresh_token</span>;
''',
        }
        for name, source in fixtures.items():
            (inputs / name).write_text(source, encoding="utf-8")
        (inputs / ".ignore").write_text("target/\n")
        (inputs / "target").mkdir()
        (inputs / "target/ignored.py").write_text('def ignored(): return "refresh_token"\n')
        secret = base / "secret.py"
        secret.write_text('def secret(): return "SECRET_CONTENT"\n')

        def code(operation, query=None, *args, paths=("/src",), grant=True, ok=True, format="json"):
            flags = [f"--format={format}", *(f"--paths={p}" for p in paths), *args]
            if grant:
                flags.append(f"--read={inputs}::/src")
            call = ["code", *flags, "--", operation]
            if query is not None:
                call.append(query)
            result = invoke(*call, ok=ok)
            if not ok:
                assert str(base) not in result.stdout + result.stderr, result
                assert "SECRET_CONTENT" not in result.stdout + result.stderr, result
                return result
            return json.loads(result.stdout) if format == "json" else result.stdout

        assert "code" in invoke("search", "Tree-sitter").stdout
        assert "--position" in invoke("code", "--help").stdout
        wit = invoke("inspect", "code", "--wit").stdout
        assert "wasi:filesystem" in wit and "export run:" in wit
        assert "wasi:sockets" not in wit and "wasi:http" not in wit

        # Each matching file is parsed once. Repeated hits share one definition.
        found = code("search", "refresh_token")
        assert found["parsed_files"] == 5, found
        assert len(found["results"]) == 6, found
        assert not found["truncated"], found
        assert found["diagnostics"][0]["status"] == "no-enclosing-definition", found
        assert all(r["complete"] and r["status"] == "ok" for r in found["results"]), found
        assert all("target" not in r["path"] for r in found["results"])

        python = code("search", "refresh_token", paths=("/src/auth.py",))
        definition = python["results"][0]
        assert definition["symbol"] == "refresh" and len(definition["hits"]) == 2, python
        assert definition["parent_signature"] == "class Auth:", definition
        assert "# Refresh credentials." in definition["source"] and "@cached" in definition["source"]
        assert definition["file_hash"] == "sha256:" + hashlib.sha256(fixtures["auth.py"].encode()).hexdigest()
        data = fixtures["auth.py"].encode()
        extent = definition["range"]
        assert definition["source"].encode() == data[extent["start_byte"]:extent["end_byte"]]
        assert extent["start_line"] == 3 and extent["end_line"] == 8, extent
        for hit in definition["hits"]:
            assert data[hit["start_byte"]:hit["end_byte"]] == b"refresh_token", hit

        bare = code("search", "refresh_token", "--include-docs=false", "--include-parent-signature=false",
                    paths=("/src/auth.py",))["results"][0]
        assert bare["source"].startswith("async def refresh") and bare["parent_signature"] is None
        inner = code("search", "nested_hit", paths=("/src/auth.py",))["results"][0]
        outer = code("search", "nested_hit", "--context=outer-function", paths=("/src/auth.py",))["results"][0]
        owning = code("search", "nested_hit", "--context=class", paths=("/src/auth.py",))["results"][0]
        assert (inner["symbol"], outer["symbol"], owning["symbol"]) == ("inner", "outer", "Auth")
        outline = code("outline", paths=("/src/auth.py",))
        assert [r["symbol"] for r in outline["results"]] == ["Auth", "refresh", "outer", "inner"], outline
        assert all(r["source"] is None and r["status"] == "outline" for r in outline["results"])
        symbol = code("read-symbol", "refresh", paths=("/src/auth.py",))["results"][0]
        assert symbol["source"] == definition["source"]
        offset = data.index("日本語".encode())
        enclosing = code("read-enclosing", None, f"--position={offset}", paths=("/src/auth.py",))["results"][0]
        assert enclosing["source"] == definition["source"]
        code("read-enclosing", None, f"--position={offset + 1}", paths=("/src/auth.py",), ok=False)

        rust = code("search", "refresh_token", paths=("/src/auth.rs",))["results"][0]
        assert "/// Refresh documentation." in rust["source"] and "#[inline]" in rust["source"], rust
        assert rust["parent_signature"] == "impl Client", rust
        js = code("read-symbol", "refresh", paths=("/src/auth.js",))["results"]
        assert len(js) == 2 and js[1]["source"].startswith("export const refresh = async"), js
        assert js[0]["parent_signature"] == "class Client", js
        assert code("search", r"refresh_\w+", "--mode=regex", paths=("/src/auth.py",))["results"]
        assert code("search", "missing")["parsed_files"] == 0
        assert code("read-symbol", "missing")["results"] == []
        clipped = code("search", "refresh_token", "--max-bytes=5", paths=("/src/auth.py",))["results"][0]
        assert clipped["status"] == "size-limit" and clipped["source"] is None and not clipped["complete"]
        limited = code("search", "refresh_token", "--max-results=1")
        assert limited["truncated"] and len(limited["results"]) == 1, limited
        readable = code("search", "nested_hit", paths=("/src/auth.py",), format="text")
        assert "inner [function_definition]" in readable and 'return "nested_hit"' in readable

        # Content hashes reflect the exact bytes parsed, including non-ASCII.
        (inputs / "auth.py").write_text(fixtures["auth.py"] + "# modified\n", encoding="utf-8")
        changed = code("read-symbol", "refresh", paths=("/src/auth.py",))["results"][0]
        assert changed["file_hash"] != definition["file_hash"]
        (inputs / "broken.py").write_text('def broken(:\n    return "syntax_hit"\n')
        broken = code("outline", paths=("/src/broken.py",))
        # Even when recovery produces no declaration, the file error is explicit.
        assert any(r["parse_errors"] for r in broken["results"]) or any(
            d["status"] == "parse-error" for d in broken["diagnostics"]), broken

        for path in (str(secret), "/src/../secret.py"):
            code("search", "SECRET_CONTENT", paths=(path,), ok=False)
        code("search", "refresh_token", grant=False, ok=False)
        if os.name != "nt":
            (inputs / "escape.py").symlink_to(secret)
            code("search", "SECRET_CONTENT", paths=("/src/escape.py",), ok=False)
            assert not code("search", "SECRET_CONTENT")["results"]
        selected = invoke("code", "read-symbol", "refresh", "--format=json",
                          f"--read={inputs / 'auth.py'}::/selected.py", "--paths=/selected.py")
        assert json.loads(selected.stdout)["results"][0]["path"] == "/selected.py"
        invoke("code", "outline", f"--read={inputs / 'auth.py'}::/selected.py",
               "--paths=/auth.rs", ok=False)
        code("search", "[", "--mode=regex", ok=False)
        code("search", "x", "--max-results=0", ok=False)
        code("search", "x", "--max-bytes=0", ok=False)
        code("search", "x", "--mode=structural", ok=False)
        print("SAT code: all five WASI grammars, search/outline/symbol/enclosing retrieval, docs/owners, nesting, ranges/hashes, limits, and filesystem isolation passed")


if __name__ == "__main__":
    main()
