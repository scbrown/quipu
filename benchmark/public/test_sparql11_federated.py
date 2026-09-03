import importlib.util
import pathlib
import sys
import tempfile
import unittest

MODULE_PATH = pathlib.Path(__file__).with_name("sparql11_federated.py")
SPEC = importlib.util.spec_from_file_location("sparql11_federated", MODULE_PATH)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader
sys.modules[SPEC.name] = MODULE
SPEC.loader.exec_module(MODULE)


class FederatedManifestTests(unittest.TestCase):
    def test_discovers_service_data(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            (root / "q.rq").write_text("SELECT * WHERE { SERVICE <http://e/query> { ?s ?p ?o } }")
            (root / "r.srx").write_text('<sparql xmlns="http://www.w3.org/2005/sparql-results#"><head/></sparql>')
            (root / "d.ttl").write_text("<s> <p> <o> .")
            (root / "manifest.ttl").write_text(''':x rdf:type mf:QueryEvaluationTest ; mf:name "x" ; dawgt:approval dawgt:Approved ; mf:action [ qt:query <q.rq> ; qt:serviceData [ qt:endpoint <http://e/query> ; qt:data <d.ttl> ] ] ; mf:result <r.srx> .''')
            cases = MODULE.discover(root / "manifest.ttl")
            self.assertEqual(cases[0][1], (("http://e/query", root / "d.ttl"),))


if __name__ == "__main__":
    unittest.main()
