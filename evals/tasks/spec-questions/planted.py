"""Prove score.py sound on every question in questions.toml, the way the
sandbox runs it (`python3 -I score.py <expected.json> <answer file>`): each
reference answer file passes, and a wrong answer, a wrong section, a missing
file and malformed output each fail. Stdlib only; no model key or Docker.

    python3 evals/tasks/spec-questions/planted.py
"""

import json
import subprocess
import sys
import tempfile
import tomllib
from pathlib import Path

HERE = Path(__file__).resolve().parent


def run(expected, text, tmp):
    """score.py's verdict on an answer file holding text (None: no file)."""
    spec, answer = tmp / "expected.json", tmp / "answer.txt"
    spec.write_text(json.dumps(expected))
    answer.unlink(missing_ok=True)
    if text is not None:
        answer.write_text(text)
    out = subprocess.run([sys.executable, "-I", str(HERE / "score.py"), str(spec), str(answer)], capture_output=True, text=True, check=True)
    return json.loads(out.stdout)


def wrong_answer(answer):
    return {"yes": "no", "no": "yes"}.get(answer[0].lower(), "not-the-answer")


def cases(q, others):
    """(name, answer file text or None, should pass) for one question."""
    answer, accepted = q["answer"], q["accepted"]
    doc, _, number = accepted[0].partition("#section-")
    # A section another question accepts and this one doesn't.
    elsewhere = next(a for o in others for a in o["accepted"] if a not in accepted)
    yield "reference", f"answer: {answer[0]}\nsection: {accepted[0]}\n", True
    for form in answer[1:]:
        yield f"answer form {form}", f"answer: {form}\nsection: {accepted[0]}\n", True
    for label in accepted[1:]:
        yield f"accepted {label}", f"answer: {answer[0]}\nsection: {label}\n", True
    yield "codecite citation", f"answer: {answer[0].upper()}\nsection: {doc}@sha256:0123456789ab#section-{number}/step-2\n", True
    yield "quoted, padded", f"\n  Answer:  `{answer[0]}`.\n\nSection: `{accepted[0]}`\n", True
    yield "wrong answer", f"answer: {wrong_answer(answer)}\nsection: {accepted[0]}\n", False
    yield "wrong section in the document", f"answer: {answer[0]}\nsection: {doc}#section-99.9\n", False
    yield "another question's section", f"answer: {answer[0]}\nsection: {elsewhere}\n", False
    yield "a subsection of the section", f"answer: {answer[0]}\nsection: {accepted[0]}.99\n", False
    yield "missing file", None, False
    yield "empty file", "", False
    yield "answer only", f"answer: {answer[0]}\n", False
    yield "an extra line", f"answer: {answer[0]}\nsection: {accepted[0]}\nwhy: it says so\n", False
    yield "prose", f"The answer is {answer[0]}, per {accepted[0]}.\n", False
    yield "section in prose", f"answer: {answer[0]}\nsection: {doc.upper()} §{number}\n", False
    yield "answer with a reason", f"answer: {answer[0]} because the spec says so\nsection: {accepted[0]}\n", False


def main():
    questions = tomllib.loads((HERE / "questions.toml").read_text())["question"]
    failures, total = [], 0
    with tempfile.TemporaryDirectory() as d:
        tmp = Path(d)
        for q in questions:
            expected = {"answer": q["answer"], "accepted": q["accepted"]}
            for name, text, should_pass in cases(q, [o for o in questions if o is not q]):
                total += 1
                verdict = run(expected, text, tmp)
                if verdict["passed"] != should_pass:
                    failures.append(f"{q['id']}: {name}: want {'pass' if should_pass else 'fail'}, got {verdict}")
    for failure in failures:
        print(failure)
    if failures:
        sys.exit(f"{len(failures)} of {total} cases scored wrong")
    print(f"{len(questions)} questions, {total} cases: every reference passes and every planted answer fails")


if __name__ == "__main__":
    main()
