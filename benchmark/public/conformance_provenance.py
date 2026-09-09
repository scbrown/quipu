"""Shared producer metadata for public conformance ledgers."""

import os
from datetime import datetime, timezone


def provenance() -> tuple[str, str]:
    """When this ledger was produced, and BY WHAT.

    `generated_at` alone is forgeable by accident: a local re-derive stamps it
    exactly as CI does, so a reader could not tell a page backed by the pinned
    runner from one backed by somebody's laptop. `generated_by` carries the CI
    run URL when GitHub Actions produced it and the literal "local" otherwise,
    which turns "only CI-produced ledgers go on the page" from a convention
    nobody can check into a property of the ARTIFACT (aegis-1gp76j).

    This is not hypothetical bookkeeping: a locally-run ledger takes
    `quipu_revision` from the repo HEAD and `quipu_version` from whatever binary
    was to hand, so it can credit a commit that never produced it.
    """
    at = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    server = os.environ.get("GITHUB_SERVER_URL")
    repo = os.environ.get("GITHUB_REPOSITORY")
    run_id = os.environ.get("GITHUB_RUN_ID")
    if server and repo and run_id:
        return at, f"{server}/{repo}/actions/runs/{run_id}"
    return at, "local"
