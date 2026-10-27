# spec-questions

Labelled questions for the spec-question task (#104): the agent answers a
question about the identity specs, and the scorer compares its answer with a
closed-form one. The questions are asked the way an agent asks them, in its
own words rather than the spec's, so a keyword `grep` doesn't find the
section. That is where codecite should beat a grep baseline.

| File | What |
| --- | --- |
| `questions.toml` | The questions, their answers, the sections that support them, and where each came from |
| `../../src/codecite_evals/spec_questions.py` | Checks the set against the identity-bench index: every accepted section exists, and no question repeats a distinctive term of its section |
| `../../tests/test_spec_questions.py` | Its tests, on made-up questions and the real set's shape |

## Keeping the answers out of the sandbox

`questions.toml` holds the answers, so it is hidden data, like a task's
`hidden/` tests. Nothing that builds the sandbox image may copy it: the agent
gets only a question's `text`, and the scorer reads `answer` and `accepted`
from this file after the agent exits.

## Checking the set

The checks read the identity-bench corpus file, so section numbers are the
ones search cites. They need no model key; CI's `bench` job runs the checker
and `evals-test` runs its tests.

```
cargo build --release -p codecite
target/release/codecite index identity-bench --manifest evals/retrieval/identity-bench.toml
cd evals
uv run --locked python -m codecite_evals.spec_questions
uv run --locked pytest tests/test_spec_questions.py
```

A word of a question is a distinctive term of its section when the section
also uses it and it either names something (it has a `_` or mixes letters and
digits) or fewer than 3% of the corpus's spec sections use it. The checker
skips plain English and a short allowlist of the field's everyday words.
