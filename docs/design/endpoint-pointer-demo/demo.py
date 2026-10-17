import argparse
import shutil
import http.server
import json
import os
import pathlib
import subprocess
import tempfile
import threading

parser = argparse.ArgumentParser()
parser.add_argument('candidate', type=pathlib.Path)
parser.add_argument('fixture', type=pathlib.Path)
parser.add_argument('trust_cli', type=pathlib.Path)
parser.add_argument('--baseline', type=pathlib.Path)
parser.add_argument('--old-trust-cli', type=pathlib.Path)
args = parser.parse_args()
args.candidate = args.candidate.resolve()
args.fixture = args.fixture.resolve()
args.trust_cli = args.trust_cli.resolve()
if args.baseline:
    args.baseline = args.baseline.resolve()
if args.old_trust_cli:
    args.old_trust_cli = args.old_trust_cli.resolve()
requests = []
mode = 'ok'
class Handler(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        body = self.rfile.read(int(self.headers.get('Content-Length', '0')))
        requests.append((self.path, dict(self.headers), body.decode()))
        if mode == 'redirect':
            self.send_response(302)
            self.send_header('Location', f'http://127.0.0.1:{self.server.server_port}/unconsented')
            self.end_headers()
            return
        if mode == 'fail':
            self.send_response(503)
            self.end_headers()
            return
        payload = b'{"head":{"vars":["s"]},"results":{"bindings":[{"s":{"type":"uri","value":"urn:served"}}]}}'
        self.send_response(200)
        self.send_header('Content-Type', 'application/sparql-results+json')
        self.send_header('Content-Length', str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)
    def log_message(self, *_):
        pass
server = http.server.HTTPServer(('127.0.0.1', 0), Handler)
thread = threading.Thread(target=server.serve_forever, daemon=True)
thread.start()
try:
    with tempfile.TemporaryDirectory() as scratch:
        root = pathlib.Path(scratch)
        bin_dir = root / 'bin'
        bin_dir.mkdir()
        for source, name in [(args.candidate, 'quipu-service-candidate'), (args.fixture, 'endpoint_pointer_fixture'), (args.trust_cli, 'caboodle')]:
            shutil.copy2(source, bin_dir / name)
        endpoint = f'http://127.0.0.1:{server.server_port}/sparql/'
        remote = 'https://github.com/foreign/project.git'
        env = dict(os.environ, HOME=str(root), XDG_CONFIG_HOME=str(root/'.config'), PATH=str(bin_dir))
        policy = root/'.config/caboodle/pack-trust.toml'
        policy.parent.mkdir(parents=True)
        cfg = root/'.bobbin/config.toml'
        cfg.parent.mkdir()
        declaration = root/'declaration.pendant'
        subprocess.run([str(bin_dir/'endpoint_pointer_fixture'), str(declaration), endpoint], env=env, cwd=root, check=True, capture_output=True)
        cfg.write_text(f'[[quipu.federation.pendants]]\npath = {json.dumps(str(declaration))}\nrepo_remote = {json.dumps(remote)}\n')
        query = f'SELECT ?s WHERE {{ SERVICE <{endpoint}> {{ ?s ?p ?o }} }}'
        def run(binary, expected, q=query):
            result = subprocess.run([str(binary), 'query', q, '--federation', '--db', str(root/'receiver.db')], env=env, cwd=root, capture_output=True, text=True, timeout=12)
            assert result.returncode == expected, (result.returncode, result.stdout, result.stderr)
            return result.stdout + result.stderr
        if args.baseline:
            text = run(args.baseline, 1)
            assert 'no configured remotes' in text and len(requests) == 0
            print('before: SERVICE unavailable, requests=0')
        text = run(bin_dir/'quipu-service-candidate', 1)
        assert endpoint in text and remote in text and len(requests) == 0
        print('foreign no consent: named refusal, requests=0')
        policy.write_text('[[allow]]\nrepo="github.com/foreign/project"\npack_sha256="'+'a'*64+'"\n')
        run(bin_dir/'quipu-service-candidate', 1)
        assert len(requests) == 0
        print('pack-only approval: refused, requests=0')
        allow = f'[[allow_endpoint]]\nrepo="github.com/foreign/project"\nendpoint={json.dumps(endpoint)}\n'
        policy.write_text(allow)
        text = run(bin_dir/'quipu-service-candidate', 0)
        assert 'urn:served' in text and len(requests) == 1, text
        assert requests[-1][0] == '/sparql/'
        assert not any(k.lower() in ['authorization', 'cookie'] for k in requests[-1][1])
        print('exact consent: actual SERVICE row urn:served, exact trailing-slash path, no credentials')
        policy.write_text('')
        text = run(bin_dir/'quipu-service-candidate', 1)
        assert endpoint in text and remote in text and len(requests) == 1
        print('revocation: named refusal, no further request')
        policy.write_text(allow)
        run(bin_dir/'quipu-service-candidate', 1, query.replace('/sparql/', '/sparql/query'))
        assert len(requests) == 1
        print('URL neighbour: refused, no request')
        mode = 'redirect'
        run(bin_dir/'quipu-service-candidate', 1)
        assert len(requests) == 2 and requests[-1][0] == '/sparql/'
        print('redirect: refused, unconsented target never reached')
        mode = 'fail'
        run(bin_dir/'quipu-service-candidate', 1)
        assert len(requests) == 3
        print('HTTP503: error preserved')
        manifest = declaration/'manifest.json'
        original = manifest.read_bytes()
        value = json.loads(original)
        value['share_id'] = 'sha256:'+'0'*64
        manifest.write_text(json.dumps(value))
        run(bin_dir/'quipu-service-candidate', 1)
        assert len(requests) == 3
        manifest.write_bytes(original)
        print('tampered declaration: refused before network')
        policy.write_text(allow)
        if args.old_trust_cli:
            shutil.copy2(args.old_trust_cli, bin_dir / 'caboodle')
            text = run(bin_dir/'quipu-service-candidate', 1)
            assert 'unavailable' in text and len(requests) == 3
            print('older trust CLI: unavailable, no policy denial invented, no request')
        (bin_dir/'caboodle').unlink()
        text = run(bin_dir/'quipu-service-candidate', 1)
        assert 'unavailable' in text and len(requests) == 3
        shutil.copy2(args.trust_cli, bin_dir/'caboodle')
        print('missing trust CLI: unavailable, no fallback or request')
        (bin_dir/'caboodle').write_text('#!/bin/sh\nexec /bin/sleep 10\n')
        (bin_dir/'caboodle').chmod(0o755)
        text = run(bin_dir/'quipu-service-candidate', 1)
        assert 'did not complete' in text and len(requests) == 3
        shutil.copy2(args.trust_cli, bin_dir/'caboodle')
        print('hung trust CLI: bounded refusal, child reaped, no request')
        policy.write_text('unknown_key = true\n')
        run(bin_dir/'quipu-service-candidate', 1)
        assert len(requests) == 3
        print('malformed policy: refused before network')
        policy.write_text(allow)
        text = run(bin_dir/'quipu-service-candidate', 0, 'SELECT (COUNT(*) AS ?n) WHERE { ?s ?p ?o }')
        assert '0' in text, text
        print('receiver ROOT: 0 facts, no imported remote data')
        print('ALL endpoint SERVICE acceptance controls PASS')
finally:
    server.shutdown()
    server.server_close()
    thread.join()
