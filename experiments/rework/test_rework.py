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
        self.assertEqual(
            rework.lens_name("**Policy & Provenance**"), "Policy & Provenance"
        )
        self.assertIsNone(rework.lens_name("Summary"))


def lens(name, verdict="PASS", must=None, should=None, nit=None, total=None):
    return LensResult(name, verdict, must, should, nit, total)


EIGHT = list(rework.LENSES)


class Rounds(unittest.TestCase):
    def test_a_combined_block_then_pass_is_two_rounds(self):
        reviews = [
            ("2026-10-02T00:00:00Z", [lens("Cold Read"), lens("Red Team")]),
            ("2026-10-01T00:00:00Z", [lens("Cold Read", "BLOCK", 1), lens("Red Team")]),
        ]
        rounds = rework.rounds(reviews)
        self.assertEqual(len(rounds), 2)
        self.assertEqual(rounds[0][0].verdict, "BLOCK")
        self.assertEqual(rework.rounds_to_pass(rounds), 2)

    def test_per_lens_comments_start_a_new_round_when_a_lens_repeats(self):
        reviews = [
            (f"2026-10-01T00:0{i}:00Z", [lens(name, "BLOCK" if i == 0 else "PASS")])
            for i, name in enumerate(EIGHT)
        ]
        reviews.append(("2026-10-02T00:00:00Z", [lens("Cold Read")]))
        rounds = rework.rounds(reviews)
        self.assertEqual([len(r) for r in rounds], [8, 1])
        self.assertEqual(rework.rounds_to_pass(rounds), 2)

    def test_a_combined_comment_ends_a_per_lens_run(self):
        reviews = [
            ("2026-10-01T00:00:00Z", [lens("Cold Read", "BLOCK")]),
            ("2026-10-02T00:00:00Z", [lens(rework.ALL)]),
        ]
        self.assertEqual(len(rework.rounds(reviews)), 2)

    def test_only_block_rounds_never_pass(self):
        reviews = [
            ("2026-10-01T00:00:00Z", [lens(rework.ALL, "BLOCK")]),
            ("2026-10-02T00:00:00Z", [lens(rework.ALL, "BLOCK")]),
        ]
        self.assertIsNone(rework.rounds_to_pass(rework.rounds(reviews)))

    def test_no_reviews_is_no_rounds(self):
        self.assertEqual(rework.rounds([]), [])
        self.assertIsNone(rework.rounds_to_pass([]))


class Violations(unittest.TestCase):
    def test_spec_and_convention_sum_must_and_should_across_rounds(self):
        rounds = [
            [
                lens("Acceptance Criteria", "BLOCK", 1, 2, 5),
                lens("Policy & Provenance", "PASS", 0, 1, 3),
                lens("Cold Read", "PASS", 0, 4, 9),
            ],
            [lens("Acceptance Criteria", "PASS", 0, 1, 0)],
        ]
        self.assertEqual(
            rework.violations(rounds), {"spec": 4, "convention": 1, "all": 9}
        )

    def test_total_stands_in_when_no_severity_is_given(self):
        rounds = [
            [
                lens("Acceptance Criteria", "BLOCK", total=3),
                lens("Policy & Provenance", "BLOCK", must=1),
                lens(rework.ALL, "BLOCK"),
            ]
        ]
        self.assertEqual(
            rework.violations(rounds), {"spec": 3, "convention": 1, "all": 4}
        )


class Plans(unittest.TestCase):
    def test_plan_paths_keeps_markdown_under_a_plans_directory(self):
        files = [
            "docs/superpowers/plans/x.md",
            "docs/plans/y.md",
            "docs/design.md",
            "plans/x.txt",
            "src/plans.md",
        ]
        self.assertEqual(
            rework.plan_paths(files),
            ["docs/superpowers/plans/x.md", "docs/plans/y.md"],
        )

    def test_citations_in_every_form(self):
        for text in [
            "see codecite@56fc2dd:crates/a.rs#L10-L42 for the loop",
            "notes@sha256:1a2b3c4d5e6f#L3-L9",
            "(rfc9449@sha256:2d8657f07291#section-4.3)",
            "`house-rules@sha256:1a2b3c4d5e6f#section-4`",
        ]:
            with self.subTest(text=text):
                self.assertTrue(rework.has_citation(text))

    def test_look_alikes_are_not_citations(self):
        for text in [
            "mail james@example.com",
            "scp user@host:path/to/file",
            "lines #L10 to #L42",
            "git@github.com:owner/repo.git#L1",
        ]:
            with self.subTest(text=text):
                self.assertFalse(rework.has_citation(text))

    def test_plan_text_is_none_when_the_head_is_gone(self):
        def gone(args):
            raise rework.GhError("gh: Not Found (HTTP 404)")

        self.assertIsNone(rework.plan_text("o/r", "plans/x.md", "abc", gh=gone))

    def test_plan_text_asks_for_the_raw_file_at_the_head(self):
        seen = []

        def ok(args):
            seen.append(args)
            return "# Plan\n"

        self.assertEqual(
            rework.plan_text("o/r", "plans/x.md", "abc", gh=ok), "# Plan\n"
        )
        self.assertIn("repos/o/r/contents/plans/x.md?ref=abc", seen[0])
        self.assertIn("Accept: application/vnd.github.raw", seen[0])


def pr(number, comments=(), reviews=(), files=(), head="abc"):
    return {
        "number": number,
        "headRefOid": head,
        "files": [{"path": f} for f in files],
        "comments": [{"createdAt": t, "body": b} for t, b in comments],
        "reviews": [{"submittedAt": t, "body": b} for t, b in reviews],
    }


BLOCK = "## Blind peer review — PR #1\n\nVerdict: BLOCK\n"
PASS = "## Blind peer review — PR #1\n\nVerdict: PASS\n"


class PrRow(unittest.TestCase):
    def test_reviews_come_from_comments_and_review_bodies_in_time_order(self):
        row = rework.pr_row(
            "o/r",
            pr(
                7,
                comments=[
                    ("2026-01-02T00:00:00Z", PASS),
                    ("2026-01-01T09:00:00Z", "hi"),
                ],
                reviews=[("2026-01-01T10:00:00Z", BLOCK)],
            ),
            plans=lambda *a: None,
        )
        self.assertEqual(row["number"], 7)
        self.assertTrue(row["reviewed"])
        self.assertEqual(row["rounds_to_pass"], 2)
        self.assertEqual(row["group"], rework.NO_PLAN)

    def test_a_pr_without_review_comments_is_unreviewed(self):
        row = rework.pr_row("o/r", pr(1, comments=[("t", "lgtm")]), plans=None)
        self.assertFalse(row["reviewed"])
        self.assertIsNone(row["rounds_to_pass"])

    def test_plan_groups(self):
        texts = {
            "docs/plans/a.md": "see codecite@56fc2dd:src/a.rs#L1-L9",
            "docs/plans/b.md": "no citations here",
        }

        def reader(repo, path, ref):
            self.assertEqual((repo, ref), ("o/r", "abc"))
            return texts.get(path)

        for files, group, unreadable in [
            (["docs/plans/a.md", "docs/plans/b.md"], rework.CITED, 0),
            (["docs/plans/b.md"], rework.UNCITED, 0),
            (["docs/plans/b.md", "docs/plans/gone.md"], rework.UNCITED, 1),
            (["docs/plans/gone.md"], rework.UNCITED, 1),
            (["src/main.rs"], rework.NO_PLAN, 0),
        ]:
            with self.subTest(files=files):
                row = rework.pr_row("o/r", pr(1, files=files), plans=reader)
                self.assertEqual(row["group"], group)
                self.assertEqual(row["unreadable"], unreadable)


def row(repo, group, rounds_to_pass, spec=0, convention=0, all_=0, reviewed=True):
    return {
        "repo": repo,
        "number": 1,
        "group": group,
        "reviewed": reviewed,
        "unreadable": 0,
        "rounds_to_pass": rounds_to_pass,
        "spec": spec,
        "convention": convention,
        "all": all_,
    }


class Report(unittest.TestCase):
    def test_means_never_passed_and_the_pooled_row(self):
        rows = [
            row("o/a", rework.CITED, 1, spec=2, all_=4),
            row("o/a", rework.CITED, None, convention=1, all_=3),
            row("o/b", rework.CITED, 3, spec=1, all_=5),
            row("o/b", rework.NO_PLAN, None, reviewed=False),
        ]
        lines = rework.report(rows).splitlines()
        cells = {
            tuple(c.strip() for c in line.strip("|").split("|")[:2]): [
                c.strip() for c in line.strip("|").split("|")[2:]
            ]
            for line in lines
            if line.startswith("|")
        }
        # PRs, reviewed, mean rounds, never passed, spec, convention, all, unreadable
        self.assertEqual(
            cells[("o/a", rework.CITED)],
            ["2", "2", "1.00", "1", "1.00", "0.50", "3.50", "0"],
        )
        self.assertEqual(
            cells[("o/b", rework.NO_PLAN)], ["1", "0", "–", "0", "–", "–", "–", "0"]
        )
        self.assertEqual(
            cells[("**all**", rework.CITED)],
            ["3", "3", "2.00", "1", "1.00", "0.33", "4.00", "0"],
        )


PAGE = """[{"data": {"repository": {"pullRequests": {
  "nodes": [{"number": 3, "title": "t", "headRefOid": "abc",
             "files": {"nodes": [{"path": "a.md"}]},
             "comments": {"nodes": []},
             "reviews": {"nodes": [{"submittedAt": "t", "body": "b"}]}}],
  "pageInfo": {"hasNextPage": false, "endCursor": null}}}}}]"""
PR3 = {
    "number": 3,
    "title": "t",
    "headRefOid": "abc",
    "files": [{"path": "a.md"}],
    "comments": [],
    "reviews": [{"submittedAt": "t", "body": "b"}],
}


class Fetch(unittest.TestCase):
    def setUp(self):
        import tempfile

        self.cache = Path(tempfile.mkdtemp())

    def test_fetches_once_then_reads_the_cache(self):
        calls = []

        def gh(args):
            calls.append(args)
            return PAGE

        self.assertEqual(rework.fetch("o/r", self.cache, gh=gh), [PR3])
        self.assertEqual(rework.fetch("o/r", self.cache, gh=gh), [PR3])
        self.assertEqual(len(calls), 1)
        self.assertIn("--paginate", calls[0])
        self.assertIn("owner=o", calls[0])
        self.assertEqual(rework.fetch("o/r", self.cache, refresh=True, gh=gh), [PR3])
        self.assertEqual(len(calls), 2)

    def test_retries_a_gateway_timeout(self):
        replies = [rework.GhError("HTTP 504: Gateway Timeout"), "[]"]

        def gh(args):
            reply = replies.pop(0)
            if isinstance(reply, Exception):
                raise reply
            return reply

        self.assertEqual(rework.fetch("o/r", self.cache, gh=gh, wait=0), [])

    def test_other_errors_are_not_retried(self):
        calls = []

        def gh(args):
            calls.append(args)
            raise rework.GhError("Could not resolve to a Repository")

        with self.assertRaises(rework.GhError):
            rework.fetch("o/nope", self.cache, gh=gh, wait=0)
        self.assertEqual(len(calls), 1)
        self.assertFalse(any(self.cache.iterdir()))


if __name__ == "__main__":
    unittest.main()
