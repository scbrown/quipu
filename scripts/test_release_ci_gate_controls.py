#!/usr/bin/env python3
"""Prove combined shape coverage refuses missing, skipped and ignored checks.

# arming: ci Pre-commit checks; also agent-invoked with isolated fixture files.
"""
from pathlib import Path
import subprocess
import sys
import tempfile

root = Path(__file__).resolve().parents[1]
ci = (root / ".github/workflows/ci.yml").read_text()
release = (root / ".github/workflows/release.yml").read_text()
commands = (
    "python3 shapes/verify_shape_invariants.py --selftest",
    "python3 shapes/verify_shape_invariants.py",
)
cases = [("current combined job", ci, True)]
for command in commands:
    line = f"        run: {command}\n"
    assert line in ci
    cases.extend([
        (f"missing {command}", ci.replace(line, "", 1), False),
        (f"conditional {command}", ci.replace(line, "        if: false\n" + line, 1), False),
        (f"ignored failure {command}", ci.replace(line, line.rstrip() + " || true\n", 1), False),
        (f"commented {command}", ci.replace(line, "#" + line, 1), False),
    ])
needle = "  source-size:\n"
assert needle in ci
cases.append(("conditional combined job", ci.replace(needle, needle + "    if: false\n", 1), False))
needle = "needs: [extended-features, build-full, wasm, source-size, load-test, query-perf, lint-markdown]"
assert needle in ci
cases.append(("missing aggregate dependency", ci.replace(needle, needle.replace("source-size, ", ""), 1), False))
for name, fixture, expected in cases:
    with tempfile.TemporaryDirectory(prefix="release-contract-") as directory:
        path = Path(directory) / ".github/workflows"
        path.mkdir(parents=True)
        (path / "ci.yml").write_text(fixture)
        (path / "release.yml").write_text(release)
        result = subprocess.run(
            [sys.executable, str(root / "scripts/test_release_ci_gate.py")],
            cwd=directory, capture_output=True, text=True,
        )
        assert (result.returncode == 0) == expected, (name, result.stderr)
        print("PASS:", name)
print(f"{len(cases)} release-contract controls passed")
