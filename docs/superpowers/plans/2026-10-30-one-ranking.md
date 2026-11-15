# One ranking over code and prose (#183) Implementation Plan

**Goal:** `Store::search` runs one full-text query over every chunk and returns
the results in score order. The three result slots held for spec sections, the
spec-first alternation and the separate `spec_fts` query go. Any tuning is
judged only on the history benchmark, and no repository may lose.

**Architecture:** today `search` ranks code from `chunk_fts` and specs from
the stemmed `spec_fts`, then `blend` merges the two lists. After this change
`ranked` runs once over `chunk_fts`. Spec chunks are already in that table, so
they compete on the same BM25 scale as code and generic documents. `--only`
becomes a `WHERE` filter on the same query. `spec_fts` then has no reader, so
migration 0009 drops it, its trigger and its writes. Its stemming and stopword
list go with it.

**Tech stack:** Rust, rusqlite (bundled SQLite 3.54.1, FTS5), the history
benchmark in `evals/history/`.

## What the benchmark can and can't see

The history benchmark indexes five repositories and no fetched specs. Every
change it measures is to code and repository prose (READMEs, `docs/`, ADRs).
Dropping the spec slots can't move its rows, since none of its corpora has a
spec. The unified query must give the same rows as the base, so Task 5 checks
that first. Only a change to the shared weights, which Task 6 sweeps, can move
the benchmark.

What the benchmark can't see is the cost on spec queries. Specs lose Porter
stemming and the stopword list. The retrieval benchmark measured them on
identity-model, and the general-purpose design no longer tunes against it.
The PR names that cost. The A/B eval is still the judge of specs.

## Decisions (DEC-009 in the loop's journal)

- **Drop `spec_fts`, don't leave it unread.** A table that is written on every
  index and never read costs index time and space. Migration 0009 drops the
  table and its trigger. #191 can bring stemming back in the spec adapter if
  the A/B eval says specs need it.
- **Scope stays.** `--only spec` and `--only code` keep their meaning. They
  become `AND (?scope IS NULL OR is_spec = ?scope)` in the one query.
- **Tuning is bounded.** Sweep only the weights that apply to every chunk:
  the `Docs` role weight (prose against code) and `NAME_BOOST`. Adopt a value
  only if no repository is lower on Recall@1, Recall@10 or MRR and at least
  one is higher. Otherwise keep today's values and report the sweep anyway.

## Files

- Modify: `crates/codecite-core/src/store/search.rs`: `search`, `ranked`,
  `add_older_versions`. Remove `blend`, `SPEC_SLOTS`, `spec_fts_query`,
  `STOPWORDS` and their tests.
- Modify: `crates/codecite-core/src/store/write.rs` (`insert_chunks` stops
  writing `spec_fts`), `crates/codecite-core/src/store.rs` (register 0009;
  migrations 5 and 8 must still run on an old store before 0009 drops it).
- Create: `crates/codecite-core/migrations/0009_one_ranking.sql`.
- Modify: `crates/codecite-index/src/index.rs:1973` (a test asserting on
  `spec_fts`).
- Modify: `docs/design.md` (schema block, ER diagram, search section,
  decisions table), `docs/usage.md:224-229`, and the MCP `search` tool
  description if it says how specs and code merge.
- Create: `experiments/one-ranking/` (sweep script, results, README).
- Modify: `evals/history/results/` (rerun).

---

### Task 1: Pin one ranking in a failing test

- [ ] **Step 1:** In `search.rs` tests, rewrite
  `a_spec_section_is_not_buried_under_code_that_repeats_its_term` as
  `results_are_in_one_score_order_across_code_and_specs`, using
  `buried_spec()`. Assert that the scores don't increase down the list, that
  the spec section ranks where its own score puts it (below the four code
  chunks), and that `--only spec` still returns only it.
- [ ] **Step 2:** Run `cargo test -p codecite-core --lib search` and watch it
  fail: today the spec result is first.

### Task 2: One query in `search`

- [ ] **Step 1:** `ranked` loses its `table` and `spec` arguments. It queries
  `chunk_fts` with `fts_query(query.text)` and filters by scope with
  `AND (?10 IS NULL OR (s.kind = 'document' AND d.text IS NULL) = ?10)`. Bind
  `query.only.map(|s| s == Scope::Spec)`.
  *Learn:* `Option::map` turns `Option<Scope>` into the `Option<bool>` that
  rusqlite binds as `NULL` or `0`/`1`. No `match` is needed.
- [ ] **Step 2:** `search` calls `ranked` once, then `add_older_versions` on
  the results whose `role == "spec"`.
  *Learn:* `iter_mut().filter(..)` hands out `&mut` to only the matching
  elements. `add_older_versions` can take `impl IntoIterator<Item = &mut
  SearchResult>` instead of a slice.
- [ ] **Step 3:** Delete `blend`, `SPEC_SLOTS`, `spec_fts_query`, `STOPWORDS`
  and their tests: `a_query_that_names_a_symbol_keeps_score_order_with_a_spec_slot`,
  `spec_queries_drop_stopwords`, `stopwords_are_sorted_lowercase_and_unique`,
  `spec_search_is_stemmed_and_ignores_stopwords` and
  `code_results_are_ranked_as_before_specs_had_their_own_table`. Keep
  `code_search_is_not_stemmed_and_keeps_stopwords`, renamed
  `search_is_not_stemmed_and_keeps_stopwords`, and extend it to a spec
  section. Rewrite the doc comments on `search`, `ranked` and
  `SearchResult::score`.
- [ ] **Step 3b:** `boosted_names`/`named` was only used by `blend`. Keep the
  name boost in SQL and drop the `named` flag.
- [ ] **Step 4:** Run `cargo test -p codecite-core`. Task 1's test passes.
  Fix any other test that expected the spec-first order, by asserting score
  order rather than a position.
- [ ] **Step 5:** Commit `feat(search): rank code, prose and specs in one
  score order (#183)`.

### Task 3: Drop `spec_fts`

- [ ] **Step 1:** Write a failing test in `store.rs`: a store at version 8,
  which has `spec_fts`, opened read-write ends with no `spec_fts` table or
  `spec_fts_delete` trigger (`sqlite_master`). Its chunks still `MATCH` in
  `chunk_fts`.
- [ ] **Step 2:** `0009_one_ranking.sql`: `DROP TRIGGER IF EXISTS
  spec_fts_delete; DROP TABLE IF EXISTS spec_fts;` with a comment saying why.
  Register it. Migrations 5 and 8 still fill `spec_fts` on the way up, and
  0009 drops it. Keep them, since migrations are history.
- [ ] **Step 3:** `insert_chunks` stops writing `spec_fts`. Update the
  `write.rs` comments at :301 and :316 and the tests at :832-844. Update
  `index.rs:1973`.
- [ ] **Step 4:** `cargo test --workspace --locked` in the background (the
  index tests take about 2.5 min). Commit `feat(index): drop the spec-only
  full-text table (#183)`.

### Task 4: CLI check

- [ ] Index a corpus with a repository and a fetched spec (the
  `/var/tmp/182-cli` setup, or a local `http.server` serving an RFC HTML
  page). Run `codecite search` for a term both hold, with `--json`, and check
  that the scores fall down the list and that `--only spec` and `--only code`
  narrow it. Adversarial check: a query of only stopwords (`what is the`)
  returns results or an empty list, not an error or a full-table scan that
  hangs. Reopen an index built by the base branch's binary and check that the
  migration drops `spec_fts` and search works.

### Task 5: History benchmark, unified ranking

- [ ] Release build, then `evals/history/run.sh` in the background
  (`RG=/home/james/.pi/agent/bin/rg`). Compare each repository's `codecite`
  `all` row with `origin/fix/reindex-matches-fresh`'s `scorecard.md`. Expect
  identical rows. If any row is lower, it's a loss: park.

### Task 6: Bounded tuning sweep

- [ ] **Step 1:** `experiments/one-ranking/sweep.sh` sets the `Docs` weight
  in {0.6, 0.8, 1.2} and `NAME_BOOST` in {2.0, 5.0}, one knob at a time from
  today's values. For each, it patches the constant, rebuilds release, reruns
  the codecite arm of the benchmark and saves the `all` rows under
  `experiments/one-ranking/results/<knob>-<value>.md`.
  *Learn:* if the script needs an override, prefer a `cfg(feature)` or a
  temporary patch over an environment variable read at run time. Ranking
  constants stay `const`.
- [ ] **Step 2:** Write `experiments/one-ranking/README.md` with a table per
  knob and value: Recall@1, Recall@10 and MRR on each repository against the
  base.
- [ ] **Step 3:** Adopt a value only under the rule in Decisions. If none
  passes, keep today's values and say so in the README. Commit `chore(experiments):
  sweep the shared ranking weights on the history benchmark (#183)`. If a
  value is adopted, commit the constant change separately with its doc and
  rerun Task 5.

### Task 7: Docs

- [ ] `docs/design.md`: the search section's spec-table paragraph (:351) and
  merging paragraph (:355) become one paragraph. One query over `chunk_fts`
  ranks code, prose and specs together, in score order. Specs are no longer
  stemmed, and the retrieval benchmark's spec numbers were measured under the
  old scheme. Also update the schema block (:187-188, :218-219), the ER
  diagram (:233, :283), the Write step's mention of both tables, and the
  vectors decision row (:596, "fills the spec slots").
- [ ] `docs/usage.md:224-229`: results come in one score order, and `--only`
  narrows to one kind.
- [ ] The MCP `search` description, if it says how kinds merge.
- [ ] Commit `docs: one ranking over code and prose (#183)`.

### Task 8: Final benchmark and gates

- [ ] Rerun the history benchmark on the final head and commit
  `evals/history/results/`. Run the full gates: fmt, clippy, test and deny,
  plus evals pytest if `evals/` changed. Record each repository's row against
  the base for the PR body.
