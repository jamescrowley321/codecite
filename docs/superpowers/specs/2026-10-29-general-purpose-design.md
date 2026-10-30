# General purpose

## Why

codecite is meant to be general purpose: a local index over any set of
repositories and documents that answers agents with checkable citations. It
grew around one corpus instead. identity-model and the OAuth and OpenID specs
were used because they were available, and every failure was fixed in a way
shaped to them.

An inventory of `main` at `edd6c1a` shows where that left it:

- **The core is general.** About 60% of the Rust works on any git repository:
  code chunking for six languages, Markdown split by headings, git and
  document sources pinned by commit or hash, the store, `search`, `grep`,
  `def`, `read`, the CLI and the MCP server.
- **About a fifth is spec-specific.** That covers the RFC and OpenID HTML
  parsers, numbered steps, uppercase BCP 14 keywords as "rules",
  `requirements`, spec version `diff`, RFC relations and spec citation forms,
  about 6,150 lines. Guidance (about 2,440 lines) is a general idea with a
  spec-shaped rule model.
- **Search assumes specs.** It reserves three result slots for spec sections
  and builds a separate query for them.
- **Every measurement is identity.** All four agent tasks run on
  identity-model, three of them on spec rules. All 122 benchmark queries run on
  one corpus, and 63% of them are spec queries.
- **Agents are told about specs.** About half of the MCP server's instructions
  are spec advice.

Nothing has measured codecite on any other repository, so there is no
evidence either way on whether the general core works.

## What codecite is for

- Agents use the **CLI** first (`codecite search`, `grep`, `def`, `read`,
  `cite`). The MCP server is a bonus over the same commands.
- They use it while planning and executing, alongside CodeGraph, to pull
  verifiable citations instead of working from training data or burning tokens
  on wide searches.
- It covers any repository and any prose kept with it: READMEs, design docs,
  ADRs, PRDs and specs, handled the same way.
- **Success means less rework.** Correctness comes first. Outputs must be no
  worse, and codecite must not be significantly slower than not using it.

## Goals

1. Measure codecite on many unrelated repositories, with labels that need no
   hand-writing and can't be tuned toward one corpus.
2. Make the CLI an agent's natural tool: compact output, help written for an
   agent learning it cold, and a short instructions snippet for AGENTS.md or
   CLAUDE.md.
3. Handle every document the same way: split on its own headings and cited by
   path, anchor and lines.
4. One ranking over code and prose, tuned only against the multi-repo
   benchmark.
5. Adding many repositories is a few commands.
6. Agent evals replayed from real past work.

## Non-goals

- New features for one document family or one naming convention.
- Removing the spec layer before the benchmark says what it costs or buys.
  It becomes an optional adapter (goal 3), judged like everything else.
- A search server (Milvus, Algolia). SQLite stays. tantivy remains the
  fallback in `docs/design.md` if size forces it.

## Anti-goals

The owner's words: outputs worse, significantly slower, or less correct.

## How it is measured

**The history benchmark (goal 1)** is the measure every later goal is judged
by. It is free to run, so it runs on every PR.

- **Ground truth from merged PRs.** For each of the owner's repositories with
  enough history, a sample of merged PRs. The question is the PR's title
  (class `history-title`). When the PR closes an issue, the issue's title and
  body are a second question (class `history-issue`): the task as it was asked,
  before anyone wrote the code. The answer is the set of files the PR modified
  that already existed at the corpus's pinned commit, excluding lockfiles and
  generated files.
- **One pinned commit per repository.** That is the base of the oldest sampled
  PR, so every answer file exists in the index.
- **Scored at file level.** A result counts when it is in an answer file:
  Recall@1, Recall@10 and MRR per repository and class.
- **Against grep.** The same questions are run through a plain grep ranking
  over the same commit: files ranked by how many of the question's words they
  match, using `git grep`. codecite has to beat it to be worth calling.
- **Frozen.** Queries are mined once by a script and committed with their
  repository's commit, and are never edited by hand. Remining is a new
  benchmark version.
- **Repositories:** identity-model (Python), identity-stack (Python,
  TypeScript, Terraform), blind-peer-review (JavaScript),
  terraform-provider-descope (Go) and codecite (Rust). Two of the owner's
  other repositories have too few PRs and are left out. Another project may be
  run locally, with nothing of it committed.

**The gate for agent evals**, once goal 6 exists: any correctness loss blocks;
a median wall-time rise of more than 25% blocks; tokens are reported only. The
panel is claude-haiku-5.5, claude-sonnet-5.5 and claude-opus-5.5, Claude-only.

## Order

1. The history benchmark, and a first scorecard of where codecite stands
   against grep on each repository.
2. CLI ergonomics for agents.
3. Generic documents. The spec parsers, `requirements` and `diff` become an
   optional adapter.
4. One ranking over code and prose.
5. `codecite add` and corpora across many repositories.
6. Replay evals: the prompt is a past issue, and the hidden tests are the
   tests from the PR that fixed it.

Goals 2–4 are each judged on the history benchmark. A change that loses on any
repository doesn't merge.

## Left to the builder

- Sample size per repository, and the rule for leaving out a PR (no modified
  files that existed at the pinned commit, or only docs or config).
- The grep ranking's exact scoring, as long as it uses only what an agent's
  grep would: the question's words.
- How the benchmark runs in CI across five repositories.
