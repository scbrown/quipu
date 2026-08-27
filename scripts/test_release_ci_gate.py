#!/usr/bin/env python3
"""Static contract connecting CI correctness to Release."""

from pathlib import Path


ci = Path(".github/workflows/ci.yml").read_text()
release = Path(".github/workflows/release.yml").read_text()

assert "  release-correctness:\n" in ci
assert "    name: Release correctness\n" in ci
assert "needs: [clippy, test, build, build-full, wasm, check, shapes]" in ci
aggregate_header = ci[ci.index("  release-correctness:\n") :].split("    runs-on:", 1)[0]
for excluded in ("source-size", "fmt", "lint-markdown"):
    assert excluded not in aggregate_header, f"housekeeping job {excluded} must not gate release"

assert "  ci-correctness:\n" in release
assert "python3 scripts/test_wait_release_correctness.py" in release
assert "python3 scripts/test_release_ci_gate.py" in release
assert 'python3 scripts/wait_release_correctness.py "$GITHUB_SHA"' in release
release_plz = release[release.index("  release-plz:\n") :]
assert "    needs: ci-correctness\n" in release_plz.split("    steps:\n", 1)[0]
assert "  workflow_dispatch:\n" in release, "manual artifact recovery must remain available"

print("release CI gate contract: ok")
