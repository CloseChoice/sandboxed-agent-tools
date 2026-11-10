"""Real CLI/SQLite/HTTP integration tests. --browser also exercises the Svelte UI."""
from contextlib import closing
import concurrent.futures
import json
import os
from pathlib import Path
import queue
import re
import sqlite3
import subprocess
import sys
import tempfile
import threading
import urllib.error
import urllib.request

ROOT = Path(__file__).resolve().parents[2]
SAT = ROOT / "sat/target/release" / ("sat.exe" if os.name == "nt" else "sat")


def main():
    if "--skip-build" not in sys.argv:
        subprocess.run(["npm", "run", "build"], cwd=ROOT / "sat/dashboard", check=True)
        subprocess.run(["cargo", "build", "--locked", "--release", "--manifest-path", "sat/Cargo.toml"], cwd=ROOT, check=True)
    if not (ROOT / "sat/target/uutils.wasm").exists():
        subprocess.run(["bash", "sat/uutils/build.sh"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="sat-dashboard-") as temporary:
        home = Path(temporary) / "registry"
        env = dict(os.environ, SAT_HOME=str(home))
        database = home / "executions.sqlite3"

        def run(*args, code=0):
            result = subprocess.run([str(SAT), *args], env=env, capture_output=True, text=True, timeout=45)
            assert result.returncode == code, (args, result)
            return result

        run("missing-tool", code=1)
        assert not home.exists(), "disabled logging must not create SAT_HOME"
        run("dashboard", "enable")
        assert database.exists()
        run("install", str(ROOT / "sat/target/uutils.wasm"))
        args = ["--input=日本語\nsecond\n", "head", "-n", "1"]
        assert run("uutils", *args).stdout == "日本語\n"
        run("uutils", "head", "--unknown-flag", code=1)
        run("missing-tool", "--read=a path", code=1)
        with closing(sqlite3.connect(database)) as db:
            rows = db.execute("SELECT tool,args_json,status,exit_code,duration_ms FROM executions ORDER BY id").fetchall()
        assert len(rows) == 3
        assert json.loads(rows[0][1]) == args
        assert [row[2] for row in rows] == ["success", "failed", "failed"]
        assert all(row[4] is not None for row in rows)
        run("dashboard", "disable")
        run("uutils", "basename", "/one/two")
        with closing(sqlite3.connect(database)) as db:
            assert db.execute("SELECT COUNT(*) FROM executions").fetchone()[0] == 3
        run("dashboard", "reset")
        assert not database.exists()
        run("dashboard", "enable")
        run("uutils", "--input=hello", "cat")

        server = subprocess.Popen([str(SAT), "dashboard", "--no-open", "--port", "0"],
                                  env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        try:
            lines = queue.Queue()
            threading.Thread(target=lambda: lines.put(server.stdout.readline()), daemon=True).start()
            address = lines.get(timeout=15).strip().removeprefix("SAT dashboard: ")
            assert address.startswith("http://127.0.0.1:"), (address, server.stderr.read() if server.poll() is not None else "server did not report a URL")

            def request(path, headers=None, method="GET", expected=200):
                req = urllib.request.Request(address + path, headers=headers or {}, method=method)
                try:
                    with urllib.request.urlopen(req, timeout=10) as response:
                        status, body, response_headers = response.status, response.read(), response.headers
                except urllib.error.HTTPError as error:
                    status, body, response_headers = error.code, error.read(), error.headers
                assert status == expected, (path, status, body)
                return body, response_headers

            summary = json.loads(request("/api/overview")[0])
            assert summary["enabled"] and summary["totals"]["successes"] == 1
            assert summary["database"] == str(database)
            assert json.loads(request("/api/executions?tool=uutils")[0])["total"] == 1
            assert json.loads(request("/api/executions?tool=%27%20OR%201%3D1--")[0])["total"] == 0
            assert json.loads(request("/api/executions?q=hello")[0])["items"][0]["args"] == ["--input=hello", "cat"]
            request("/api/executions?status=invalid", expected=400)
            request("/api/executions?limit=not-a-number", expected=400)
            request("/api/overview", headers={"Host": "attacker.example"}, expected=403)
            request("/api/overview", headers={"Origin": "https://attacker.example"}, expected=403)
            request("/api/overview", headers={"Sec-Fetch-Site": "cross-site"}, expected=403)
            request("/api/overview", method="POST", expected=405)
            request("/executions.sqlite3", expected=404)
            request("/../dashboard.enabled", expected=404)
            index, headers = request("/")
            assert headers["X-Frame-Options"] == "DENY" and "frame-ancestors 'none'" in headers["Content-Security-Policy"]
            script = re.search(rb'src="(/assets/[^\"]+\.js)"', index).group(1).decode()
            assert request(script)[1]["Content-Type"].startswith("text/javascript")
            run("dashboard", "reset")
            assert not database.exists()
            summary = json.loads(request("/api/overview")[0])
            assert summary["enabled"] and summary["totals"]["executions"] == 0
            assert not database.exists(), "viewing an empty dashboard must not recreate the DB"
            with concurrent.futures.ThreadPoolExecutor(max_workers=6) as pool:
                list(pool.map(lambda _: run("uutils", "basename", "/one/two"), range(8)))
            assert json.loads(request("/api/overview")[0])["totals"]["successes"] == 8
            if "--browser" in sys.argv:
                # Deterministic fixture only in this temporary test registry.
                with closing(sqlite3.connect(database)) as db:
                    db.execute("DELETE FROM executions")
                    import time
                    now = int(time.time() * 1000)
                    names = ["python", "ripgrep", "code", "jaq", "uutils"]
                    for i in range(100):
                        tool = "python" if i % 2 == 0 else names[1 + ((i // 2) % 4)]
                        flags = ["-c", 'print("hello")', "--read=./src::/src", "<img src=x onerror=alert(1)>"] if tool == "python" else ["--paths=/src", "search"]
                        failed = i % 7 == 0
                        db.execute("INSERT INTO executions(token,tool,args_json,started_at_ms,duration_ms,status,exit_code,error) VALUES(?,?,?,?,?,?,?,?)",
                            (f"fixture-{i}", tool, json.dumps(flags), now - (99-i)*3600000, 80+i*17,
                             "failed" if failed else "success", 1 if failed else 0, "Example failure" if failed else None))
                    db.commit()
                browser_env = dict(env, SAT_DASHBOARD_URL=address, SAT_DASHBOARD_HOME=str(home))
                subprocess.run(["npm", "run", "test:browser"], cwd=ROOT / "sat/dashboard", env=browser_env, check=True)
            print("Dashboard CLI, opt-in lifecycle, SQLite, concurrency, loopback API and security checks passed")
        finally:
            server.terminate()
            server.wait(timeout=10)


if __name__ == "__main__":
    main()
