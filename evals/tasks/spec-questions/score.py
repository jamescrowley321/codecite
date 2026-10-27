"""Score one spec-question answer file. Runs in the sandbox after the agent
exits, as `python3 -I score.py <expected.json> <answer file>`, and prints one
JSON object: {"passed", "reason", "answer", "section"}.

expected.json is the question's `answer` and `accepted` from questions.toml,
which reach the sandbox only now. The answer file holds exactly two lines:

    answer: <the answer>
    section: <document>#section-<number>

A sample passes when the answer is one of the accepted forms and the section
is an accepted one. Comparison ignores case, surrounding whitespace, quotes
and backticks, and a trailing full stop. The section may carry codecite's
`@sha256:<hex>` pin and a `/step-<k>` suffix: a step in an accepted section
counts, and an accepted step must be cited as that step. Stdlib only.
"""

import json
import re
import sys
from pathlib import Path

KEYS = ("answer", "section")
SECTION = re.compile(
    r"^(?P<doc>[a-z0-9_\-]+)(?:@sha256:[0-9a-f]+)?#section-(?P<number>[0-9a-z.]+?)(?:/step-(?P<step>[0-9]+))?$"
)


def clean(value):
    """Without surrounding whitespace, quotes, backticks or a final full stop,
    in any nesting (`` `yes`. ``, ``"yes."``)."""
    while True:
        stripped = value.strip().strip("`\"'").removesuffix(".")
        if stripped == value:
            return value
        value = stripped


def parse_section(value):
    m = SECTION.match(clean(value).lower())
    return (m["doc"], m["number"], m["step"]) if m else None


def read(path):
    """The answer file's two fields, or why it can't be read."""
    if not path.is_file():
        return None, "no answer file"
    try:
        lines = [line for line in path.read_text().splitlines() if line.strip()]
    except (OSError, UnicodeDecodeError):
        return None, "answer file is unreadable"
    fields = {}
    for line in lines:
        key, sep, value = line.partition(":")
        key = key.strip().lower()
        if not sep or key not in KEYS or key in fields:
            return None, f"malformed: want exactly the lines {' and '.join(f'`{k}: ...`' for k in KEYS)}"
        fields[key] = value
    if set(fields) != set(KEYS):
        return None, f"malformed: want exactly the lines {' and '.join(f'`{k}: ...`' for k in KEYS)}"
    return fields, None


def score(expected, path):
    fields, problem = read(path)
    if problem:
        return {"passed": False, "reason": problem, "answer": None, "section": None}
    answer, section = clean(fields["answer"]), clean(fields["section"])
    cited = parse_section(section)
    if cited is None:
        return {"passed": False, "reason": "malformed: section is not `<document>#section-<number>`", "answer": answer, "section": section}

    reasons = []
    if answer.casefold() not in {a.casefold() for a in expected["answer"]}:
        reasons.append("wrong answer")
    supports = False
    for label in expected["accepted"]:
        doc, number, step = parse_section(label)
        if (cited[0], cited[1]) == (doc, number) and (step is None or cited[2] == step):
            supports = True
    if not supports:
        reasons.append("section is not an accepted one")
    return {"passed": not reasons, "reason": "; ".join(reasons) or "correct", "answer": answer, "section": section}


def main():
    expected = json.loads(Path(sys.argv[1]).read_text())
    print(json.dumps(score(expected, Path(sys.argv[2]))))


if __name__ == "__main__":
    main()
