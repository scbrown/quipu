"""Replay extraction uses the shared credential contract and keeps public reads open."""
import importlib.util
from pathlib import Path
import os
import tempfile
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import unittest
from unittest import mock

spec = importlib.util.spec_from_file_location('replay_corpus', Path(__file__).with_name('build-replay-corpus.py'))
corpus = importlib.util.module_from_spec(spec)
spec.loader.exec_module(corpus)


class Credentials(unittest.TestCase):
    def test_precedence_and_no_implicit_legacy(self):
        with tempfile.TemporaryDirectory() as home, mock.patch.dict(os.environ, {'HOME': home}, clear=True):
            legacy = Path(home) / '.config/aegis/quipu_token'
            legacy.parent.mkdir(parents=True)
            legacy.write_text('legacy-fixture')
            self.assertIsNone(corpus.client_token())
            canonical = Path(home) / '.config/quipu/token'
            canonical.parent.mkdir()
            canonical.write_text('canonical-fixture\n')
            self.assertEqual(corpus.client_token(), 'canonical-fixture')
            os.environ['QUIPU_AUTH_TOKEN_FILE'] = str(legacy)
            self.assertEqual(corpus.client_token(), 'legacy-fixture')
            os.environ['QUIPU_AUTH_TOKEN'] = '  inline-fixture \n'
            self.assertEqual(corpus.client_token(), 'inline-fixture')
            os.environ['QUIPU_AUTH_TOKEN'] = ' \n'
            os.environ['QUIPU_AUTH_TOKEN_FILE'] = str(Path(home) / 'absent')
            self.assertIsNone(corpus.client_token())
            os.environ['QUIPU_AUTH_TOKEN'] = 'private-fixture\ninjected-header'
            with self.assertRaises(SystemExit) as caught:
                corpus.client_token()
            self.assertNotIn('private-fixture', str(caught.exception))

    def test_public_read_and_definite_401_are_distinct(self):
        seen = []
        class Handler(BaseHTTPRequestHandler):
            def do_POST(self):
                self.rfile.read(int(self.headers['Content-Length']))
                seen.append(self.headers.get('Authorization'))
                self.send_response(401 if seen[-1] else 200)
                self.end_headers()
                self.wfile.write(b'private-response-fixture' if seen[-1] else b'{"rows": [{"s": "fixture"}]}')
            def log_message(self, *args):
                pass
        server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        url = f'http://127.0.0.1:{server.server_port}'
        try:
            self.assertEqual(corpus.query(url, 'control'), [{'s': 'fixture'}])
            with self.assertRaises(SystemExit) as caught:
                corpus.query(url, 'control', 'private-token-fixture')
            self.assertIn('credential rejected (HTTP 401)', str(caught.exception))
            self.assertNotIn('private-token-fixture', str(caught.exception))
            self.assertNotIn('private-response-fixture', str(caught.exception))
            self.assertEqual(len(seen), 2)
        finally:
            server.shutdown()
            server.server_close()
            thread.join()


if __name__ == '__main__':
    unittest.main()
