#!/usr/bin/env python3
"""Replay qpack merge from immutable Git parents; no local merge driver needed."""
import argparse
import json
import os
import subprocess
from pathlib import Path


def git(*args):
    return subprocess.check_output(["git", *args], text=True).strip()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", default="target/debug/quipu")
    args = parser.parse_args()
    event_name = os.environ.get("GITHUB_EVENT_NAME", "")
    result = git("rev-parse", "HEAD")
    if event_name == "pull_request":
        event = json.loads(Path(os.environ["GITHUB_EVENT_PATH"]).read_text())
        theirs = event["pull_request"]["head"]["sha"]
        parents = git("rev-list", "--parents", "-n", "1", result).split()
        if len(parents) != 3 or parents[2] != theirs:
            raise SystemExit("refusing: checkout must be the PR's synthetic merge commit")
        ours = parents[1]
    elif event_name == "push":
        event = json.loads(Path(os.environ["GITHUB_EVENT_PATH"]).read_text())
        ours = event["before"]
        if set(ours) == {"0"}:
            ours = result
        theirs = result
    elif event_name in ("schedule", "workflow_dispatch"):
        ours = theirs = result
    else:
        raise SystemExit("refusing: expected a supported GitHub event, not an unbound check")
    base = git("merge-base", "--all", ours, theirs)
    if "\n" in base:
        raise SystemExit("refusing: ambiguous merge base")
    # Replaying only the synthetic PR merge misses unsafe text merges already
    # committed on the topic branch: the new base may be an ancestor of its tip.
    for merge in git("rev-list", "--reverse", "--merges", f"{ours}..{theirs}").splitlines():
        parents = git("rev-list", "--parents", "-n", "1", merge).split()
        if len(parents) != 3:
            raise SystemExit("refusing: qpack verification requires two-parent merges")
        left, right = parents[1:]
        ancestor = git("merge-base", "--all", left, right)
        if "\n" in ancestor:
            raise SystemExit("refusing: ambiguous historical merge base")
        subprocess.run([args.binary, "pendant-check", ancestor, left, right, merge], check=True)
    subprocess.run([args.binary, "pendant-check", base, ours, theirs, result], check=True)


if __name__ == "__main__":
    main()
