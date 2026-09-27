#!/usr/bin/env python3
"""Drive a stdio MCP command the way a registry client does, twice.

    usage: mcp-stdio-smoke.py -- <command> [args...]

For the OCI image the command is the exact line VS Code writes into mcp.json
after an Install click (`docker run -i --rm -v <volume>:/data <image>`), so a
pass means a stranger's click works, not merely that the binaries run.

It starts the command TWICE on purpose. The first run proves a brand-new store
answers (initialize, tools/list, an empty query, one write, the write read
back). The second run proves the write outlived the process. For the image that
second half is the whole point of the volume: without it every restart of the
editor would silently hand the agent an empty graph, and nothing in a single
run can see that.
"""
import json
import subprocess
import sys

# Fewer than this means the server came up as something other than the full
# MCP surface (46 tools, 48 with owl; see README) - e.g. a stub or wrong build.
MIN_TOOLS = 40
FACT = '<urn:quipu-smoke:subject> <urn:quipu-smoke:p> "registry smoke" .'
QUERY = 'SELECT ?o WHERE { <urn:quipu-smoke:subject> <urn:quipu-smoke:p> ?o }'


def session(command, *, write):
    proc = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                            text=True)
    ident = 0

    def send(method, params=None, notify=False):
        nonlocal ident
        message = {'jsonrpc': '2.0', 'method': method, 'params': params or {}}
        if not notify:
            ident += 1
            message['id'] = ident
        proc.stdin.write(json.dumps(message) + '\n')
        proc.stdin.flush()
        if notify:
            return None
        line = proc.stdout.readline()
        if not line:
            raise SystemExit(f'{method}: the server closed stdout without answering')
        reply = json.loads(line)
        if 'error' in reply:
            raise SystemExit(f'{method}: {reply["error"]}')
        return reply['result']

    def call(name, arguments):
        result = send('tools/call', {'name': name, 'arguments': arguments})
        if result.get('isError'):
            raise SystemExit(f'{name} failed: {result["content"]}')
        return json.loads(result['content'][0]['text'])

    try:
        info = send('initialize', {'protocolVersion': '2025-03-26', 'capabilities': {},
                                   'clientInfo': {'name': 'registry-smoke', 'version': '1'}})
        if info['serverInfo']['name'] != 'quipu':
            raise SystemExit(f'unexpected server {info["serverInfo"]}')
        send('notifications/initialized', notify=True)
        tools = send('tools/list')['tools']
        if len(tools) < MIN_TOOLS:
            raise SystemExit(f'only {len(tools)} tools listed; expected at least {MIN_TOOLS}')
        before = call('quipu_query', {'query': QUERY})['count']
        if write:
            if before != 0:
                raise SystemExit(f'a NEW store already held {before} matching facts')
            call('quipu_knot', {'turtle': FACT})
            after = call('quipu_query', {'query': QUERY})['count']
            if after != 1:
                raise SystemExit(f'wrote one fact, read back {after}')
            print(f'first start: {len(tools)} tools, empty store, write read back')
        else:
            if before != 1:
                raise SystemExit(f'after a restart the store held {before} facts, expected 1: '
                                 'the database did not persist')
            print('restart: the write persisted')
    finally:
        proc.stdin.close()
        code = proc.wait(timeout=30)
    if code != 0:
        raise SystemExit(f'the server exited {code} on stdin EOF')


def main(argv):
    if len(argv) < 2 or argv[0] != '--':
        print(__doc__.split('\n\n')[1], file=sys.stderr)
        return 2
    session(argv[1:], write=True)
    session(argv[1:], write=False)
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
