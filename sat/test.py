"""Integration checks against real components; HTTP uses a local echo server."""
import http.server
import json
import os
from pathlib import Path
import subprocess
import tempfile
import threading

from demo import ROOT, SAT, prepare


def main():
    with tempfile.TemporaryDirectory(prefix="sat-test-") as temp:
        root = Path(temp)
        env = prepare(root)
        env["SAT_TEST_SECRET"] = "must-not-leak"

        def run(*args, ok=True):
            p = subprocess.run([str(SAT), *map(str, args)], env=env, capture_output=True, text=True)
            assert (p.returncode == 0) == ok, (args, p.stdout, p.stderr)
            return p.stdout if ok else p.stdout + p.stderr

        assert 0 <= int(run("random")) < 2**64
        assert len(json.loads(run("list", "--json"))) == 3
        assert "httpbin" in run("search", "mirrored")
        assert "--method" in run("httpbin", "--help")
        assert json.loads(run("httpbin", "--help", "--json"))["tool"]["name"] == "httpbin"
        assert "export run" in run("inspect", "httpbin", "--wit")
        assert run("inspect", "httpbin", "--policy") == (ROOT / "sat/examples/http-policy.toml").read_text()
        assert "No embedded" in run("inspect", "random", "--policy")
        assert "HttpRequestDenied" in run("httpbin", "https://httpbin.org/get", ok=False)
        for url in ["https://example.com/get", "https://httpbin.org/post", "http://httpbin.org/get"]:
            assert "HttpRequestDenied" in run("httpbin", url, "--allow-http", ok=False)

        bindings = [f"--documents={root / 'documents'}", f"--input={root / 'input.txt'}", f"--output={root / 'output.txt'}"]
        def files(operation, *args, ok=True):
            return run("files", operation, *bindings, *args, ok=ok)

        for flag in ["--read", "--read-write"]:
            assert "cannot override" in files("input", flag, root, ok=False)
        assert "hello.txt" in files("list")
        assert "Hello from the granted" in files("read", "--path=hello.txt")
        assert "selected input" in files("input")
        assert "not inherited" in files("env")
        files("output", "--text=written through the file handle")
        assert (root / "output.txt").read_text() == "written through the file handle"
        assert "write denied" in files("write-dir", "--path=blocked.txt", "--text=bad", ok=False)
        assert not (root / "documents/blocked.txt").exists()
        for path in ["../input.txt", str(root / "input.txt")]:
            error = files("read", f"--path={path}", ok=False)
            assert "filesystem operation failed" in error and str(root) not in error
        if os.name != "nt":
            (root / "documents/escape").symlink_to(root / "input.txt")
            assert "filesystem operation failed" in files("read", "--path=escape", ok=False)
        assert "required" in run("files", "read", ok=False)

        # Empty embedded filesystem categories also block the generic grants.
        deny = root / "deny-files.toml"
        deny.write_text("files = []\n")
        restricted_meta = root / "restricted.toml"
        restricted_meta.write_text((ROOT / "sat/examples/random.toml").read_text().replace('name = "random"', 'name = "restricted-random"'))
        restricted = root / "restricted.wasm"
        run("pack", ROOT / "random-number-example/target/wasm32-wasip2/release/random_number_component.wasm",
            "--metadata", restricted_meta, "--policy", deny, "-o", restricted)
        run("install", restricted)
        for flag in ["--read", "--read-write"]:
            assert "cannot override" in run("restricted-random", flag, root, ok=False)

        class Echo(http.server.BaseHTTPRequestHandler):
            calls = 0
            def do_GET(self):
                Echo.calls += 1
                body = json.dumps({"path": self.path}).encode()
                self.send_response(200)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)
            def log_message(self, *args):
                pass

        server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Echo)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            origin = f"http://127.0.0.1:{server.server_port}"
            # Same real HTTP guest, with an explicit local policy for offline testing.
            policy = root / "local.toml"
            policy.write_text(f'[[http]]\nmethods=["GET"]\norigin="{origin}"\npaths=["/get"]\n')
            guest = ROOT / "http-get-example/target/wasm32-wasip2/release/http_get_component.wasm"
            meta = root / "local-tool.toml"
            meta.write_text((ROOT / "sat/examples/http.toml").read_text().replace('name = "httpbin"', 'name = "local-http"'))
            package = root / "local-http.wasm"
            run("pack", guest, "--metadata", meta, "--policy", policy, "-o", package)
            run("install", package)
            assert json.loads(run("local-http", origin + "/get?hello=world", "--allow-http"))["path"] == "/get?hello=world"
            assert "HttpRequestDenied" in run("local-http", origin + "/other", "--allow-http", ok=False)
            assert Echo.calls == 1
            assert "denied" in files("socket", f"--path=127.0.0.1:{server.server_port}")
            # Missing package policy adds no restriction, but still needs an invocation grant.
            run("pack", guest, "--metadata", meta, "-o", package)
            run("install", package, "--replace")
            assert "HttpRequestDenied" in run("local-http", origin + "/other", ok=False)
            assert json.loads(run("local-http", origin + "/other", "--allow-http"))["path"] == "/other"
        finally:
            server.shutdown()
            server.server_close()
            thread.join()

        # Reject incorrect CLI/WIT mappings at packaging time, before execution.
        wrong = root / "wrong.toml"
        wrong.write_text((ROOT / "sat/examples/http.toml").read_text().replace("options.method", "options.missing"))
        assert "unknown WIT parameter" in run("pack", guest, "--metadata", wrong, "-o", root / "wrong.wasm", ok=False)
        malformed = root / "malformed.wasm"
        malformed.write_bytes(b"not a component")
        run("install", malformed, ok=False)
        installed = Path(env["SAT_HOME"]) / "random/tool.wasm"
        installed.write_bytes(installed.read_bytes() + b"tampered")
        assert "integrity check failed" in run("random", "--help", ok=False)
        print("PASS: CLI/help, WIT extraction, HTTP grants/routes, file isolation, read-only policy, environment/socket denial, and integrity.")


if __name__ == "__main__":
    main()
