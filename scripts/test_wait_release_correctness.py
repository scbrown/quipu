#!/usr/bin/env python3
"""Both-direction unit tests for the cross-workflow release gate."""

import unittest

from wait_release_correctness import FAILURE, PENDING, SUCCESS, classify


def check(
    ident: int,
    status: str = "completed",
    conclusion: str | None = "success",
    name: str = "Release correctness",
    app: str = "github-actions",
) -> dict:
    return {
        "id": ident,
        "name": name,
        "status": status,
        "conclusion": conclusion,
        "app": {"slug": app},
    }


class ClassifyTests(unittest.TestCase):
    def test_absent_is_pending(self):
        self.assertEqual(classify([])[0], PENDING)

    def test_in_progress_is_pending(self):
        self.assertEqual(classify([check(1, "in_progress", None)])[0], PENDING)

    def test_success_passes(self):
        self.assertEqual(classify([check(1)])[0], SUCCESS)

    def test_failure_refuses(self):
        self.assertEqual(classify([check(1, conclusion="failure")])[0], FAILURE)

    def test_spoofed_check_name_is_ignored(self):
        self.assertEqual(classify([check(1, app="external-ci")])[0], PENDING)

    def test_newest_rerun_is_authoritative(self):
        runs = [check(10, conclusion="failure"), check(11)]
        self.assertEqual(classify(runs)[0], SUCCESS)


if __name__ == "__main__":
    unittest.main()
