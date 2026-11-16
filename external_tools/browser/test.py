"""Real Chrome + WASI end-to-end tests. No external web service or Python dependency."""
import argparse
import http.server
import json
import os
from pathlib import Path
import struct
import subprocess
import tempfile
import threading
import time

ROOT = Path(__file__).resolve().parents[2]
TOOL = Path(__file__).resolve().parent
SAT = ROOT / 'sat/target/debug' / ('sat.exe' if os.name == 'nt' else 'sat')


class App(http.server.BaseHTTPRequestHandler):
    saved = []

    def log_message(self, *args):
        pass

    def do_GET(self):
        if self.path == '/api/drop':
            self.connection.close()
            return
        if self.path == '/api/fail':
            self.send_response(503)
            body = b'Unavailable'
        else:
            self.send_response(200)
            body = (TOOL / 'tests/app.html').read_bytes() if self.path == '/' else b'<html><body><button id="child" onclick="this.textContent=\'Frame clicked\'">Frame button</button><input id="child-input"></body></html>'
        self.send_header('Content-Type', 'text/html; charset=utf-8')
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_POST(self):
        data = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        self.saved.append(data)
        time.sleep(.2)
        body = json.dumps(data).encode()
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)


def check(build=True, artifacts=None):
    if build:
        subprocess.run(['bash', str(TOOL / 'package.sh')], check=True)
        subprocess.run(['cargo', 'build', '--locked', '--manifest-path', str(ROOT / 'sat/Cargo.toml')], check=True)
    with tempfile.TemporaryDirectory(prefix='sat-browser-test-') as temporary:
        base = Path(temporary)
        env = dict(os.environ, SAT_HOME=str(base / 'registry'), SAT_TEST_SECRET='unrelated-browser-host-secret')
        checks = 0

        def invoke(*args, ok=True, input=None):
            nonlocal checks
            result = subprocess.run([str(SAT), *map(str, args)], cwd=base, env=env,
                                    capture_output=True, input=input, timeout=180)
            assert (result.returncode == 0) == ok, (args, result.stdout.decode(), result.stderr.decode())
            assert b'unrelated-browser-host-secret' not in result.stdout + result.stderr
            checks += 1
            return result

        invoke('register', TOOL / 'dist/browser.tar', input=b'y\n'*6)
        invoke('browser', '--help')
        invoke('browser', 'tabs', ok=False)  # no browser authority
        server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), App)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        url = f'http://127.0.0.1:{server.server_port}/'
        sessions = []

        def start_session():
            p = subprocess.Popen([str(SAT), 'chrome-session'], cwd=base, env=env,
                                 stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            sessions.append(p)
            # select() on pipes is Unix-only; use a reader thread for portability.
            from concurrent.futures import ThreadPoolExecutor
            with ThreadPoolExecutor(1) as pool:
                line = pool.submit(p.stdout.readline).result(timeout=25)
            assert line, p.stderr.read().decode()
            endpoint = json.loads(line)['endpoint']
            return p, endpoint

        try:
            process, endpoint = start_session()

            def browser(action, *, target=None, value=None, tab=None, output=None, timeout=None, grants=(), ok=True, endpoint_override=None):
                args = ['browser', action]
                for key, val in [('target', target), ('value', value), ('tab', tab), ('output', output), ('timeout', timeout)]:
                    if val is not None:
                        args += ['--'+key, str(val)]
                args += ['--', '--allow-cdp', endpoint_override or endpoint, *map(str, grants)]
                result = invoke(*args, ok=ok)
                return json.loads(result.stdout) if ok else result

            def one(action, **kwargs):
                return browser(action, **kwargs)['results'][0]['result']

            tab = browser('tabs')['tabs'][0]['targetId']
            # Host-side grant parsing rejects other hosts, schemes, routes, credentials,
            # queries and fragments before any connection can be attempted.
            for bad in [endpoint.replace('127.0.0.1', 'localhost'), endpoint.replace('127.0.0.1', 'example.com'),
                        endpoint.replace('ws:', 'http:'), endpoint.replace('/browser/', '/page/'),
                        endpoint+'?redirect=1', endpoint+'#fragment', endpoint.replace('ws://', 'ws://user@')]:
                browser('tabs', endpoint_override=bad, ok=False)
            # HTTP permission alone never grants CDP authority.
            invoke('browser', 'tabs', '--', '--allow-http', '127.0.0.1', ok=False)
            one('navigate', value=url)
            one('wait-visible', target='#increment')
            snapshot = one('snapshot')
            assert snapshot['title'] == 'SAT browser laboratory'
            assert any(e['selector'] == '#increment' for e in snapshot['elements'])
            one('click', target='#increment')
            assert one('query', target='#counter')['text'] == '1'
            state = one('evaluate', value='window.lab')
            assert state['trusted'] is True and state['clicks'] == 1
            # Separate CLI invocations preserve page state.
            one('click', target='#increment')
            assert one('query', target='#counter')['text'] == '2'
            one('fill', target='#name', value='Ada "WASI" 🦀')
            one('type', target='#name', value=' Lovelace')
            assert one('query', target='#name')['value'] == 'Ada "WASI" 🦀 Lovelace'
            assert one('evaluate', value='lab.inputEvents') >= 2
            one('fill', target='#name', value='Grace Hopper')
            assert one('query', target='#name')['value'] == 'Grace Hopper'
            one('fill', target='#note', value='First line\nSecond line')
            one('fill', target='#editable', value='Content editable works')
            assert one('query', target='#editable')['text'] == 'Content editable works'
            one('click', target='#shadow >>> #shadow-button')
            assert one('query', target='#shadow >>> #shadow-result')['text'] == 'Shadow clicked'
            one('wait-visible', target='#delayed')
            assert one('evaluate', value='document.querySelector("#frame").contentDocument.querySelector("#child").textContent') == 'Frame button'
            one('click', target='#frame >>> #child')
            assert one('query', target='#frame >>> #child')['text'] == 'Frame clicked'
            one('fill', target='#frame >>> #child-input', value='Typing inside frame')
            assert one('query', target='#frame >>> #child-input')['value'] == 'Typing inside frame'
            assert one('query', target='#missing') is None
            browser('wait-visible', target='#missing', timeout=150, ok=False)
            browser('wait-text', target='#status', value='Impossible', timeout=150, ok=False)
            browser('click', target='#disabled', ok=False)
            browser('click', target='#covered', ok=False)
            browser('query', target='[', ok=False)
            browser('evaluate', value='throw new Error("intentional evaluation error")', ok=False)
            browser('evaluate', value='while(true){}', timeout=100, ok=False)
            browser('snapshot', timeout=0, ok=False)
            browser('unknown', ok=False)
            browser('fill', target='#increment', ok=False)
            assert one('query', target='#counter')['text'] == '2'  # bad arguments never click
            one('press', value='Tab')
            one('scroll', value='{"x":0,"y":400}')
            one('click', target='#bottom')  # scroll into view internally
            assert one('evaluate', value='scrollY') > 0

            shot = base / 'page.png'
            shot.touch()
            # Writes without grants, with read-only grants, and outside grants fail.
            browser('screenshot', output=shot, ok=False)
            browser('screenshot', output=shot, grants=['--read', shot], ok=False)
            other = base / 'other.png'
            other.touch()
            browser('screenshot', output=other, grants=['--write', shot], ok=False)
            assert shot.read_bytes() == other.read_bytes() == b''
            output_dir = base / 'output'
            output_dir.mkdir()
            created = output_dir / 'new.png'
            one('screenshot', output=created, grants=['--write', output_dir])
            assert created.read_bytes().startswith(b'\x89PNG')
            blocked = output_dir / 'blocked.png'
            blocked.write_bytes(b'unchanged')
            browser('screenshot', output=blocked, grants=['--write', output_dir, '--deny-write', '**/blocked.png'], ok=False)
            assert blocked.read_bytes() == b'unchanged'
            link = output_dir / 'escape.png'
            try:
                link.symlink_to(other)
            except OSError:
                pass  # Windows may require developer mode for symlinks.
            else:
                browser('screenshot', output=link, grants=['--write', output_dir], ok=False)
                assert other.read_bytes() == b''
            # One invocation executes the entire delayed interaction and diagnostics.
            steps = [
                {'action':'click','target':'#save'},
                {'action':'wait-text','target':'#status','value':'Saved: Grace Hopper'},
                {'action':'click','target':'#failure'},
                {'action':'click','target':'#network-failure'},
                {'action':'click','target':'#exception'},
                {'action':'diagnostics','timeout':700},
                {'action':'screenshot','output':str(shot)},
            ]
            result = browser('batch', value=json.dumps(steps), grants=['--write', shot])
            assert len(result['results']) == len(steps)
            events = result['results'][5]['result']['events']
            assert any(e['method'] == 'Runtime.consoleAPICalled' for e in events), events
            assert any(e['method'] == 'Runtime.exceptionThrown' for e in events), events
            assert any(e['method'] == 'Network.responseReceived' and e['status'] == 503 for e in events), events
            assert any(e['method'] == 'Network.loadingFailed' for e in events), events
            assert App.saved[-1] == {'name':'Grace Hopper','note':'First line\nSecond line'}
            image = shot.read_bytes()
            assert image[:8] == b'\x89PNG\r\n\x1a\n'
            width, height = struct.unpack('>II', image[16:24])
            assert width >= 1000 and height >= 600
            one('screenshot', output=shot, value='full', grants=['--write', shot])
            full_size = struct.unpack('>II', shot.read_bytes()[16:24])
            assert full_size[1] > height
            if artifacts:
                artifacts.mkdir(parents=True, exist_ok=True)
                (artifacts / 'laboratory.png').write_bytes(shot.read_bytes())
                (artifacts / 'diagnostics.json').write_text(json.dumps(events, indent=2))
                (artifacts / 'snapshot.json').write_text(json.dumps(snapshot, indent=2))
            browser('batch', value='[{}]', ok=False)
            browser('batch', value='[]', ok=False)
            browser('batch', value=json.dumps([{'action':'snapshot'}]*101), ok=False)
            browser('cdp', value='{"method":"NoSuchDomain.noSuchMethod"}', ok=False)
            stress = browser('batch', value=json.dumps([
                {'action':'evaluate','value':'for(let i=0;i<600;i++)console.warn("event flood",i)'},
                {'action':'diagnostics','timeout':100},
            ]))['results'][1]['result']
            assert stress['dropped'] > 0 and len(stress['events']) <= 128

            new_tab = browser('new', value='about:blank')['targetId']
            browser('snapshot', ok=False)  # never silently select another tab
            assert one('query', tab=tab, target='#counter')['text'] == '2'
            browser('close', tab=new_tab)

            # A different Chrome process is outside the granted endpoint even when
            # the attacker knows its target ID. This exercises process isolation.
            foreign_process, foreign_endpoint = start_session()
            foreign_tab = browser('tabs', endpoint_override=foreign_endpoint)['tabs'][0]['targetId']
            assert foreign_tab != tab
            browser('snapshot', tab=foreign_tab, ok=False)
            browser('cdp', value=json.dumps({'method':'Target.attachToTarget','params':{'targetId':foreign_tab,'flatten':True}}), ok=False)
            assert browser('tabs', endpoint_override=foreign_endpoint)['tabs'][0]['url'] == 'about:blank'
            foreign_process.stdin.close()
            assert foreign_process.wait(timeout=15) == 0
            process.stdin.close()
            assert process.wait(timeout=15) == 0
            browser('tabs', ok=False)  # cleanup removed endpoint
            timed = subprocess.Popen([str(SAT), 'chrome-session', '--duration', '1'], cwd=base, env=env,
                                     stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            sessions.append(timed)
            line = timed.stdout.readline()
            assert line, timed.stderr.read().decode()
            timed_endpoint = json.loads(line)['endpoint']
            assert timed.wait(timeout=15) == 0
            browser('tabs', endpoint_override=timed_endpoint, ok=False)
            if artifacts:
                (artifacts / 'report.json').write_text(json.dumps({'checks':checks,'viewport':[width,height],'full_page':list(full_size),'diagnostic_events':len(events)}, indent=2))
            print(f'browser: {checks} CLI checks passed; real Chrome DOM/input/waits/screenshots/diagnostics and isolation verified')
        finally:
            for p in sessions:
                if p.poll() is None:
                    try:
                        if p.stdin is not None and not p.stdin.closed:
                            p.stdin.close()
                        p.wait(timeout=15)
                    except (OSError, subprocess.TimeoutExpired):
                        p.kill()
                        p.wait()
                for pipe in (p.stdout, p.stderr):
                    if pipe is not None:
                        pipe.close()
            server.shutdown()
            server.server_close()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--skip-build', action='store_true')
    parser.add_argument('--artifacts', type=Path)
    args = parser.parse_args()
    check(build=not args.skip_build, artifacts=args.artifacts)
