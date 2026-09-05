#!/usr/bin/env python3
"""The wasm acceptance must have ONE definition, not two that agree today.

## Why this exists (aegis-dsbt2g, from aegis-tpqccc)

`Wasm (browser bundle)` in ci.yml and `Wasm (release asset)` in release.yml ran
the same acceptance -- scripts/smoke-wasm-explorer.mjs -- from two separately
maintained job definitions. They had ALREADY diverged at the moment they were
written: the CI copy built the native binary and the release copy did not. The
smoke mints a share with the native binary and imports the browser's export back
with it, so the release job died with

    Error: spawnSync .../target/release/quipu ENOENT

AFTER v0.3.33 was tagged -- while the PR job was green on the identical commit.
The PR-side job existed precisely so the release job's first execution would not
be a live release. It was anyway, because what the PR job exercised was ci.yml's
environment, not release.yml's.

The first fix added the missing step and a comment at both sites asking readers
to keep them in step. That is the weakest possible fix for the class: it relies
on someone noticing a comment. The steps now live in
.github/actions/wasm-explorer-{build,smoke}, and this check is what stops the
duplication growing back.

## What it actually asserts

Not "the two jobs are identical" -- they legitimately differ, and one difference
is load-bearing: ci.yml smokes wasm-bindgen's output directory while release.yml
smokes the PACKAGED layout, which is the stronger check because only it can
catch a file missed by the packaging `cp` list.

What it asserts is that neither job carries its own INLINE copy of a shared
step. A step defined once cannot diverge; a step defined twice will, and this
repository has already paid for that once, on a live release.

## The proof the bead asked for

"Sabotage the shared definition (drop the native-binary step) and confirm BOTH
jobs go red." That is now true by construction -- there is one definition, so
removing a step removes it from both callers. This check covers the other
direction, which construction does NOT give you for free: someone adding an
inline step back to one workflow, which is exactly how the divergence arose the
first time.

Run: python3 scripts/check-wasm-workflow-shared.py
"""
from __future__ import annotations

import pathlib
import sys

import yaml

ROOT = pathlib.Path(__file__).resolve().parent.parent
BUILD_ACTION = "./.github/actions/wasm-explorer-build"
SMOKE_ACTION = "./.github/actions/wasm-explorer-smoke"

# The jobs that run the browser acceptance, by workflow and job id.
CALLERS = [
    (".github/workflows/ci.yml", "wasm-explorer"),
    (".github/workflows/release.yml", "wasm"),
]

# Fragments that mark a step as an inline copy of something the shared actions
# own. Each of these was duplicated across both workflows before dsbt2g.
FORBIDDEN_INLINE = [
    ("wasm-bindgen --target web", "builds the bundle"),
    ("cargo install wasm-bindgen-cli", "installs the pinned wasm-bindgen"),
    ("--bin quipu", "builds the native binary the smoke needs"),
    ("smoke-wasm-explorer.mjs", "runs the browser acceptance"),
    ("playwright install", "installs the browser the smoke drives"),
]


def steps_of(path: str, job_id: str):
    doc = yaml.safe_load((ROOT / path).read_text())
    jobs = doc.get("jobs") or {}
    if job_id not in jobs:
        raise SystemExit(f"FAIL: {path} has no job `{job_id}` "
                         f"(jobs: {sorted(jobs)})")
    return jobs[job_id].get("steps") or []


def main() -> int:
    failures: list[str] = []
    for path, job_id in CALLERS:
        steps = steps_of(path, job_id)
        uses = [s.get("uses") for s in steps if isinstance(s, dict)]

        for action, label in ((BUILD_ACTION, "build"), (SMOKE_ACTION, "smoke")):
            if action not in uses:
                failures.append(
                    f"{path}:{job_id} does not call the shared {label} action "
                    f"({action}). Two definitions of one acceptance is the "
                    f"dsbt2g defect; it shipped v0.3.33 with no wasm asset.")

        for step in steps:
            if not isinstance(step, dict):
                continue
            body = str(step.get("run") or "")
            for needle, what in FORBIDDEN_INLINE:
                if needle in body:
                    failures.append(
                        f"{path}:{job_id} step {step.get('name', '<unnamed>')!r} "
                        f"inlines `{needle}` -- that {what}, and the shared "
                        f"action owns it. Put the change in "
                        f".github/actions/wasm-explorer-*/action.yml so BOTH "
                        f"callers get it.")

    if failures:
        print("The wasm acceptance has drifted back into per-workflow copies:\n")
        for f in failures:
            print(f"  - {f}")
        print("\nSee aegis-dsbt2g. Running 'the same' acceptance in two places "
              "protects nothing\nwhen the setup around it differs -- the script "
              "was byte-identical and the\nenvironment was not.")
        return 1

    print(f"OK: {len(CALLERS)} wasm acceptance job(s) share one definition; "
          f"no inlined copies.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
