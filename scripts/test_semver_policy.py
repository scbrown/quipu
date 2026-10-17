#!/usr/bin/env python3
"""The exception must refuse the transitions that require its reversal."""

import importlib.util
import unittest
from copy import deepcopy
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "semver_policy", Path(__file__).with_name("check-semver-policy.py")
)
policy = importlib.util.module_from_spec(spec)
spec.loader.exec_module(policy)


class PolicyTests(unittest.TestCase):
    def setUp(self):
        self.manifest = {"package": {"name": "quipu-ai", "version": "0.14.1"}}
        self.config = {
            "workspace": {"publish": False},
            "package": [{"name": "quipu-ai", "semver_check": False}],
        }

    def test_private_pre_one_positive(self):
        self.assertEqual(policy.violations(self.manifest, self.config), [])

    def test_publishing_requires_reversal(self):
        for scope in [self.config["workspace"], self.config["package"][0]]:
            scope["publish"] = True
            self.assertTrue(policy.violations(self.manifest, self.config))
            del scope["publish"]

    def test_one_x_requires_reversal(self):
        for version in ["1.0.0", "1.0.0-rc.1", "2.0.0"]:
            self.manifest["package"]["version"] = version
            self.assertTrue(policy.violations(self.manifest, self.config))

    def test_reenabled_check_allows_published_one_x(self):
        self.manifest["package"]["version"] = "1.0.0"
        self.config["workspace"]["publish"] = True
        self.config["package"][0]["semver_check"] = True
        self.assertEqual(policy.violations(self.manifest, self.config), [])

    def test_global_disable_refuses(self):
        self.config["workspace"]["semver_check"] = False
        self.assertTrue(policy.violations(self.manifest, self.config))

    def test_unknown_package_cannot_borrow_exception(self):
        self.config["package"].append({"name": "other", "semver_check": False})
        self.assertTrue(policy.violations(self.manifest, self.config))

    def test_missing_or_inherited_version_is_unknown(self):
        for value in [None, {"workspace": True}, "unknown"]:
            manifest = deepcopy(self.manifest)
            manifest["package"]["version"] = value
            self.assertTrue(policy.violations(manifest, self.config))


if __name__ == "__main__":
    unittest.main()
