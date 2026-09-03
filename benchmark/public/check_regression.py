#!/usr/bin/env python3
"""Compare a fresh conformance ledger against the committed baseline.

The evaluation runner exits nonzero whenever anything fails, which is correct
for the runner and useless as a CI gate: at 18/168 it will exit nonzero on
every commit for as long as the work takes. That turns a red build into
background noise, and a real regression then arrives as one more red build
nobody reads.

This tool answers the different question CI needs: *is this worse than what we
published?* It exits nonzero only when a class's pass count drops or a test that
passed in the baseline stops passing, and it names those tests. Improvement
exits zero and prints the newly passing tests, because a stale baseline is the
next way a published number goes wrong.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

PASSED = "passed"


class LedgerError(RuntimeError):
    """A ledger is missing, malformed, or not comparable with the other."""


def load_rows(path: Path) -> dict[tuple[str, str, str], dict]:
    """Index a ledger by (class, manifest, id).

    Keyed on the manifest as well as the identifier: identifiers are local to
    their manifest in the W3C suite, so two families are free to reuse one. They
    do not today, and a key that only works today is how a comparison quietly
    starts comparing the wrong pair of tests.
    """
    try:
        document = json.loads(path.read_text())
    except FileNotFoundError as error:
        raise LedgerError(f"missing ledger: {path}") from error
    except json.JSONDecodeError as error:
        raise LedgerError(f"{path}: not valid JSON: {error}") from error

    rows = document.get("results")
    if not isinstance(rows, list) or not rows:
        raise LedgerError(f"{path}: no result rows")

    indexed: dict[tuple[str, str, str], dict] = {}
    for row in rows:
        try:
            key = (row["class"], row["manifest"], row["id"])
        except (KeyError, TypeError) as error:
            raise LedgerError(f"{path}: result row missing class/manifest/id: {row!r}") from error
        if key in indexed:
            raise LedgerError(f"{path}: duplicate result row for {key!r}")
        indexed[key] = row
    return indexed


def class_counts(rows: dict[tuple[str, str, str], dict]) -> dict[str, dict[str, int]]:
    counts: dict[str, dict[str, int]] = {}
    for (test_class, _, _), row in rows.items():
        bucket = counts.setdefault(test_class, {"passed": 0, "cases": 0})
        bucket["cases"] += 1
        if row.get("status") == PASSED:
            bucket["passed"] += 1
    return counts


def compare(baseline: dict, candidate: dict) -> dict:
    """Everything the caller needs to decide and to explain the decision."""
    regressed = sorted(
        key
        for key, row in baseline.items()
        if row.get("status") == PASSED
        and candidate.get(key, {}).get("status") != PASSED
    )
    improved = sorted(
        key
        for key, row in candidate.items()
        if row.get("status") == PASSED
        and baseline.get(key, {}).get("status") != PASSED
    )
    # A test present in the baseline and absent from the candidate is a
    # regression in its own right even if it was already failing: the suite
    # shrank, and a shrinking denominator raises every ratio for free.
    dropped = sorted(set(baseline) - set(candidate))
    added = sorted(set(candidate) - set(baseline))

    before = class_counts(baseline)
    after = class_counts(candidate)
    class_drops = sorted(
        name
        for name in before
        if after.get(name, {}).get("passed", 0) < before[name]["passed"]
    )
    return {
        "regressed": regressed,
        "improved": improved,
        "dropped": dropped,
        "added": added,
        "class_drops": class_drops,
        "before": before,
        "after": after,
    }


def describe(key: tuple[str, str, str]) -> str:
    test_class, manifest, identifier = key
    return f"{test_class} {identifier} ({manifest})"


def report(result: dict, stream=sys.stdout) -> None:
    before, after = result["before"], result["after"]
    print("class                pass (baseline -> candidate)   cases", file=stream)
    for name in sorted(set(before) | set(after)):
        was = before.get(name, {}).get("passed", 0)
        now = after.get(name, {}).get("passed", 0)
        cases = after.get(name, {}).get("cases", before.get(name, {}).get("cases", 0))
        arrow = "  " if was == now else ("UP" if now > was else "DOWN")
        print(f"{name:<20} {was:>4} -> {now:<4} {arrow:>6}          {cases:>5}", file=stream)

    for label, keys in (
        ("REGRESSED (passed in baseline, not in candidate)", result["regressed"]),
        ("DISAPPEARED (in baseline, absent from candidate)", result["dropped"]),
        ("newly passing", result["improved"]),
        ("new tests (absent from baseline)", result["added"]),
    ):
        if not keys:
            continue
        print(f"\n{label}: {len(keys)}", file=stream)
        for key in keys:
            print(f"  {describe(key)}", file=stream)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    args = parser.parse_args(argv)

    try:
        baseline = load_rows(args.baseline)
        candidate = load_rows(args.candidate)
    except LedgerError as error:
        print(f"check_regression: {error}", file=sys.stderr)
        return 2

    result = compare(baseline, candidate)
    report(result)

    failures = []
    if result["regressed"]:
        failures.append(f"{len(result['regressed'])} test(s) stopped passing")
    if result["dropped"]:
        failures.append(f"{len(result['dropped'])} test(s) disappeared from the suite")
    if result["class_drops"]:
        failures.append(f"pass count dropped in: {', '.join(result['class_drops'])}")

    if failures:
        print("\nREGRESSION: " + "; ".join(failures), file=sys.stderr)
        return 1

    if result["improved"] or result["added"]:
        print(
            "\nNo regression. The baseline is now behind the candidate — regenerate the"
            "\ncommitted ledger and page so the published numbers match reality."
        )
    else:
        print("\nNo regression; no change.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
