"""The spec-question checks, on made-up questions and the real set's shape.

The index checks run on made-up sections here; CI's bench job runs them on
the real identity-bench index.
"""

import pytest

from codecite_evals import config
from codecite_evals import spec_questions as sq

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

QUERIES = [
    {"id": "spec-plain-old", "class": "spec-plain", "text": "old", "answers": ["rfc6749#section-2.3.1"]},
    {"id": "spec-at-hash", "class": "spec-keyword", "text": "at_hash", "answers": ["rfc6749#section-2.3.1"]},
]
DOCUMENTS = {f"doc{n}" for n in range(8)}


def question(text, accepted, **extra):
    return {"id": "q", "text": text, "answer": ["yes"], "accepted": accepted, "source": "a task", **extra}


def spread(n):
    """n well-formed new questions spread over the eight documents."""
    return [{**question(f"Q{i}?", [f"doc{i % 8}#section-1"]), "id": f"q{i}"} for i in range(n)]


def test_the_real_set_is_well_formed():
    questions = sq.load(config.task_dir(sq.TASK) / "questions.toml")["question"]
    queries = sq.load(config.QUERIES)["query"]
    documents = {s["name"] for s in sq.load(config.BENCH_MANIFEST)["source"] if s["kind"] == "document"}
    assert sq.check_set(questions, queries, documents) == []


# Against the index.


def test_a_question_in_plain_words_passes():
    q = question("Can an app with no secret skip proving who it is at the endpoint?", ["rfc6749#section-3.2.1"])
    assert sq.check_against_index([q], FOUND) == []


def test_a_name_from_the_section_fails():
    q = question("What does RFC 6749 say about client_secret_basic?", ["rfc6749#section-2.3.1"])
    assert sq.check_against_index([q], FOUND) == ["q: repeats client_secret_basic from `rfc6749#section-2.3.1`"]


def test_a_rare_word_from_the_section_fails():
    q = question("May a client send its password in Basic?", ["rfc6749#section-2.3.1"])
    assert sq.check_against_index([q], FOUND) == ["q: repeats basic, password from `rfc6749#section-2.3.1`"]


def test_allowed_and_common_words_pass():
    q = question("Must the client support the endpoint when making requests?", ["rfc6749#section-3.2.1"])
    assert sq.check_against_index([q], FOUND) == []


def test_a_missing_section_or_step_fails():
    qs = [question("Is it?", ["rfc6749#section-9.9"]), question("Is it?", ["rfc6749#section-3.2.1/step-3"])]
    assert sq.check_against_index(qs, FOUND) == [
        "q: accepted `rfc6749#section-9.9` is not a section of the indexed document",
        "q: accepted `rfc6749#section-3.2.1/step-3` is not a step of that section",
    ]


def test_a_malformed_question_is_left_to_check_set():
    qs = [{"id": "q", "accepted": ["rfc6749#section-9.9"]}, question("Is it?", "rfc6749#section-9.9")]
    assert sq.check_against_index(qs, FOUND) == []


def test_a_step_of_an_accepted_section_counts():
    q = question("Is it?", ["rfc6749#section-3.2.1/step-2"])
    assert sq.check_against_index([q], FOUND) == []


# The set's shape.


def test_a_well_formed_set_passes():
    assert sq.check_set(spread(24), QUERIES, DOCUMENTS) == []


def test_too_few_questions_fail():
    assert "23 questions; want at least 24" in sq.check_set(spread(23), QUERIES, DOCUMENTS)


def test_free_text_answers_fail():
    qs = spread(24)
    qs[0]["answer"] = ["it must reject it"]
    assert sq.check_set(qs, QUERIES, DOCUMENTS) == ["q0: answer `it must reject it` is not closed-form"]


def test_a_missing_field_fails_without_a_crash():
    qs = spread(24)
    del qs[0]["text"]
    assert "q0: missing text" in sq.check_set(qs, QUERIES, DOCUMENTS)


@pytest.mark.parametrize("accepted", ["doc0#section-1", [1], [], 7])
def test_accepted_that_is_not_a_list_of_sections_fails(accepted):
    qs = spread(24)
    qs[0]["accepted"] = accepted
    assert "q0: accepted must be a non-empty list of sections" in sq.check_set(qs, QUERIES, DOCUMENTS)


@pytest.mark.parametrize("field, value", [("id", ["q0"]), ("adapted_from", ["spec-plain-old"]), ("text", 1)])
def test_a_field_of_the_wrong_type_fails_without_a_crash(field, value):
    qs = spread(24)
    qs[0][field] = value
    assert any("must be a string" in f for f in sq.check_set(qs, QUERIES, DOCUMENTS))


def test_more_than_half_adapted_fails():
    qs = spread(24)
    for q in qs[:13]:
        q["adapted_from"] = "spec-plain-old"
    assert sq.check_set(qs, QUERIES, DOCUMENTS) == ["11 of 24 questions are new; want at least half"]


def test_adapting_a_keyword_query_fails():
    qs = spread(24)
    qs[0]["adapted_from"] = "spec-at-hash"
    assert sq.check_set(qs, QUERIES, DOCUMENTS) == [
        "q0: adapted_from `spec-at-hash` is not a spec-plain or spec-requirement query"
    ]


def test_one_document_with_over_a_quarter_fails():
    qs = spread(24)
    for q in qs[1:7]:  # with q0, q8 and q16, nine of 24
        q["accepted"] = ["doc0#section-1"]
    assert "doc0: 9 of 24 questions; want at most a quarter" in sq.check_set(qs, QUERIES, DOCUMENTS)


def test_a_new_question_must_match_its_retrieval_query():
    qs = spread(24)
    qs[0]["id"] = "spec-plain-old"
    assert sq.check_set(qs, QUERIES, DOCUMENTS) == [
        "spec-plain-old: differs from the query of the same id in queries.toml"
    ]
