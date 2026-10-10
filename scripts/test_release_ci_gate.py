#!/usr/bin/env python3
"""Static contract connecting CI correctness to Release."""

from pathlib import Path
import yaml


ci = Path(".github/workflows/ci.yml").read_text()
release = Path(".github/workflows/release.yml").read_text()

assert "  release-correctness:\n" in ci
assert "    name: Release correctness\n" in ci
assert "needs: [clippy, test, build, check]" in ci
aggregate_header = ci[ci.index("  release-correctness:\n") :].split("    runs-on:", 1)[0]
for excluded in ("source-size", "fmt", "lint-markdown", "build-full", "wasm", "shapes"):
    assert excluded not in aggregate_header, f"housekeeping job {excluded} must not gate release"

assert "  extended-correctness:\n" in ci
assert "    name: Extended correctness\n" in ci
extended_header = ci[ci.index("  extended-correctness:\n") :].split("    runs-on:", 1)[0]
for required in (
    "extended-features",
    "build-full",
    "wasm",
    "source-size",
    "load-test",
    "lint-markdown",
):
    assert required in extended_header, f"extended surface {required} must remain covered"

# Reject duplicate keys rather than letting a later value hide a safety gate.
class UniqueSafeLoader(yaml.SafeLoader):
    def construct_mapping(self, node, deep=False):
        self.flatten_mapping(node)
        keys = [self.construct_object(key, deep=deep) for key, _ in node.value]
        assert len(keys) == len(set(keys)), "duplicate workflow mapping keys"
        return super().construct_mapping(node, deep=deep)


jobs = yaml.load(ci, Loader=UniqueSafeLoader)["jobs"]
combined = jobs["source-size"]
assert "if" not in combined, "combined invariants must not be conditional"
assert combined.get("continue-on-error", False) is False, "combined failures must gate"
aggregate = jobs["extended-correctness"]
assert "source-size" in aggregate["needs"], "combined invariants must gate the aggregate"
assert aggregate.get("continue-on-error", False) is False, "aggregate failures must gate"
for name, command in (
    ("Self-test the invariant checker", "python3 shapes/verify_shape_invariants.py --selftest"),
    ("Verify shape invariants (static)", "python3 shapes/verify_shape_invariants.py"),
):
    matches = [step for step in combined["steps"] if step.get("name") == name]
    assert len(matches) == 1, f"exactly one shape step required: {name}"
    step = matches[0]
    assert step.get("run", "").strip() == command, f"shape command missing: {name}"
    assert "if" not in step, f"shape step must be unconditional: {name}"
    assert step.get("continue-on-error", False) is False, f"shape failure must gate: {name}"

assert "  ci-correctness:\n" in release
assert "python3 scripts/test_wait_release_correctness.py" in release
assert "python3 scripts/test_release_ci_gate.py" in release
assert 'python3 scripts/wait_release_correctness.py "$GITHUB_SHA"' in release
release_plz = release[release.index("  release-plz:\n") :]
assert "    needs: ci-correctness\n" in release_plz.split("    steps:\n", 1)[0]
assert "  workflow_dispatch:\n" in release, "manual artifact recovery must remain available"

print("release CI gate contract: ok")
