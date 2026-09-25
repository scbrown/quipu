#!/usr/bin/env python3
"""Exercise actual MCP protocol and credential boundaries on isolated binaries."""
import concurrent.futures
import hashlib
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request

server_binary = Path(sys.argv[1]).resolve()
cli_binary = server_binary.with_name('quipu')
SHARED = 's' * 43
TOKENS = {'one': 'a' * 43, 'two': 'b' * 43}


def envelope(method, params=None, ident=1):
    return {'jsonrpc': '2.0', 'id': ident, 'method': method, 'params': params or {}}


def initialized():
    return envelope('initialize', {'protocolVersion': '2025-03-26',
                    'capabilities': {}, 'clientInfo': {'name': 'acceptance', 'version': '1'}})


def unpack(result):
    if 'error' in result:
        raise AssertionError(result['error'])
    return result['result']


def tool_body(result):
    return json.loads(unpack(result)['content'][0]['text'])


with tempfile.TemporaryDirectory(prefix='quipu-native-mcp-') as directory:
    root = Path(directory)
    home = root / 'home'; home.mkdir()
    config_dir = root / '.bobbin'; config_dir.mkdir()
    registry = root / 'registry.json'
    registry.write_text(json.dumps({'version': 1, 'credentials': [
        {'credential_id': name, 'principal': 'urn:crew:' + name, 'audience': 'quipu',
         'token_sha256': hashlib.sha256(token.encode()).hexdigest()}
        for name, token in TOKENS.items()]}))
    config_file = config_dir / 'config.toml'
    model = os.environ.get('QUIPU_MCP_TEST_MODEL_DIR')
    embedding_config = ''
    if model:
        embedding_config = ('[quipu.embedding]\nmodel_path = ' + json.dumps(str(Path(model) / 'onnx/model.onnx'))
                            + '\ntokenizer_path = ' + json.dumps(str(Path(model) / 'tokenizer.json')) + '\n')
    config = (embedding_config + '[quipu.server]\nauth_token = "' + SHARED + '"\n'
              + 'crew_credentials_file = ' + json.dumps(str(registry)) + '\n')
    config_file.write_text(config)
    env = {key: os.environ[key] for key in ('PATH', 'LD_LIBRARY_PATH', 'ORT_DYLIB_PATH', 'LANG') if key in os.environ}
    env['HOME'] = str(home)
    # These point to real-user credential/config locations in some harnesses.
    for key in ('XDG_CONFIG_HOME', 'QUIPU_AUTH_TOKEN', 'QUIPU_AUTH_TOKEN_FILE'):
        env.pop(key, None)
    processes = []
    log = (root / 'server.log').open('w+')

    def start(*args):
        proc = subprocess.Popen([str(server_binary), *args], cwd=root, env=env,
                                stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                stderr=log, text=True)
        processes.append(proc)
        return proc

    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0)); port = sock.getsockname()[1]
    base = 'http://127.0.0.1:' + str(port)

    def http(path, value=None, token=None, extra=None):
        headers = {'Accept': 'application/json, text/event-stream'}
        if token is not None: headers['Authorization'] = 'Bearer ' + token
        if extra: headers.update(extra)
        data = None if value is None else json.dumps(value).encode()
        if data is not None: headers['Content-Type'] = 'application/json'
        request = urllib.request.Request(base + path, data=data, headers=headers)
        try:
            response = urllib.request.urlopen(request, timeout=15)
        except urllib.error.HTTPError as error:
            return error.code, error.read().decode()
        text = response.read().decode()
        if text.startswith('event:') or text.startswith('data:'):
            text = next(line[6:] for line in text.splitlines() if line.startswith('data: '))
        return response.status, json.loads(text) if text else None

    def call(name, args=None, token=None):
        code, response = http('/mcp', envelope('tools/call', {'name': name, 'arguments': args or {}}), token)
        assert code == 200, (code, response)
        return response

    def ready(proc):
        for _ in range(300):
            if proc.poll() is not None:
                log.flush(); log.seek(0)
                raise AssertionError('server exited: ' + log.read())
            try:
                if http('/health')[0] == 200: return
            except (OSError, urllib.error.URLError): pass
            time.sleep(.1)
        raise AssertionError('server did not become ready')

    try:
        proc = start('--db', str(root / 'http.db'), '--bind', '127.0.0.1:' + str(port))
        ready(proc)
        code, init = http('/mcp', initialized()); assert code == 200
        assert unpack(init)['serverInfo']['name'] == 'quipu'
        code, listing = http('/mcp', envelope('tools/list')); assert code == 200
        tools = unpack(listing)['tools']
        names = {t['name'] for t in tools}
        assert {'quipu_query', 'quipu_search', 'quipu_episode', 'quipu_knot'} <= names
        assert len(names) == len(tools)
        by_name = {t['name']: t for t in tools}
        assert by_name['quipu_query']['annotations']['readOnlyHint'] is True
        assert by_name['quipu_set']['annotations']['destructiveHint'] is True
        query = {'query': 'SELECT ?s WHERE { ?s <urn:p> ?o }'}
        assert tool_body(call('quipu_query', query))['count'] == 0
        assert unpack(call('quipu_search', {'embedding': [1.0, 0.0]})).get('isError') is not True
        text_search = unpack(call('quipu_search', {'query': 'fixture'}))
        if model:
            assert text_search.get('isError') is not True, text_search
        else:
            assert text_search.get('isError') is True
            assert 'no embedding provider' in text_search['content'][0]['text']
        turtle = {'turtle': '<urn:fixture> <urn:p> "fixture" .', 'actor': 'urn:spoof'}
        assert unpack(call('quipu_knot', turtle)).get('isError') is True
        assert unpack(call('quipu_knot', turtle, 'invalid')).get('isError') is True
        assert tool_body(call('quipu_query', query, 'invalid'))['count'] == 0
        assert http('/mcp', initialized(), extra={'Origin': 'https://untrusted.example'})[0] == 403
        assert 'error' in call('quipu_not_registered')
        code, created_graph = http('/graph/create', {'graph': 'urn:fixture:graph'}, SHARED)
        assert code == 200, created_graph
        assert tool_body(call('quipu_graph_list'))['count'] >= 1
        assert tool_body(call('quipu_graph_list', {'lifecycle': 'frozen'}))['count'] == 0

        def write(name, token):
            body = {'turtle': '<urn:' + name + '> <urn:p> "' + name + '" .', 'actor': 'urn:spoof'}
            result = unpack(call('quipu_knot', body, token))
            assert result.get('isError') is not True, result
            return name, json.loads(result['content'][0]['text'])['tx_id']
        with concurrent.futures.ThreadPoolExecutor(max_workers=3) as pool:
            futures = [pool.submit(write, name, token) for name, token in [*TOKENS.items(), ('shared', SHARED)]]
            written = dict(future.result() for future in futures)
        assert tool_body(call('quipu_query', query))['count'] == 3
        code, transactions = http('/transactions?since=0&limit=100')
        assert code == 200
        identities = [t['authenticated'] for t in transactions['transactions'] if t.get('authenticated')]
        assert {x['principal'] for x in identities} == {'urn:crew:one', 'urn:crew:two', 'legacy-shared-bearer'}, identities
        assert {x['credential_id'] for x in identities} == {'one', 'two', None}
        by_tx = {t['id']: t for t in transactions['transactions']}
        for name, tx in written.items():
            expected = 'legacy-shared-bearer' if name == 'shared' else 'urn:crew:' + name
            assert by_tx[tx]['authenticated']['principal'] == expected, by_tx[tx]
            assert by_tx[tx]['actor'] == 'urn:spoof'

        assert unpack(call('quipu_knot', turtle)).get('isError') is True  # no previous-identity leak
        print('HTTP: manifest, reads, spoofing, concurrent named/shared attribution, origins, bad/no bearer passed')

        proc.terminate(); proc.wait(timeout=10)
        config_file.write_text(config + 'read_only = true\n')
        proc = start('--db', str(root / 'http.db'), '--bind', '127.0.0.1:' + str(port))
        ready(proc)
        assert unpack(call('quipu_knot', turtle, TOKENS['one'])).get('isError') is True
        assert tool_body(call('quipu_query', query))['count'] == 3
        print('HTTP: restart without stale session and read-only refusal passed')
        proc.terminate(); proc.wait(timeout=10)
        config_file.write_text(config)

        token_file = root / 'client-token'; token_file.write_text(TOKENS['one']); token_file.chmod(0o600)
        proc = subprocess.Popen([str(cli_binary), 'mcp', '--db', str(root / 'stdio.db'),
                                 '--mcp-token-file', str(token_file)], cwd=root, env=env,
                                stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=log, text=True)
        processes.append(proc)
        def stdio(message):
            proc.stdin.write(json.dumps(message) + '\n'); proc.stdin.flush()
            line = proc.stdout.readline()
            assert line, 'stdio ended without a protocol response'
            return json.loads(line)
        assert unpack(stdio(initialized()))['serverInfo']['name'] == 'quipu'
        proc.stdin.write(json.dumps({'jsonrpc': '2.0', 'method': 'notifications/initialized'}) + '\n'); proc.stdin.flush()
        assert len(unpack(stdio(envelope('tools/list', ident=2)))['tools']) == len(tools)
        assert tool_body(stdio(envelope('tools/call', {'name': 'quipu_query', 'arguments': query}, 3)))['count'] == 0
        assert unpack(stdio(envelope('tools/call', {'name': 'quipu_knot', 'arguments': turtle}, 4))).get('isError') is not True
        assert tool_body(stdio(envelope('tools/call', {'name': 'quipu_query', 'arguments': query}, 5)))['count'] == 1
        proc.stdin.close(); proc.wait(timeout=15)
        assert proc.returncode == 0
        print('STDIO: CLI companion, initialize/list/query/write, private credential and clean EOF passed')
        if os.name == 'posix':
            token_file.chmod(0o644)
            rejected = subprocess.run([str(cli_binary), 'mcp', '--db', str(root / 'stdio.db'),
                                       '--mcp-token-file', str(token_file)], cwd=root, env=env,
                                      input='', capture_output=True, text=True, timeout=20)
            assert rejected.returncode != 0 and 'must be private' in rejected.stderr
            token_file.chmod(0o600)
            link = root / 'token-link'; link.symlink_to(token_file)
            rejected = subprocess.run([str(cli_binary), 'mcp', '--db', str(root / 'stdio.db'),
                                       '--mcp-token-file', str(link)], cwd=root, env=env,
                                      input='', capture_output=True, text=True, timeout=20)
            assert rejected.returncode != 0
            assert TOKENS['one'] not in rejected.stderr
            print('STDIO: public-mode and symlink credentials refused')
    finally:
        for process in processes:
            if process.poll() is None:
                process.terminate()
                try: process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    process.kill(); process.wait()
        log.close()
