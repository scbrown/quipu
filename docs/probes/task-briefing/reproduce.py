"""Run the real candidate CLI against an isolated synthetic fixture, never a server."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

binary = str(Path(sys.argv[1]).resolve())
with tempfile.TemporaryDirectory(prefix='briefing-demo-') as tmp:
    root = Path(tmp)
    env = dict(os.environ, HOME=str(root))
    data = ('<urn:task:one> <urn:status> "closed" .\n'
            '<urn:task:one> <urn:owner> <urn:alice> .\n'
            '<urn:task:similar> <urn:status> "open" .\n')
    (root/'fixture.nt').write_text(data)
    (root/'.bobbin').mkdir()
    config = root/'.bobbin/config.toml'
    def run(*args):
        return subprocess.run([binary, *args], cwd=root, env=env,
                              capture_output=True, text=True)
    seed = run('ingest', 'fixture.nt', '--graph', 'urn:demo', '--timestamp',
               '2026-01-01T00:00:00Z', '--declare-count', '3',
               '--declare-sha256', hashlib.sha256(data.encode()).hexdigest(),
               '--db', 'fixture.db')
    assert seed.returncode == 0, seed.stderr
    (root/'request.json').write_text(json.dumps({'task_iri':'urn:task:one','graph':'urn:demo'}))
    off = run('task-briefing', 'request.json', '--db', 'fixture.db')
    assert off.returncode != 0 and 'disabled' in off.stderr, off.stderr
    print('DEFAULT OFF: explicit request refused (no briefing).', flush=True)
    config.write_text('[quipu.search]\ntask_briefing=true\n')
    before = hashlib.sha256((root/'fixture.db').read_bytes()).hexdigest()
    on = run('task-briefing', 'request.json', '--db', 'fixture.db')
    assert on.returncode == 0, on.stderr
    reply = json.loads(on.stdout)
    assert reply['outcome'] == 'CONTEXT'
    print('EXPLICIT OPT-IN:', reply['outcome'], flush=True)
    print(reply['first_screen'], flush=True)
    (root/'request.json').write_text(json.dumps({'task_iri':'urn:task:on','graph':'urn:demo'}))
    absent = run('task-briefing', 'request.json', '--db', 'fixture.db')
    assert absent.returncode == 0 and json.loads(absent.stdout)['outcome'] == 'NO_CONTEXT'
    assert hashlib.sha256((root/'fixture.db').read_bytes()).hexdigest() == before
    print('ABSENT EXACT ID: NO_CONTEXT; similar task never substitutes; database bytes unchanged.', flush=True)
    print('Synthetic fixture only. No live usefulness or current-policy acceptance.', flush=True)
