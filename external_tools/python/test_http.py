"""Exercise urllib over WASI HTTP and verify host-enforced network boundaries."""
import argparse
from collections import Counter
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import time

ROOT = Path(__file__).resolve().parent
LOGGER = Path.home() / ".codex/skills/sat/scripts/run_sat.py"


def check(sat):
    hits = Counter()

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass

        def respond(self, status=200, body=b"", headers=()):
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            for name, value in headers:
                self.send_header(name, value)
            self.end_headers()
            if self.command != "HEAD":
                try:
                    self.wfile.write(body)
                except (BrokenPipeError, ConnectionResetError):
                    pass

        def do_GET(self):
            hits[self.path] += 1
            if self.path == "/redirect":
                self.respond(302, headers=[("Location", "/json")])
            elif self.path == "/redirect-denied":
                self.respond(302, headers=[("Location", f"http://localhost:{self.server.server_port}/forbidden")])
            elif self.path == "/missing":
                self.respond(404, b'{"error":"missing"}')
            elif self.path == "/large":
                self.respond(200, b"x" * (8 * 1024 * 1024 + 1))
            elif self.path == "/slow":
                time.sleep(0.3)
                self.respond(200, b"{}")
            else:
                body = json.dumps({"hello": "wasi", "host": self.headers["Host"],
                                   "custom": self.headers.get("X-Demo")}).encode()
                self.respond(200, body, [("X-Bridge", "wasi-http")])

        def do_HEAD(self):
            hits[self.path] += 1
            self.respond(200, headers=[("X-Bridge", "head")])

        def do_POST(self):
            hits[self.path] += 1
            self.respond(200, self.rfile.read(int(self.headers["Content-Length"])))

    with ThreadingHTTPServer(("127.0.0.1", 0), Handler) as server:
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        with tempfile.TemporaryDirectory(prefix="sat-python-http-") as temporary:
            project = Path(temporary)
            env = dict(os.environ, SAT_HOME=str(project / "registry"), SAT_BINARY=str(sat))
            prefix = [sys.executable, str(LOGGER), "--"] if LOGGER.exists() else [str(sat)]

            def invoke(*args, data=b""):
                result = subprocess.run([*prefix, *map(str, args)], cwd=project, env=env,
                                        input=data, capture_output=True, timeout=240)
                assert result.returncode == 0, (args, result.stdout, result.stderr)
                return result.stdout.decode()

            invoke("register", ROOT / "dist/python.tar", data=b"[]\n" * 5 + b"unset\n")
            base = f"http://127.0.0.1:{server.server_port}"
            prelude = "import urllib.request, urllib.error, json\n"

            def run(code, allowed=True):
                return invoke("python", "-c", prelude + code,
                              "--", "--allow-http", *(["127.0.0.1"] if allowed else ["unused.invalid"]))

            denied = f"""
try:
    urllib.request.urlopen({base + '/json'!r})
except urllib.error.URLError as error:
    assert "denied by SAT host policy" in str(error), str(error)
    print("denied")
else:
    raise AssertionError("request unexpectedly allowed")
"""
            # Registration defaults deny HTTP without requiring an explicit override.
            assert "denied" in invoke("python", "-c", prelude + denied)
            assert hits["/json"] == 0
            output = run(f"""
with urllib.request.urlopen({base + '/json'!r}) as response:
    assert response.status == 200
    assert response.headers["X-Bridge"] == "wasi-http"
    assert json.load(response)["hello"] == "wasi"
print("GET passed")
""")
            assert "GET passed" in output
            assert "denied" in run(denied, allowed=False)
            assert hits["/json"] == 1
            run(f"""
request = urllib.request.Request({base + '/json'!r}, headers={{"Host": "unrelated.invalid", "X-Demo": "kept"}})
with urllib.request.urlopen(request) as response:
    data = json.load(response)
assert data["host"] == {"127.0.0.1:" + str(server.server_port)!r}
assert data["custom"] == "kept"
""")
            run(f"""
with urllib.request.urlopen({base + '/redirect'!r}) as response:
    assert response.geturl() == {base + '/json'!r}
    assert json.load(response)["hello"] == "wasi"
""")
            run(f"""
try:
    urllib.request.urlopen({base + '/redirect-denied'!r})
except urllib.error.URLError as error:
    assert "denied by SAT host policy" in str(error), str(error)
else:
    raise AssertionError("redirect escaped allowlist")
""")
            assert hits["/forbidden"] == 0
            run(f"""
payload = b"echo " * 4000
with urllib.request.urlopen({base + '/echo'!r}, data=payload) as response:
    assert response.read() == payload
request = urllib.request.Request({base + '/head'!r}, method="HEAD")
with urllib.request.urlopen(request) as response:
    assert response.headers["X-Bridge"] == "head"
    assert response.read() == b""
""")
            run(f"""
try:
    urllib.request.urlopen({base + '/missing'!r})
except urllib.error.HTTPError as error:
    assert error.code == 404
    assert json.load(error)["error"] == "missing"
else:
    raise AssertionError("missing HTTPError")
""")
            run(f"""
try:
    urllib.request.urlopen({base + '/large'!r})
except urllib.error.URLError as error:
    assert "8 MiB" in str(error)
else:
    raise AssertionError("response buffer limit ignored")
""")
            started = time.monotonic()
            run(f"""
try:
    urllib.request.urlopen({base + '/slow'!r}, timeout=0.02)
except urllib.error.URLError:
    pass
else:
    raise AssertionError("explicit timeout ignored")
""")
            assert time.monotonic() - started < 5
            run(f"""
import socket
try:
    socket.create_connection(("127.0.0.1", {server.server_port}))
except OSError:
    pass
else:
    raise AssertionError("HTTP grant enabled raw sockets")
""")
        server.shutdown()
    print("python HTTP: GET, POST, HEAD, headers, redirects, HTTP errors, size limits, "
          "timeouts, default denial, hostname restrictions, and raw-socket isolation passed")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sat", type=Path, default=ROOT.parents[1] / "sat/target/debug/sat")
    args = parser.parse_args()
    check(args.sat.resolve())
