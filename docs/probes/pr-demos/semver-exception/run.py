"""Exercise the actual policy entrypoint using only temporary TOML fixtures."""

import subprocess
import sys
import tempfile
from pathlib import Path

script = Path("scripts/check-semver-policy.py").resolve()
with tempfile.TemporaryDirectory() as directory:
    root = Path(directory)
    for version, publish, semver, expected in [
        ("0.14.1", False, False, 0),
        ("0.14.1", True, False, 1),
        ("1.0.0", False, False, 1),
        ("1.0.0", True, True, 0),
    ]:
        (root / "Cargo.toml").write_text(
            f'[package]\nname="quipu-ai"\nversion="{version}"\n'
        )
        (root / "release-plz.toml").write_text(
            f"[workspace]\npublish={str(publish).lower()}\n"
            f'[[package]]\nname="quipu-ai"\nsemver_check={str(semver).lower()}\n'
        )
        result = subprocess.run(
            [sys.executable, str(script)], cwd=root, capture_output=True, text=True, check=False
        )
        print(
            f"version={version} publish={publish} semver_check={semver}: exit={result.returncode}"
        )
        print((result.stdout + result.stderr).strip())
        assert result.returncode == expected
