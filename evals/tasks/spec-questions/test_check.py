"""Tests for check.py on made-up questions, so they need no index.

Usage: python3 -I -m unittest discover -s evals/tasks/spec-questions
"""

import unittest

import check

SECTION = {
    ("rfc6749", "2.3.1"): {
        "text": "2.3.1. Client Password\n\nClients in possession of a client password MAY use the HTTP Basic "
        "authentication scheme. The authorization server MUST support client_secret_basic.",
        "steps": set(),
    },
    ("rfc6749", "3.2.1"): {
        "text": "3.2.1. Client Authentication\n\nConfidential clients MUST authenticate with the "
        "authorization server when making requests to the token endpoint.",
        "steps": {"1", "2"},
    },
}
# Filler sections, so that words in only the two above are rare and words in
# every section are common.
FOUND = dict(SECTION)
for n in range(100):
    FOUND[("rfc9999", str(n))] = {"text": "the server MUST support the endpoint when making requests", "steps": set()}


def question(text, accepted, **extra):
    return {"id": "q", "text": text, "answer": ["yes"], "accepted": accepted, "source": "a task", **extra}


class AgainstIndex(unittest.TestCase):
    def test_a_question_in_plain_words_passes(self):
        q = question("Can an app with no secret skip proving who it is at the endpoint?", ["rfc6749#section-3.2.1"])
        self.assertEqual(check.check_against_index([q], FOUND), [])

    def test_a_name_from_the_section_fails(self):
        q = question("What does RFC 6749 say about client_secret_basic?", ["rfc6749#section-2.3.1"])
        self.assertEqual(check.check_against_index([q], FOUND), ["q: repeats client_secret_basic from `rfc6749#section-2.3.1`"])

    def test_a_rare_word_from_the_section_fails(self):
        q = question("May a client send its password in Basic?", ["rfc6749#section-2.3.1"])
        self.assertEqual(check.check_against_index([q], FOUND), ["q: repeats basic, password from `rfc6749#section-2.3.1`"])

    def test_allowed_and_common_words_pass(self):
        q = question("Must the client support the endpoint when making requests?", ["rfc6749#section-3.2.1"])
        self.assertEqual(check.check_against_index([q], FOUND), [])

    def test_a_missing_section_or_step_fails(self):
        qs = [question("Is it?", ["rfc6749#section-9.9"]), question("Is it?", ["rfc6749#section-3.2.1/step-3"])]
        self.assertEqual(
            check.check_against_index(qs, FOUND),
            [
                "q: accepted `rfc6749#section-9.9` is not a section of the indexed document",
                "q: accepted `rfc6749#section-3.2.1/step-3` is not a step of that section",
            ],
        )

    def test_a_step_of_an_accepted_section_counts(self):
        q = question("Is it?", ["rfc6749#section-3.2.1/step-2"])
        self.assertEqual(check.check_against_index([q], FOUND), [])


QUERIES = [
    {"id": "spec-plain-old", "class": "spec-plain", "text": "old", "answers": ["rfc6749#section-2.3.1"]},
    {"id": "spec-at-hash", "class": "spec-keyword", "text": "at_hash", "answers": ["rfc6749#section-2.3.1"]},
]
DOCUMENTS = {f"doc{n}" for n in range(8)}


def spread(n, **extra):
    """n well-formed new questions spread over the eight documents."""
    return [
        {**question(f"Q{i}?", [f"doc{i % 8}#section-1"]), "id": f"q{i}", **extra} for i in range(n)
    ]


class Set(unittest.TestCase):
    def test_a_well_formed_set_passes(self):
        self.assertEqual(check.check_set(spread(24), QUERIES, DOCUMENTS), [])

    def test_too_few_questions_fail(self):
        self.assertIn("23 questions; want at least 24", check.check_set(spread(23), QUERIES, DOCUMENTS))

    def test_free_text_answers_fail(self):
        qs = spread(24)
        qs[0]["answer"] = ["it must reject it"]
        self.assertEqual(check.check_set(qs, QUERIES, DOCUMENTS), ["q0: answer `it must reject it` is not closed-form"])

    def test_more_than_half_adapted_fails(self):
        qs = spread(24)
        for q in qs[:13]:
            q["adapted_from"] = "spec-plain-old"
        self.assertEqual(check.check_set(qs, QUERIES, DOCUMENTS), ["11 of 24 questions are new; want at least half"])

    def test_adapting_a_keyword_query_fails(self):
        qs = spread(24)
        qs[0]["adapted_from"] = "spec-at-hash"
        self.assertEqual(
            check.check_set(qs, QUERIES, DOCUMENTS),
            ["q0: adapted_from `spec-at-hash` is not a spec-plain or spec-requirement query"],
        )

    def test_one_document_with_over_a_quarter_fails(self):
        qs = spread(24)
        for q in qs[1:7]:  # with q0, q8 and q16, nine of 24
            q["accepted"] = ["doc0#section-1"]
        self.assertIn("doc0: 9 of 24 questions; want at most a quarter", check.check_set(qs, QUERIES, DOCUMENTS))

    def test_a_new_question_must_match_its_retrieval_query(self):
        qs = spread(24)
        qs[0]["id"] = "spec-plain-old"
        self.assertEqual(
            check.check_set(qs, QUERIES, DOCUMENTS),
            ["spec-plain-old: differs from the query of the same id in queries.toml"],
        )


if __name__ == "__main__":
    unittest.main()
