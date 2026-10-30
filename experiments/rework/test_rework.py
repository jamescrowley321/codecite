"""Tests for rework.py. Run: python3 -m unittest experiments/rework/test_rework.py"""

import sys
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import rework  # noqa: E402
from rework import LensResult  # noqa: E402


def fixture(name):
    return (HERE / "fixtures" / name).read_text()


REVIEWS = [
    "combined.md",
    "combined-local.md",
    "combined-listing.md",
    "combined-must-only.md",
    "per-lens-dash.md",
    "per-lens-colon.md",
    "per-lens-json.md",
    "per-lens-bare.md",
]


class IsReview(unittest.TestCase):
    def test_every_review_fixture_is_a_review(self):
        for name in REVIEWS:
            with self.subTest(name):
                self.assertTrue(rework.is_review(fixture(name)))

    def test_an_unrelated_comment_is_not(self):
        self.assertFalse(rework.is_review(fixture("unrelated.md")))
        self.assertFalse(rework.is_review("Ran the blind peer review again.\n"))


class ParseReview(unittest.TestCase):
    def by_lens(self, name):
        return {r.lens: r for r in rework.parse_review(fixture(name))}

    def test_combined_table_with_severity_columns(self):
        lenses = self.by_lens("combined.md")
        self.assertEqual(len(lenses), 8)
        self.assertEqual(
            lenses["Cold Read"], LensResult("Cold Read", "PASS", 0, 3, 3, None)
        )
        self.assertEqual(lenses["Red Team"].should, 1)
        self.assertEqual(lenses["OWASP Web Top 10"].verdict, "PASS")

    def test_combined_table_with_finding_totals_only(self):
        lenses = self.by_lens("combined-local.md")
        self.assertEqual(len(lenses), 8)
        self.assertEqual(
            lenses["Policy & Provenance"],
            LensResult("Policy & Provenance", "PASS", None, None, None, 0),
        )

    def test_combined_table_with_must_fix_only(self):
        lenses = self.by_lens("combined-must-only.md")
        self.assertEqual(
            lenses["Policy & Provenance"],
            LensResult("Policy & Provenance", "BLOCK", 1, None, None, None),
        )
        self.assertEqual(lenses["OWASP LLM Top 10"].verdict, "PASS")

    def test_combined_listing_in_a_code_block(self):
        lenses = self.by_lens("combined-listing.md")
        self.assertEqual(len(lenses), 8)
        self.assertEqual(
            lenses["Cold Read"], LensResult("Cold Read", "PASS", 0, None, None, 1)
        )

    def test_per_lens_dash_spelling(self):
        self.assertEqual(
            rework.parse_review(fixture("per-lens-dash.md")),
            [LensResult("Cold Read", "PASS", 0, None, None, 0)],
        )

    def test_per_lens_colon_spelling_with_a_finding(self):
        self.assertEqual(
            rework.parse_review(fixture("per-lens-colon.md")),
            [LensResult("Cold Read", "BLOCK", 1, None, None, 1)],
        )

    def test_per_lens_bare_verdict(self):
        self.assertEqual(
            rework.parse_review(fixture("per-lens-bare.md")),
            [LensResult("Cold Read", "PASS", None, None, None, 0)],
        )

    def test_per_lens_json(self):
        self.assertEqual(
            rework.parse_review(fixture("per-lens-json.md")),
            [LensResult("Acceptance Criteria", "PASS", 0, 0, 0, 0)],
        )

    def test_per_lens_json_counts_severities(self):
        body = (
            "# Blind peer review — policy (codex, out-of-host)\n\n```json\n"
            '{"lens": "Policy & Provenance", "findings": ['
            '{"severity": "MUST FIX"}, {"severity": "SHOULD FIX"},'
            '{"severity": "NITPICK"}]}\n```\n'
        )
        self.assertEqual(
            rework.parse_review(body),
            [LensResult("Policy & Provenance", "BLOCK", 1, 1, 1, 3)],
        )

    def test_a_verdict_without_lenses_is_one_overall_result(self):
        body = "## Blind peer review — PR #4\n\n### Verdict: **BLOCK**\n"
        self.assertEqual(
            rework.parse_review(body),
            [LensResult(rework.ALL, "BLOCK", None, None, None, None)],
        )

    def test_lens_aliases(self):
        self.assertEqual(rework.lens_name("edge_case"), "Edge Cases")
        self.assertEqual(rework.lens_name("OWASP LLM"), "OWASP LLM Top 10")
        self.assertEqual(rework.lens_name("**Policy & Provenance**"), "Policy & Provenance")
        self.assertIsNone(rework.lens_name("Summary"))


if __name__ == "__main__":
    unittest.main()
