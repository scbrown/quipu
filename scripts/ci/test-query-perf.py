#!/usr/bin/env python3
"""Offline controls: failed naive queries must not masquerade as speedups."""
import importlib.util
import json
from pathlib import Path
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('query_perf', ROOT / 'scripts/quipu-query-perf.py')
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
budget = json.loads((ROOT / 'benchmark/query-perf-budget.json').read_text())
rows = [{'s': 'http://example.org/perf/test', 'l': 'index.example'}]


def arm(name, response=None, *, no_speedup=False):
    defeated_calls = 0

    def query(url, sparql, timeout):
        nonlocal defeated_calls
        if '|| false' in sparql:
            defeated_calls += 1
            # Differential remains valid; only timed defeated calls fail.
            if defeated_calls > 1 and response:
                if name != 'partial-error' or defeated_calls == 3:
                    return response
            return (.001 if no_speedup else .010, 'ok', rows)
        if 'COUNT' in sparql:
            return (.001, 'ok', [{'n': '1'}])
        return (.001, 'ok', rows)

    with patch.object(module, 'run_query', side_effect=query):
        report, failures = module.gate('http://127.0.0.1:1', budget, 3, 30, ['Q3_label_regex'])
    result = report['classes']['Q3_label_regex']
    if name == 'healthy':
        assert not failures, failures
        assert result['ratio'] == 10
    else:
        assert failures, f'{name}: incorrectly accepted as green'
        if not no_speedup:
            assert result['ratio'] is None, f'{name}: invalid timing used in ratio'
    print(f'{name}: {"PASS valid" if not failures else "PASS rejected"}')


arm('healthy')
arm('timeout', (30, '408', None))
arm('malformed-null', (.001, 'ok', None))
arm('malformed-object', (.001, 'ok', {'error': 'invalid response'}))
arm('malformed-row', (.001, 'ok', ['not a binding']))
arm('empty', (.001, 'ok', []))
arm('partial-error', (30, '408', None))
arm('no-speedup', no_speedup=True)

# Validate the real HTTP decoder, not just the controlled query adapter.
class Response:
    def __init__(self, data):
        self.data = data
    def __enter__(self):
        return self
    def __exit__(self, *args):
        return False
    def read(self):
        return json.dumps(self.data).encode()

for payload, expected in [
    ({'rows': rows}, 'ok'),
    ({'rows': None}, 'malformed'),
    ({'rows': {'s': 'invalid'}}, 'malformed'),
    ({'rows': ['invalid']}, 'malformed'),
    ({'rows': rows, 'error': 'query failed'}, 'malformed'),
]:
    with patch.object(module.urllib.request, 'urlopen', return_value=Response(payload)):
        _, status, _ = module.run_query('http://127.0.0.1:1', 'SELECT ?s WHERE { ?s ?p ?o }', 1)
    assert status == expected, (payload, status)
print('HTTP decoder: 5 controls PASS')
