# Generic Retrieval Benchmark Implementation Plan

> **For agentic workers:** Use superpowers:subagent-driven-development or superpowers:executing-plans. The benchmark itself invokes local CLIs directly, never a model or agent.

**Goal:** Distinguish extraction, candidate retrieval, rank and latency failures that repeat across repositories before changing Codecite.

**Architecture:** Keep the frozen identity/history and agent evals unchanged. Add an independent source-labeled direct-CLI experiment with repository-disjoint development and evaluation splits, pinned source and tool versions, and a Codecite-only diagnostic for untruncated candidate membership. Treat previously measured Linux and Keycloak questions as development/regression data.

**Tech stack:** Python 3.15 standard library, Git, locked Rust release binaries, SQLite FTS5, CodeGraph CLI.

**Spec:** `docs/design.md` (Ranking and Evaluation plan). Reuse #255 for scale methodology; do not repeat its storage experiments.

## Global constraints

- No project-, package-, path-, or language-name rules in ranking. Language-directed parsing is not a ranking rule.
- No model, agent, MCP, or network calls in the measurement loop. Preserve command argv, stdout/stderr, exit, bytes, and elapsed time locally; publish summaries and citations, not source dumps.
- Separate exact definition lookup, ranked symbol queries, free-text discovery, and edge queries. Report unsupported operations rather than scoring them as misses. Do not claim CodeGraph `query` measures full-text discovery.
- Use private copies of indexes. Record tool/source/schema/chunker pins, source inventory and filters, configuration, host, warmup policy, and input hashes. Do not overwrite prior evidence.

## Review focus

- Different questions about the same file do not make a repository holdout.
- A capped result list cannot distinguish a missing candidate from a low rank.
- Ambiguity, legitimate empty output, infrastructure failure, and unsupported coverage are distinct.
- Output sizes and command time are comparable only with modes and limits explicitly reported.
- A speedup cannot re-order ties or discard filtered or weighted winners.

### Task 1: Freeze the workload and repository split

**Files:** `experiments/retrieval-generalization/{README.md,corpora.toml,cases.json}`.

- [ ] Predeclare at least two public repositories absent from development runs and chosen to differ in domain and language; add a distinct third if measurement cost allows. Pin revisions before seeing candidate results. Keep Linux, Keycloak and existing frozen datasets as controls, not fresh holdouts.
- [ ] Sample the same query classes per repo: exact, ambiguous/qualified, behavior, path-scoped, and no-result controls. Give every case an ID, repository and revision, query, class, filters, independently verified accepted file/spans, and label source. Preserve qualified alternatives; leave ambiguous cases unscored if acceptable answers cannot be enumerated reliably.
- [ ] Validate duplicate IDs, wrong pins, missing source lines, reused repositories between splits, and no-result labels with contradictory answers. Expected: malformed inputs fail before any CLI is spawned.

### Task 2: Capture and classify CLI behavior

**Files:** `experiments/retrieval-generalization/{run.py,test_run.py}`; consult `crates/codecite-bench/src/{main,answer}.rs` and `crates/codecite-core/src/store/search.rs`.

- [ ] Test offline with fixtures: target in output; target absent but in the candidate set; target in index but absent from the candidate set; target absent from the index; unresolved (scope/rank cannot be proved); explicit unsupported/error and true empty. Assert pin, path and span matching, not symbol spelling alone.
- [ ] Implement a runner over the frozen case format. Rank metrics consume observed CLI results. The separate Codecite explanation uses the same pinned read-only Store input, production term generation and scope, and actual untruncated candidate membership; use an experiment-only internal diagnostic if required, not a public production flag. Never infer candidate absence from a top-50 list. Persist machine-readable reason and evidence per missed answer.
- [ ] Run one untimed sweep, then at least five paired balanced A/B–B/A sweeps with one fresh process per call and no concurrent jobs. Report startup-inclusive p50/p95, distinct queries, errors, bytes, Hit@1/@10, MRR@10, and counts by failure class and repository. Verify citation round trips outside timed runs. Record unmeasured strata instead of widening them after the fact.

### Task 3: Select one library change from the evidence

- [ ] Only choose a relevance change when the same diagnosis repeats across unrelated repositories. Compare exact/qualified definition lookup versus lexical discovery instead of adding per-corpus boosts. Require no frozen history repository loss and make protected bucket regressions visible. A new query/weight candidate is a separate decision with a predeclared acceptance bar.
- [ ] If latency dominates, profile existing full ranking and consider an exact bounded top-k fast path. Require ordered result, score and tie parity (including path/role filtering, rare-name boosts and no-result cases); use the exhaustive ranker whenever the bound cannot certify equivalence. Do not claim a relevance improvement from an exact speedup.
- [ ] Report comparison on repository-disjoint holdouts once per accepted design. After looking at a holdout, move it to regression and select a new one for subsequent tuning. Keep the agent evals as downstream verification, not the benchmark or the acceptance oracle for this work.
