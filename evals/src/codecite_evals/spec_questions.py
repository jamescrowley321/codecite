"""Check the spec-question set in questions.toml.

Usage: uv run --locked python -m codecite_evals.spec_questions [--db PATH]

Reads the identity-bench index that `codecite index identity-bench --manifest
evals/retrieval/identity-bench.toml` builds, so every section is numbered the
way search cites it. Needs no model key. Prints one line per failure and exits
non-zero if there is any.

It checks that:
- the set is well-formed: enough questions, closed-form answers, at least half
  new, the adapted ones naming a spec-plain or spec-requirement query, and the
  questions spread over the documents;
- a new question that is also a retrieval query matches it in queries.toml;
- every accepted section (or step) exists in the pinned document;
- no question repeats a distinctive term of an accepted section verbatim.
"""

import argparse
import os
import re
import sqlite3
import sys
import tomllib
from collections import Counter
from pathlib import Path

from codecite_evals import config

TASK = "spec-questions"
CORPUS = "identity-bench"

MIN_QUESTIONS = 24
MIN_DOCUMENTS = 6
ADAPTED_CLASSES = {"spec-plain", "spec-requirement"}
FIELDS = {"id", "text", "answer", "accepted", "source"}
OPTIONAL_FIELDS = {"adapted_from"}
STRING_FIELDS = {"id", "text", "source", "adapted_from"}

# `yes`/`no`, or one name or value: a parameter, claim, error, header, scheme,
# path or number. No spaces, so no free text.
CLOSED_ANSWER = re.compile(r"^[A-Za-z0-9_.:/+=\-]+$")
ACCEPTED = re.compile(r"^(?P<doc>[a-z0-9_\-]+)#section-(?P<number>[0-9A-Z.]+?)(?:/step-(?P<step>[0-9]+))?$")

# A word of the question that also appears in its section is distinctive when
# it names something (it has a `_` or mixes letters and digits) or when fewer
# than this share of the corpus's spec sections use it. The field's everyday
# words are in more; plain English is in fewer, since specs rarely say "my".
RARE_SHARE = 0.03
# Words an agent can't ask about the field without, however rare.
ALLOWED = {"token", "tokens", "client", "clients", "app", "user", "server", "header", "error", "code"}
# Plain English: function words, and the units a question asks a number in.
PLAIN = set(
    """
    a about after again all am an and any are as at be because been before being between both but by can
    cannot could did do does doesnt dont each else ever few for from had hasnt has have havent having he her
    here hers him his how i if in into is isnt it its itself just like many may me might more most must my
    no nor not now of off on once one only or other our out over own same say says she should so some still
    such than that the their them then there these they theyve this those through to too under until up
    us very was we were what when where whether which while who whom why will with without would yet you
    your youre
    seconds minutes hours days characters bytes
    """.split()
)
WORD = re.compile(r"[a-z0-9_]+")


def words(text):
    return set(WORD.findall(text.lower().replace("'", "").replace("\u2019", "")))


def names_something(word):
    return "_" in word or (re.search(r"[a-z]", word) and re.search(r"[0-9]", word))


def default_db():
    data = os.environ.get("XDG_DATA_HOME") or Path.home() / ".local/share"
    return Path(data) / "codecite" / f"{CORPUS}.db"


def malformed(q):
    """Why a question can't be checked at all, or None: a missing field, or a
    field of the wrong type."""
    missing = FIELDS - q.keys()
    if missing:
        return f"missing {', '.join(sorted(missing))}"
    wrong = sorted(f for f in STRING_FIELDS if f in q and not isinstance(q[f], str))
    if wrong:
        return f"{', '.join(wrong)} must be a string"
    accepted = q["accepted"]
    if not (isinstance(accepted, list) and accepted and all(isinstance(a, str) for a in accepted)):
        return "accepted must be a non-empty list of sections"
    return None


def load(path):
    with open(path, "rb") as f:
        return tomllib.load(f)


def check_set(questions, queries, documents):
    """The checks that need only the files."""
    failures = []
    if len(questions) < MIN_QUESTIONS:
        failures.append(f"{len(questions)} questions; want at least {MIN_QUESTIONS}")

    ids = Counter(q["id"] for q in questions if isinstance(q.get("id"), str))
    failures += [f"{i}: id used {n} times" for i, n in ids.items() if n > 1]

    by_id = {q["id"]: q for q in queries}
    new = 0
    per_document = Counter()
    for q in questions:
        qid = q["id"] if isinstance(q.get("id"), str) else "?"
        problem = malformed(q)
        if problem:
            failures.append(f"{qid}: {problem}")
            continue
        extra = q.keys() - FIELDS - OPTIONAL_FIELDS
        if extra:
            failures.append(f"{qid}: unknown field {', '.join(sorted(extra))}")

        answer = q["answer"]
        if not (isinstance(answer, list) and answer and all(isinstance(a, str) for a in answer)):
            failures.append(f"{qid}: answer must be a non-empty list of forms")
        else:
            failures += [f"{qid}: answer `{a}` is not closed-form" for a in answer if not CLOSED_ANSWER.match(a)]
            if any(a.lower() in ("yes", "no") for a in answer) and len(answer) != 1:
                failures.append(f"{qid}: a yes/no answer has one form")

        docs = set()
        for label in q["accepted"]:
            m = ACCEPTED.match(label)
            if not m:
                failures.append(f"{qid}: accepted `{label}` is not `<document>#section-<n>[/step-<k>]`")
            elif m["doc"] not in documents:
                failures.append(f"{qid}: accepted `{label}` names a document not in identity-bench.toml")
            else:
                docs.add(m["doc"])
        per_document.update(docs)

        origin = q.get("adapted_from")
        if origin is None:
            new += 1
            # A new question that is also a retrieval query must be the same query.
            query = by_id.get(qid)
            if query and (query["text"] != q["text"] or sorted(query["answers"]) != sorted(q["accepted"])):
                failures.append(f"{qid}: differs from the query of the same id in queries.toml")
        elif origin not in by_id:
            failures.append(f"{qid}: adapted_from `{origin}` is not in queries.toml")
        elif by_id[origin].get("class") not in ADAPTED_CLASSES:
            failures.append(f"{qid}: adapted_from `{origin}` is not a spec-plain or spec-requirement query")

    if 2 * new < len(questions):
        failures.append(f"{new} of {len(questions)} questions are new; want at least half")
    if len(per_document) < MIN_DOCUMENTS:
        failures.append(f"questions cite {len(per_document)} documents; want at least {MIN_DOCUMENTS}")
    for doc, n in per_document.items():
        if 4 * n > len(questions):
            failures.append(f"{doc}: {n} of {len(questions)} questions; want at most a quarter")
    return failures


def sections(db):
    """Every current spec section's text and its steps, by document and number."""
    con = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
    rows = con.execute(
        """
        SELECT s.name, sec.number, sec.text, step.number
        FROM chunk sec
        JOIN document d ON d.id = sec.document_id
        JOIN source s ON s.id = d.source_id
        LEFT JOIN chunk step ON step.parent_id = sec.id AND step.kind = 'step'
        WHERE s.kind = 'document' AND s.is_current = 1 AND sec.kind = 'section'
        """
    )
    found = {}
    for doc, number, text, step in rows:
        entry = found.setdefault((doc, number), {"text": text, "steps": set()})
        if step is not None:
            entry["steps"].add(str(step))
    con.close()
    return found


def check_against_index(questions, found):
    """The checks that need the index: sections exist, no distinctive term repeated."""
    failures = []
    document_frequency = Counter()
    for entry in found.values():
        document_frequency.update(words(entry["text"]))
    rare_below = RARE_SHARE * len(found)

    for q in questions:
        if malformed(q):
            continue  # check_set reports it
        asked = words(q["text"]) - ALLOWED - PLAIN
        for label in q["accepted"]:
            m = ACCEPTED.match(label)
            if not m:
                continue
            entry = found.get((m["doc"], m["number"]))
            if entry is None:
                failures.append(f"{q['id']}: accepted `{label}` is not a section of the indexed document")
                continue
            if m["step"] and m["step"] not in entry["steps"]:
                failures.append(f"{q['id']}: accepted `{label}` is not a step of that section")
            repeated = sorted(
                w for w in asked & words(entry["text"]) if names_something(w) or document_frequency[w] < rare_below
            )
            if repeated:
                failures.append(f"{q['id']}: repeats {', '.join(repeated)} from `{label}`")
    return failures


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--db", type=Path, default=default_db(), help="the identity-bench corpus file")
    args = parser.parse_args()

    questions = load(config.task_dir(TASK) / "questions.toml")["question"]
    queries = load(config.QUERIES)["query"]
    documents = {s["name"] for s in load(config.BENCH_MANIFEST)["source"] if s["kind"] == "document"}

    failures = check_set(questions, queries, documents)
    if not args.db.exists():
        failures.append(
            f"no index at {args.db}; build it with "
            f"`codecite index {CORPUS} --manifest evals/retrieval/identity-bench.toml`"
        )
    else:
        failures += check_against_index(questions, sections(args.db))

    for failure in failures:
        print(failure)
    if failures:
        sys.exit(1)
    print(f"{len(questions)} questions: all checks pass")


if __name__ == "__main__":
    main()
