# Spec Questions for the Spec-Question Task Implementation Plan

**Goal:** Implement #130: label at least 24 spec questions, asked in an agent's words, each with a closed-form answer and the sections that support it, for #104's spec-question task. Check them against the index in CI without a model key.

**Architecture:** Data plus a stdlib Python checker, no Rust changes:
- `evals/tasks/spec-questions/questions.toml`: one `[[question]]` per question with `id`, `text`, `answer`, `accepted`, `source`, and `adapted_from` for one adapted from `queries.toml`.
- `evals/src/codecite_evals/spec_questions.py`, in the `codecite-evals` package with its paths from `config.py`: checks the set's shape (count, closed answers, at least half new, spread over documents) and, against the identity-bench corpus file, that every accepted section exists and no question repeats a distinctive term of its section.
- `evals/tests/test_spec_questions.py`: pytest tests for the checker on made-up questions, and the real set's shape.
- `evals/retrieval/queries.toml`: every new question, as a `spec-plain` query under the same id.
- `.github/workflows/ci.yml`: the `bench` job, which already builds the index, runs the checker after the benchmark; `evals-test` runs the tests with the rest of `evals/tests/`.

## Decisions

- **Answers.** `answer` is a list of forms, any one of them right: `["yes"]`, `["invalid_grant"]`, `["none", "prompt=none"]`. A form has no spaces, so no free text. A number's unit is in the question ("how many seconds").
- **Section numbering.** The checker reads the corpus file `codecite index identity-bench` writes: current `document` sources, `section` chunks by `number`, and their `step` children. That is the numbering search cites, so no second parser can drift from it.
- **Distinctive terms.** A word of the question is distinctive in a section when the section also uses it and the word either names something (has a `_`, or mixes letters and digits) or appears in fewer than 3% of the corpus's spec sections. Plain English (function words and units) and a short allowlist (`token`, `client`, `app`, `user`, `server`, `header`, `error`, `code`) never count. Matching is verbatim: lower-cased whole words, apostrophes dropped.
- **Spread.** A question counts once for each document its accepted sections name; no document may have more than a quarter of the questions.
- **Hidden answers.** `questions.toml` is hidden data. The sandbox image is built from a task's `project/` only, so nothing copies it in; #104's solver passes only `text` to the agent and its scorer reads the rest after the agent exits.
- **Sources.** Each `source` names the identity-model task the question came up in, in the style of `queries.toml`. Adapted questions keep their query's source.

## Tasks

1. Write the checker and its tests, test-first on made-up questions: a plain question passes, `client_secret_basic` and a rare word fail, a missing section or step fails, and each shape rule fails on its own.
2. Write the questions, checking each answer against the section text in the index, and rephrase every question the checker flags.
3. Add the new questions to `queries.toml` and run `codecite-bench --class spec` before and after.
4. Add the checker to the `bench` job; describe the directory in `evals/README.md` and its own `README.md`.
5. Run `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --locked`, `uv run --locked pytest` in `evals/`, and the checker.
