#!/usr/bin/env python3
"""Wait for CI's exact-SHA release correctness check, failing closed."""

from __future__ import annotations

import json
import os
import subprocess
import sys
import time
from typing import Any


CHECK_NAME = "Release correctness"
PENDING = 0
SUCCESS = 1
FAILURE = 2


def classify(check_runs: list[dict[str, Any]]) -> tuple[int, str]:
    """Classify the newest genuine GitHub Actions correctness check."""
    matches = [
        run for run in check_runs
        if run.get("name") == CHECK_NAME
        and (run.get("app") or {}).get("slug") == "github-actions"
    ]
    if not matches:
        return PENDING, "check has not appeared"
    latest = max(matches, key=lambda run: int(run.get("id", 0)))
    status = latest.get("status")
    conclusion = latest.get("conclusion")
    if status != "completed":
        return PENDING, f"check {latest.get('id')} is {status or 'pending'}"
    if conclusion == "success":
        return SUCCESS, f"check {latest.get('id')} passed"
    return FAILURE, f"check {latest.get('id')} concluded {conclusion or 'without a verdict'}"


def fetch(repo: str, sha: str) -> list[dict[str, Any]]:
    result = subprocess.run(
        ["gh", "api", f"repos/{repo}/commits/{sha}/check-runs?per_page=100"],
        text=True,
        capture_output=True,
        check=False,
    )
    if result.returncode != 0:
        raise RuntimeError(result.stderr.strip() or "gh api failed without an error message")
    payload = json.loads(result.stdout)
    return payload.get("check_runs", [])


def main() -> int:
    if len(sys.argv) != 2:
        print(f"usage: {sys.argv[0]} <commit-sha>", file=sys.stderr)
        return 2
    repo = os.environ.get("GITHUB_REPOSITORY", "")
    if not repo:
        print("GITHUB_REPOSITORY is required", file=sys.stderr)
        return 2
    sha = sys.argv[1]
    interval = float(os.environ.get("CI_GATE_INTERVAL_SECONDS", "10"))
    attempts = int(os.environ.get("CI_GATE_MAX_ATTEMPTS", "270"))

    for attempt in range(1, attempts + 1):
        try:
            state, detail = classify(fetch(repo, sha))
        except (RuntimeError, json.JSONDecodeError) as exc:
            state, detail = PENDING, f"could not read checks: {exc}"
        print(f"[{attempt}/{attempts}] {sha}: {detail}", flush=True)
        if state == SUCCESS:
            return 0
        if state == FAILURE:
            return 1
        if attempt < attempts:
            time.sleep(interval)

    print(f"timed out waiting for {CHECK_NAME!r} on exact SHA {sha}", file=sys.stderr)
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
