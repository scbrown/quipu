#!/usr/bin/env python3
"""Bound the release latency exception to the private 0.x quipu package."""

import re
import sys
import tomllib
from pathlib import Path


def violations(manifest: dict, config: dict) -> list[str]:
    errors = []
    workspace = config.get("workspace", {})
    if workspace.get("semver_check") is False:
        errors.append("workspace-wide semver_check=false is not permitted")
    for package in config.get("package", []):
        if package.get("semver_check") is not False:
            continue
        actual = manifest.get("package", {})
        private = package.get("publish", workspace.get("publish")) is False
        pre_one = re.fullmatch(
            r"0\.\d+\.\d+(?:[-+][0-9A-Za-z.+-]+)?", str(actual.get("version", ""))
        )
        if (
            package.get("name") != "quipu-ai"
            or actual.get("name") != "quipu-ai"
            or not private
            or not pre_one
        ):
            errors.append(
                "Restore semver_check=true before publishing quipu-ai, reaching 1.x, or extending the exception"
            )
    return errors


def main() -> int:
    try:
        manifest = tomllib.loads(Path("Cargo.toml").read_text())
        config = tomllib.loads(Path("release-plz.toml").read_text())
        errors = violations(manifest, config)
    except (OSError, ValueError, TypeError, AttributeError) as error:
        print(f"semver policy unreadable: {type(error).__name__}", file=sys.stderr)
        return 1
    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1
    print("semver policy: scoped private 0.x exception or normal checks")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
