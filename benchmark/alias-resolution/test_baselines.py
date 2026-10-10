import unittest

from baselines import id_control, input_hash, label_score


class FreeBaselines(unittest.TestCase):
    def test_normalization_and_edit_distance(self):
        self.assertEqual(label_score("Ａlpha__Beta", "alpha beta"), 1)
        self.assertAlmostEqual(label_score("kitten", "sitting"), 4 / 7)
        self.assertIsNone(label_score("", "a"))
        self.assertIsNone(label_score("---", "---"))

    def test_swapping_evidence_changes_cache_identity(self):
        a = {"left": {"label": "first"}, "right": {"label": "second"}}
        b = {"left": {"label": "first"}, "right": {"label": "third"}}
        self.assertNotEqual(input_hash(a), input_hash(b))
        self.assertEqual(input_hash(a), input_hash(dict(reversed(list(a.items())))))

    def test_identifier_control_requires_repo_and_unique_prefix(self):
        def record(repo, sha):
            return {"labels": [f"{repo}@{sha}"], "edges": []}

        a, b = record("repo", "abcdef0"), record("repo", "abcdef0" + "1" * 33)
        self.assertEqual(id_control(a, b, [a, b]), 1)
        collision = record("repo", "abcdef0" + "2" * 33)
        self.assertIsNone(id_control(a, b, [a, b, collision]))
        self.assertIsNone(id_control(a, record("other", "abcdef0"), [a, b]))
        self.assertIsNone(id_control(a, a, [a]))


if __name__ == "__main__":
    unittest.main()
