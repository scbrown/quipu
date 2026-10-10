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
# Key order is irrelevant to YAML: trailing keys govern the same step.
for command in commands:
    line = f"        run: {command}\n"
    for key in ("if: false", "continue-on-error: true", 'continue-on-error: "false"'):
        for order in ("before", "after"):
            extra = f"        {key}\n"
            replacement = extra + line if order == "before" else line + extra
            cases.append((f"{order} {key} {command}", ci.replace(line, replacement, 1), False))
    for order in ("before", "after"):
        extra = "        continue-on-error: false\n"
        replacement = extra + line if order == "before" else line + extra
        cases.append((f"safe boolean false {order} {command}", ci.replace(line, replacement, 1), True))
    duplicate = "        continue-on-error: true\n        continue-on-error: false\n"
    cases.append((f"duplicate step keys {command}", ci.replace(line, line + duplicate, 1), False))

name = "Self-test the invariant checker"
command = commands[0]
step = f"      - name: {name}\n        run: {command}\n"
assert step in ci
cases.append(("safe reordered step", ci.replace(step, f"      - run: {command}\n        name: {name}\n", 1), True))

needle = "  source-size:\n"
assert needle in ci
cases.append(("conditional combined job", ci.replace(needle, needle + "    if: false\n", 1), False))
needle = "needs: [extended-features, build-full, wasm, source-size, load-test, query-perf, lint-markdown]"
assert needle in ci
cases.append(("missing aggregate dependency", ci.replace(needle, needle.replace("source-size, ", ""), 1), False))
for job in ("source-size", "extended-correctness"):
    start = ci.index(f"  {job}:\n")
    cases.append((f"safe job false {job}", ci[:start] + ci[start:].replace(f"  {job}:\n", f"  {job}:\n    continue-on-error: false\n", 1), True))
    for key in ("continue-on-error: true", 'continue-on-error: "false"'):
        cases.append((f"ignored {job} {key}", ci[:start] + ci[start:].replace(f"  {job}:\n", f"  {job}:\n    {key}\n", 1), False))
# False is a safe boolean, but a duplicate true followed by false is refused.
start = ci.index("  source-size:\n")
cases.append(("duplicate job keys", ci[:start] + ci[start:].replace("  source-size:\n", "  source-size:\n    continue-on-error: true\n    continue-on-error: false\n", 1), False))
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
