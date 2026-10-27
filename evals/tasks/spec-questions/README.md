# spec-questions

The spec-question task: the agent answers a question about the identity
specs and cites the section, and the scorer compares its answer with a
closed-form one and its section with the accepted ones. The questions are asked the way an agent asks them, in its
own words rather than the spec's, so a keyword `grep` doesn't find the
section. That is where codecite should beat a grep baseline.

| File | What |
| --- | --- |
| `task.json` | The task: where the prompt, questions and scorer are, the answer file and spec directory the prompt names, the corpus, and the epochs per tier |
| `prompt.md` | The prompt, with `{question}`, `{specs}` and `{answer_file}` filled in per sample |
| `questions.toml` | The questions, their answers, the sections that support them, and where each came from |
| `score.py` | The scorer, run in the sandbox after the agent exits: answer and section both right, or the sample fails |
| `check.sh`, `planted.py` | Prove `score.py` sound: every reference answer file passes, and wrong and malformed ones fail |
| `../../src/codecite_evals/spec_task.py` | The Inspect task: one sample per question per arm |
| `../../src/codecite_evals/spec_questions.py` | Checks the set against the identity-bench index: every accepted section exists, and no question repeats a distinctive term of its section |
| `../../tests/test_spec_questions.py` | Its tests, on made-up questions and the real set's shape |

## Keeping the answers out of the sandbox

`questions.toml` holds the answers, so it is hidden data, like a task's
`hidden/` tests. Nothing that builds the sandbox image may copy it: the agent
gets only a question's `text`, and the scorer reads `answer` and `accepted`
from this file after the agent exits. `prepare.sh` stages no file of this
directory (`task.json` names no project), and a sample's Inspect metadata
holds only its arm and question id.

## The answer file

The prompt asks for exactly two lines in `/work/answer.txt`:

```
answer: <the answer>
section: <document>#section-<number>
```

`score.py` accepts any of the question's `answer` forms, ignoring case,
surrounding quotes or backticks and a final full stop. The section must be
one of `accepted`; a step of an accepted section (`/step-<k>`) counts, and
codecite's `@sha256:<hex>` pin may stay in. A subsection of an accepted
section doesn't count, and neither does a missing file, a missing or extra
line, or prose. `check.sh` runs these cases on every question:

```
evals/tasks/spec-questions/check.sh
```

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
