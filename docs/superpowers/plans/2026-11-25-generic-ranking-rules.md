# Generic ranking rules (#300)

**Goal:** no ranking rule names a language, and the path conventions that
give tests, examples and docs their lower weights become defaults a
manifest can override per source.

**Why this shape:** the first attempt dropped the role weights entirely and
lost on identity-model (Recall@10 0.77 → 0.65). The owner's Decision on
#300 (2026-11-26) keeps the conventions as the documented defaults and adds
an optional per-source manifest setting that replaces them. The name-count
change alone is neutral on the history benchmark and ships as it was.

**Architecture:**

- `codecite-core` gets `RoleGlobs`, the manifest's override as data (three
  optional glob lists), and `RoleRules`, those globs compiled. `Role::of`
  keeps its convention and becomes the default case of a new
  `Role::classify`.
- The globs are stored on the source row (`source.role_globs`, JSON text,
  NULL for the defaults; migration 0015), because the write path computes
  `document.role` from the source row, and an unchanged file is not
  rewritten: when a source's globs change, `upsert_source` re-derives the
  role of every document in it.
- Every reader that recomputed a role from the path (search results, grep,
  `definition`, `references`) reads the stored `document.role` instead, so
  results and ranking agree with the manifest.
- The manifest gets an optional `roles` table on `git` and `local` sources.

**Manifest form:**

```toml
[[source]]
url = "https://github.com/acme/app.git"
commit = "…"
use = "adapt"

[source.roles]
test = ["spec/**", "**/*_spec.rb"]
example = ["playground/**"]
```

Each list that is given replaces that role's convention for the source; an
empty list means no file has that role; a role left out keeps its
convention. Precedence stays test, then example, then docs, then source.
Globs match source-relative paths as `include` does (`*` stays in one
directory, `**` crosses them).

## Task 1: language-free name count

Cherry-pick `f8110dd` from `parked/generic-ranking-rules-attempt1`
(`BOOSTED_NAME_COUNT` loses its `d.language = 'java'` clause, with the test
`same_name_methods_count_with_their_class_in_any_language`). Run
`cargo test -p codecite-core boosted` and commit as picked.

## Task 2: role rules in core

Files: `crates/codecite-core/src/role.rs`, `crates/codecite-core/src/lib.rs`,
`crates/codecite-core/src/error.rs`.

1. Tests first in `role.rs`:
   - `Role::classify(path, "git", true, &RoleRules::default())` equals
     `Role::of` for every path in the existing tests.
   - With `test = ["spec/**"]`, `spec/models/user_spec.rb` is a test and
     `tests/foo.py` is source (the list replaces the convention).
   - With `example = []`, `examples/app.py` is source.
   - With `docs = ["notes/**"]`, `notes/plan.txt` is docs, `README.md` is
     source.
   - Test still wins over example when both globs match.
   - A document source ignores the rules (spec, or docs with text).
   - `RoleRules::new(&globs)` with `test = ["a/[b"]` is an error naming the
     glob.
   - `Role::parse("example") == Some(Role::Example)`, and `None` for
     anything else; `parse(r.as_str()) == Some(r)` for every role.
2. Implement:
   - `#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
     pub struct RoleGlobs { test, example, docs: Option<Vec<String>> }`
     with `#[serde(default, skip_serializing_if = "Option::is_none")]`.
     Learn: `Option<Vec<_>>` tells "left out" (`None`, keep the convention)
     from "empty" (`Some(vec![])`, nothing has this role), which a bare
     `Vec` can't.
   - `pub struct RoleRules { test, example, docs: Option<GlobSet> }`,
     `Default` = every role on its convention, and
     `RoleRules::new(&RoleGlobs) -> Result<RoleRules>` compiling with
     `literal_separator(true)` as `walk.rs` and `context.rs` do.
   - `Error::RoleGlob { pattern, source: globset::Error }`.
   - `Role::classify(path, source_kind, has_text, &RoleRules)`: document
     sources as `of_stored`; otherwise each role's set when present, else
     its convention. `Role::of` and `Role::of_stored` delegate with
     `RoleRules::default()`. Learn: `Option::map_or_else` picks the glob
     set or the convention in one expression without an `if let`.
   - `Role::parse(&str) -> Option<Role>`.
   - Re-export `RoleGlobs` and `RoleRules` from `lib.rs`.
3. `cargo test -p codecite-core role`, clippy, commit
   `feat(core): role rules that a source's globs can override (#300)`.

## Task 3: store a source's role globs

Files: `crates/codecite-core/migrations/0015_role_globs.sql`,
`crates/codecite-core/src/store.rs`, `crates/codecite-core/src/store/write.rs`,
`crates/codecite-core/src/store/corpora.rs` and every `SourceRecord`
literal (`grep -rn "SourceRecord {" crates`).

1. Tests first in `write.rs`:
   - A source upserted with `test = ["spec/**"]` stores
     `spec/a.rb` as role `test` with `Role::Test.weight()`.
   - Upserting the same source again with no globs, without rewriting the
     document, turns `spec/a.rb` back to `source` with weight 1.0, and
     `tests/a.rb` (written under the globs as source) to `test`.
   - Upserting with unchanged globs leaves `document` untouched (check a
     role changed by hand stays, to prove nothing re-derives).
   - The migration test: a corpus at version 14 opens, gains a NULL
     `role_globs`, and its roles are unchanged.
2. Implement:
   - `0015_role_globs.sql`: `ALTER TABLE source ADD COLUMN role_globs TEXT;`
     registered `read_compatible: true` (an older build ignores the column
     and reads stored roles).
   - `SourceRecord` gains `role_globs: Option<&'a RoleGlobs>`; every caller
     passes `None` except the index path in Task 5.
   - `upsert_source` serialises the globs (`serde_json::to_string`, `None`
     → NULL), reads the stored value first, writes it, and when it differs
     re-derives `role`/`role_weight` for every document of the source in
     the same savepoint. Learn: `Savepoint` rolls back on drop unless
     committed, so an error mid-way leaves no half-updated roles.
   - `write_split_document` reads `kind, role_globs` and classifies with
     `RoleRules::new` of the parsed globs (default rules when NULL).
   - `fill_rank_inputs` (migration 10) keeps the defaults: no source has
     globs before 0015.
3. `cargo test -p codecite-core`, commit
   `feat(store): store a source's role globs and re-derive roles (#300)`.

## Task 4: read the stored role

Files: `crates/codecite-core/src/store/search.rs`, `grep.rs`,
`definition.rs`, `graph.rs`, `store.rs` (`register_functions` doc).

1. Test first: one corpus with a source whose globs make `spec/a.rb` a
   test; `search`, `grep`, `definition` report role `test` for it, and
   `definition` orders it after a source definition of the same name.
   (`references` reads the same column; cover it with a graph test if the
   SCIP fixture allows, else note it in the PR.)
2. Implement: each query selects `d.role`; the row reads it with
   `Role::parse(..).unwrap_or(Role::Source)` (a NULL role only on a corpus
   written before migration 10, which fills it). Drop the now-unused
   `has_text`/`path` reads that existed only for the role. Update the
   `codecite_role_weight` doc: it gives the convention's weight, not a
   source's override.
3. `cargo test -p codecite-core`, commit
   `refactor(search): read each document's stored role (#300)`.

## Task 5: the manifest's `roles`

Files: `crates/codecite-index/src/manifest.rs`, `crates/codecite-index/src/index.rs`
(and wherever `Walked` is built).

1. Tests first:
   - `manifest.rs`: a git and a local source parse `[source.roles]` with
     `test`, `example`, `docs`; an unknown key under `roles` is an error;
     a bad glob is an error naming the source and `roles.test`; `roles` on
     a `document` or `guidance` source is an error.
   - `index.rs`: index a local repo with `roles.test = ["spec/**"]`, search
     reports `spec/…` as `test`; reindex with the `roles` removed and no
     file changed, the role is `source`.
2. Implement: `#[serde(default)] pub roles: RoleGlobs` on `GitSource` and
   `LocalSource` (`#[serde(deny_unknown_fields)]` on `RoleGlobs`), validated
   at load with `RoleRules::new`; carried on `Walked` and passed to
   `SourceRecord` (`None` when all three are `None`, so an unconfigured
   source stores NULL).
3. `cargo test -p codecite-index manifest` and the index test by name (the
   full lib suite takes ~155 s; run it in the background), commit
   `feat(manifest): per-source role globs (#300)`.

## Task 6: docs

- `docs/design.md`: Ranking says role weights come from path conventions
  by default and a source's `roles` globs replace them; the name-boost
  count is language-free. Data model: `source.role_globs`. Write step:
  migration 0015 and the re-derive on a globs change. Readers use the
  stored role.
- `docs/usage.md`: `roles` row in the field table and a `[source.roles]`
  example with the replace/empty/left-out rules.
- Commit `docs: describe per-source role globs (#300)`.

## Task 7: gates and benchmarks

1. `cargo fmt --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked && cargo deny check`.
2. `cargo build --release --locked -p codecite -p codecite-bench`, then
   `evals/history/run.sh` (background). Compare each repository's pooled
   `all` codecite row with `origin/main`'s `scorecard.md`; any drop in
   Recall@1, Recall@10 or MRR parks the issue. Expected: identical, since
   no benchmark manifest sets `roles` and the name count was neutral.
3. Validate the generic retrieval benchmark's cases
   (`experiments/retrieval-generalization/`) with the release build; no
   case that passed on `main` may miss.
4. Commit `evals/history/results/` as
   `chore(evals): rerun the history benchmark for #300`.
