import importlib.util
import pathlib
import sys
import tempfile
import unittest

PATH = pathlib.Path(__file__).with_name("shacl_core.py")
SPEC = importlib.util.spec_from_file_location("shacl_core", PATH)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader
sys.modules[SPEC.name] = MODULE
SPEC.loader.exec_module(MODULE)


class ShaclHarnessTests(unittest.TestCase):
    def test_balanced_preserves_nested_property_lists(self):
        self.assertEqual(MODULE.balanced('x [ a [ " ] " ] ; b 1 ] z', 2), '[ a [ " ] " ] ; b 1 ]')

    def test_manifest_cycle_is_inventory_error(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            (root / "manifest.ttl").write_text("<> mf:include <manifest.ttl> .")
            with self.assertRaisesRegex(MODULE.HarnessError, "cycle"):
                MODULE.walk_manifests(root)

    def test_category_mapping_is_exclusive(self):
        self.assertEqual(MODULE.category_of(pathlib.Path("core/misc/a.ttl")), "core-complex-misc")
        self.assertEqual(MODULE.category_of(pathlib.Path("core/node/a.ttl")), "core-node")
        self.assertEqual(MODULE.category_of(pathlib.Path("sparql/node/a.ttl")), "shacl-sparql")


if __name__ == "__main__":
    unittest.main()
