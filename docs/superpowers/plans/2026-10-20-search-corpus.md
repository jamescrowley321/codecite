# Search a Corpus with `codecite search` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement #26: `codecite search <corpus> "<query>"` returns ranked, cited chunks. Ranking is FTS5 `bm25()` with column weights (symbol, then path, then body), and an exact symbol-name match goes first. It takes `--language`, `--path <glob>` and `--limit` (default 10), and prints JSON (`--json`) or a short human form. Snippets are capped at 40 lines, code results carry `dirty`, and spec results carry their pinned citation and `license_class`. An index older than `max_age_days` is refused unless `--allow-stale`, every result carries `indexed_at`, and an unknown corpus, a stale index and an empty query are typed errors.

**Architecture:** The query engine lives in `codecite-core`, as `design.md` places it, so the MCP server (#28 onward) reuses it unchanged. A new `store/search.rs` adds `Store::search(&SearchQuery) -> Result<Vec<SearchResult>>` and `Store::freshness()`. `SearchResult` derives `serde::Serialize`, so the CLI's `--json` and the MCP server print the same objects. The query text is split by the same `text_terms` the index uses. Each term is lowercased, deduplicated and double-quoted (so FTS5 never reads user text as syntax), and the terms are joined with `OR`. One SQL statement joins `chunk_fts` to `chunk`, `document`, `source`, the parent chunk and `symbol`. It filters on `is_current`, `language = ?` and `path GLOB ?`, orders by "symbol name equals the query" and then by `-bm25(chunk_fts, 10, 5, 1)`, and applies the limit. Staleness needs the manifest's `max_age_days`, which a query has no manifest to read from. So `Store::set_indexed_at` now takes it and stores it in `meta.max_age_days` beside `meta.indexed_at`, and `freshness()` compares the two with SQLite's `julianday`. `Store::open_corpus(dir, name)` validates the name (`is_corpus_name`, moved from the binary) and turns a missing file into `Error::UnknownCorpus` with the corpora that exist. The binary adds a `Search` subcommand that opens the corpus read-only and prints the results.

**Tech Stack:** Rust 1.99.1 (edition 2024), rusqlite 0.40 (bundled SQLite, FTS5), `serde` (already in the workspace) in core, and `serde_json` 1 (new workspace dependency, MIT OR Apache-2.0, already in `Cargo.lock` through tree-sitter's build). `cargo deny check` is unchanged. Prototyped: every gate passes (245 passed, 6 ignored), and the real CLI searches this repo plus the seven real spec pages (output in Task 2, Step 6).

**Spec:** Issue #26; `docs/design.md#indexing-and-retrieval` (Ranking, Freshness) and `docs/design.md#interfaces`, which Task 3 updates. The decisions that the issue and design.md leave open are recorded in the loop's DEC-016 and restated in the Global Constraints.

## Global Constraints

- Matching: `text_terms(query)`, lowercased (the `unicode61` tokenizer folds case anyway) and deduplicated, each term quoted with `"` doubled inside, joined with ` OR `. A query with no terms (`""`, `"  !! "`) is `Error::EmptyQuery`, checked before anything else.
- Ranking: `score = -bm25(chunk_fts, 10.0, 5.0, 1.0)` (symbol, path, terms), so higher is better. A chunk whose symbol's `display_name` equals the trimmed query, case and all, sorts first, and the rest follow by score, then chunk ID. The exact match's own score is not changed, so `score` is not monotonic across the boundary.
- Only sources with `is_current = 1` are searched: an older pinned document version is not a search result. The `SourceRecord::is_current` doc already says it marks "the version … that queries cite by default".
- Filters: `--language` is `document.language = ?` (the names the chunker writes: `rust`, `typescript`, `markdown`, …; spec documents have none). `--path` is SQLite `GLOB` on `document.path`: `*` crosses `/`, and case matters. `--limit` is a `u32` of at least 1 (clap rejects 0).
- Result fields, in order: `citation`, `kind`, `symbol` (omitted when none), `score`, `license_class`, `indexed_at` (the source's), `dirty` (code only; omitted for specs), `snippet` (the first `MAX_SNIPPET_LINES` = 40 lines). A code citation is `<source name>@<stored revision, all 40 hex>:<path>#L<a>-L<b>`, or `#L<a>` for one line. A spec citation is `<doc>@sha256:<first 12 hex>#section-<number>`, or `#section-<parent's number>/step-<number>` for a step. An appendix section is numbered `A.1`, which the strict `SpecCitation` parser does not accept yet. That is left to the `read`/`cite` work.
- Freshness: `set_indexed_at(max_age_days)` writes `meta.indexed_at` and `meta.max_age_days` together. Stale means `julianday('now') - julianday(indexed_at) > max_age_days`. A file with no `indexed_at` (its first run never finished) is `Error::InvalidCorpus`, and one with no `max_age_days` (indexed before this change) uses `DEFAULT_MAX_AGE_DAYS` = 30, which the manifest's default now names too.
- Errors (core, typed): `InvalidCorpusName { name }`, `UnknownCorpus { corpus, known }` (sorted `.db` stems in the data directory; a missing directory has none), `StaleIndex { corpus, indexed_at, max_age_days }`, `EmptyQuery`. The binary prints them through `anyhow`, so `codecite search` exits 1 with the message. A search that matches nothing prints `no results` (or `[]`) and exits 0.
- Human form, per result: `<rank>. <citation>`, then `   <kind>[ <symbol>] · score <2 decimals> · <license_class> · indexed <indexed_at>[ · dirty]`, then the snippet's first 3 lines indented 5 spaces.
- The workspace forbids `unsafe_code` and warns on `missing_docs`; every public item and field gets a doc comment.
- Gates: `cargo fmt --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked && cargo deny check`. Before the PR, also run `/var/tmp/search-proto/real-run.sh <worktree>` (Task 2, Step 6) and put its output in the PR body.

## Review Focus

- FTS5 input handling (`query_syntax_is_searched_as_words`): `validate AND`, `NEAR(validate`, `path:validate` and `"validate*` all search as plain words instead of failing or filtering a column. `fts_query` quotes each term, and it is the only place user text reaches `MATCH`.
- The symbol boost (`an_exact_symbol_name_comes_first`): the fixture makes BM25 alone rank another chunk above `validate`. The test asserts that chunk has the higher score and still comes second.
- Staleness (`a_stale_index_is_refused_unless_allowed`, `the_stored_max_age_wins_and_defaults_to_30`, and the CLI's `a_stale_index_is_refused_unless_allowed`) backdates `meta.indexed_at` through a raw connection. The corpus is opened read-only, so the check runs on a handle that cannot write.
- Acceptance (issue #26) through the binary, in `crates/codecite/tests/search.rs`: JSON shape with `dirty` and `indexed_at`, each filter, the human form, stale refusal and `--allow-stale`, and the three typed errors.

---

### Task 1: Search the store (`codecite-core`)

**Files:**
- Create: `crates/codecite-core/src/store/search.rs`
- Modify: `crates/codecite-core/src/error.rs`, `crates/codecite-core/src/store.rs`, `crates/codecite-core/src/store/write.rs`, `crates/codecite-core/src/lib.rs`, `crates/codecite-core/Cargo.toml`, `Cargo.toml`
- Modify: `crates/codecite-index/src/index.rs`, `crates/codecite-index/src/manifest.rs` (the `set_indexed_at` call and the default age only)

**Interfaces:**
- Produces (for Task 2 and the MCP server): `Store::open_corpus(dir: &Path, corpus: &str) -> Result<Store>`, `is_corpus_name(&str) -> bool`, `Store::search(&self, &SearchQuery<'_>) -> Result<Vec<SearchResult>>`, `Store::freshness(&self) -> Result<Freshness>`, `SearchQuery { text, language, path, limit, allow_stale }`, `SearchResult` (`Serialize`), `MAX_SNIPPET_LINES`, `DEFAULT_MAX_AGE_DAYS`. New `Error` variants: `InvalidCorpusName`, `UnknownCorpus`, `StaleIndex`, `EmptyQuery`.
- Changes: `Store::set_indexed_at(&mut self, max_age_days: u32)`, which previously took no arguments.

- [ ] **Step 1: Write the failing tests**

This adds the error, store and write tests. It creates `store/search.rs` holding only its test module, registers it with `mod search;`, and adds the dev-dependencies the tests use (`serde_json`, `tempfile`) plus the workspace's `serde_json` entry:

```diff
diff --git a/Cargo.toml b/Cargo.toml
index f3de494..8480b75 100644
--- a/Cargo.toml
+++ b/Cargo.toml
@@ -31,6 +31,7 @@ anyhow = "1"
 directories = "6"
 rusqlite = { version = "0.40", features = ["bundled"] }
 serde = { version = "1", features = ["derive"] }
+serde_json = "1"
 toml = "1"
 globset = "0.4"
 sha2 = "0.11"
diff --git a/crates/codecite-core/Cargo.toml b/crates/codecite-core/Cargo.toml
index 1553f3a..847dc64 100644
--- a/crates/codecite-core/Cargo.toml
+++ b/crates/codecite-core/Cargo.toml
@@ -12,7 +12,10 @@ workspace = true
 
 [dependencies]
 rusqlite.workspace = true
+serde.workspace = true
 thiserror.workspace = true
 
 [dev-dependencies]
 proptest.workspace = true
+serde_json.workspace = true
+tempfile.workspace = true
diff --git a/crates/codecite-core/src/error.rs b/crates/codecite-core/src/error.rs
index b0a16c8..365ab1d 100644
--- a/crates/codecite-core/src/error.rs
+++ b/crates/codecite-core/src/error.rs
@@ -93,6 +93,39 @@ mod tests {
         assert_eq!(err.source().unwrap().to_string(), msg);
     }
 
+    #[test]
+    fn unknown_corpus_lists_the_known_ones() {
+        let err = Error::UnknownCorpus {
+            corpus: "identty".into(),
+            known: vec!["identity".into(), "specs".into()],
+        };
+        assert_eq!(
+            err.to_string(),
+            "unknown corpus `identty`; known corpora: identity, specs"
+        );
+        let err = Error::UnknownCorpus {
+            corpus: "identity".into(),
+            known: vec![],
+        };
+        assert_eq!(
+            err.to_string(),
+            "unknown corpus `identity`; no corpus has been indexed yet; run codecite index"
+        );
+    }
+
+    #[test]
+    fn stale_index_names_the_age_limit() {
+        let err = Error::StaleIndex {
+            corpus: "identity".into(),
+            indexed_at: "2026-09-01T00:00:00Z".into(),
+            max_age_days: 30,
+        };
+        assert_eq!(
+            err.to_string(),
+            "corpus `identity` was indexed at 2026-09-01T00:00:00Z, more than 30 days ago; re-run codecite index, or allow a stale index"
+        );
+    }
+
     #[test]
     fn schema_too_new_display_names_path_and_versions() {
         let err = Error::SchemaTooNew {
diff --git a/crates/codecite-core/src/store.rs b/crates/codecite-core/src/store.rs
index d5bcb3f..7b674fa 100644
--- a/crates/codecite-core/src/store.rs
+++ b/crates/codecite-core/src/store.rs
@@ -9,6 +9,8 @@ use rusqlite::{Connection, ErrorCode, OpenFlags, OptionalExtension, TransactionB
 
 use crate::{Error, Result};
 
+mod search;
+
 mod write;
 pub use write::{ChunkRecord, DocumentRecord, SourceRecord, SymbolRecord};
 
@@ -649,6 +651,43 @@ mod tests {
         fs::remove_dir_all(&dir).unwrap();
     }
 
+    #[test]
+    fn corpus_names_are_plain_file_names() {
+        for good in ["identity", "oidc-impls", "rfc_9449", "v1.2"] {
+            assert!(is_corpus_name(good), "{good}");
+        }
+        for bad in ["", ".hidden", "..", "../x", "a/b", "a\\b", "名前", "a b"] {
+            assert!(!is_corpus_name(bad), "{bad:?}");
+        }
+    }
+
+    #[test]
+    fn open_corpus_names_the_known_corpora() {
+        let dir = scratch("open-corpus");
+        let err = Store::open_corpus(&dir.join("missing"), "identity")
+            .err()
+            .unwrap();
+        assert!(
+            matches!(&err, Error::UnknownCorpus { known, .. } if known.is_empty()),
+            "{err:?}"
+        );
+        drop(Store::open_read_write(&dir.join("specs.db")).unwrap());
+        drop(Store::open_read_write(&dir.join("identity.db")).unwrap());
+        fs::write(dir.join("notes.txt"), "not a corpus").unwrap();
+        let err = Store::open_corpus(&dir, "identty").err().unwrap();
+        assert!(
+            matches!(&err, Error::UnknownCorpus { corpus, known }
+                if corpus == "identty" && known == &["identity", "specs"]),
+            "{err:?}"
+        );
+        let err = Store::open_corpus(&dir, "../specs").err().unwrap();
+        assert!(matches!(err, Error::InvalidCorpusName { .. }), "{err:?}");
+        let store = Store::open_corpus(&dir, "specs").unwrap();
+        assert_eq!(store.path(), dir.join("specs.db"));
+        drop(store);
+        fs::remove_dir_all(&dir).unwrap();
+    }
+
     #[test]
     fn every_migration_file_is_registered() {
         let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/migrations");
diff --git a/crates/codecite-core/src/store/search.rs b/crates/codecite-core/src/store/search.rs
new file mode 100644
index 0000000..8aee166
--- /dev/null
+++ b/crates/codecite-core/src/store/search.rs
@@ -0,0 +1,420 @@
+//! Searching a corpus: BM25 over the full-text index, with an exact symbol
+//! name first, and each result cited.
+
+#[cfg(test)]
+mod tests {
+    use super::*;
+    use crate::{ChunkRecord, Citation, DocumentRecord, Keyword, SourceRecord, SymbolRecord};
+    use std::path::{Path, PathBuf};
+
+    const HEAD: &str = "0123456789abcdef0123456789abcdef01234567";
+    /// The one function named `validate`. Its long body dilutes its BM25
+    /// score below another chunk's.
+    const VALIDATE: &str = "pub fn validate(token: &str) -> Result<Claims, Error> {
+    let claims = decode(token)?;
+    check_expiry(&claims)?;
+    check_audience(&claims)
+}";
+    const SHA256: &str = "1a2b3c4d5e6f00000000000000000000000000000000000000000000000000ff";
+
+    /// A store holding a small fixture corpus, in its own directory.
+    struct Fixture {
+        store: Store,
+        dir: tempfile::TempDir,
+    }
+
+    impl Fixture {
+        fn search(&self, text: &str) -> Vec<SearchResult> {
+            self.store.search(&query(text)).unwrap()
+        }
+
+        fn citations(&self, query: &SearchQuery<'_>) -> Vec<String> {
+            let results = self.store.search(query).unwrap();
+            results.into_iter().map(|r| r.citation).collect()
+        }
+
+        fn path(&self) -> PathBuf {
+            self.dir.path().join("demo.db")
+        }
+
+        fn raw(&self) -> rusqlite::Connection {
+            rusqlite::Connection::open(self.path()).unwrap()
+        }
+    }
+
+    fn query(text: &str) -> SearchQuery<'_> {
+        SearchQuery {
+            text,
+            language: None,
+            path: None,
+            limit: 10,
+            allow_stale: false,
+        }
+    }
+
+    fn code_source(name: &str) -> SourceRecord<'_> {
+        SourceRecord {
+            name,
+            kind: "local",
+            url: "/src/app",
+            revision: HEAD,
+            license_spdx: Some("MIT"),
+            license_class: "adapt",
+            is_current: true,
+            obsoleted_by: None,
+            updated_by: None,
+        }
+    }
+
+    fn chunk<'a>(
+        kind: &'a str,
+        lines: (u32, u32),
+        text: &'a str,
+        symbol: Option<&'a str>,
+    ) -> ChunkRecord<'a> {
+        ChunkRecord {
+            kind,
+            start_line: lines.0,
+            end_line: lines.1,
+            text,
+            number: None,
+            anchor: None,
+            title: None,
+            parent: None,
+            strength: &[],
+            symbol: symbol.map(|name| SymbolRecord {
+                id: format!("codecite . app . {name}()."),
+                display_name: name,
+                kind: "function",
+            }),
+        }
+    }
+
+    fn file<'a>(
+        path: &'a str,
+        language: &'a str,
+        dirty: bool,
+        chunks: Vec<ChunkRecord<'a>>,
+    ) -> DocumentRecord<'a> {
+        DocumentRecord {
+            path,
+            language: Some(language),
+            content_hash: path.as_bytes(),
+            size_bytes: 1,
+            dirty,
+            chunks,
+        }
+    }
+
+    /// One spec version: section 4.3 with step 2, which holds MUST.
+    fn spec<'a>(text: &'a str) -> DocumentRecord<'a> {
+        let section = ChunkRecord {
+            kind: "section",
+            number: Some("4.3"),
+            anchor: Some("checking"),
+            title: Some("Checking DPoP Proofs"),
+            ..chunk("section", (10, 20), "Checking DPoP Proofs", None)
+        };
+        let step = ChunkRecord {
+            kind: "step",
+            number: Some("2"),
+            anchor: Some("checking"),
+            parent: Some(0),
+            strength: &[Keyword::Must],
+            ..chunk("step", (12, 13), text, None)
+        };
+        DocumentRecord {
+            path: "rfc9449",
+            language: None,
+            content_hash: text.as_bytes(),
+            size_bytes: 1,
+            dirty: false,
+            chunks: vec![section, step],
+        }
+    }
+
+    fn spec_source<'a>(sha256: &'a str, is_current: bool) -> SourceRecord<'a> {
+        SourceRecord {
+            name: "rfc9449",
+            kind: "document",
+            url: "https://www.rfc-editor.org/rfc/rfc9449.html",
+            revision: sha256,
+            license_spdx: Some("IETF Trust"),
+            license_class: "quote",
+            is_current,
+            obsoleted_by: None,
+            updated_by: None,
+        }
+    }
+
+    fn fixture() -> Fixture {
+        let dir = tempfile::tempdir().unwrap();
+        let mut store = Store::open_read_write(&dir.path().join("demo.db")).unwrap();
+        let app = store.upsert_source(&code_source("app")).unwrap();
+        let docs = [
+            // Says "validate" most often, but defines nothing by that name.
+            file(
+                "src/checks.rs",
+                "rust",
+                false,
+                vec![chunk(
+                    "function",
+                    (1, 3),
+                    "fn checks() { validate(); validate(); validate(); }",
+                    Some("checks"),
+                )],
+            ),
+            file(
+                "src/validate.rs",
+                "rust",
+                true,
+                vec![chunk("function", (5, 9), VALIDATE, Some("validate"))],
+            ),
+            file(
+                "web/validate_token.ts",
+                "typescript",
+                false,
+                vec![chunk(
+                    "function",
+                    (1, 9),
+                    "export function validateToken(token) { return validate(token); }",
+                    Some("validateToken"),
+                )],
+            ),
+            file(
+                "README.md",
+                "markdown",
+                false,
+                vec![chunk(
+                    "section",
+                    (1, 2),
+                    "# App\nValidates the token.",
+                    None,
+                )],
+            ),
+        ];
+        for doc in &docs {
+            store.write_document(app, doc).unwrap();
+        }
+        let old = "a".repeat(64);
+        let current = store.upsert_source(&spec_source(SHA256, true)).unwrap();
+        store
+            .write_document(current, &spec("The jti MUST be unique."))
+            .unwrap();
+        let older = store.upsert_source(&spec_source(&old, false)).unwrap();
+        store
+            .write_document(older, &spec("The jti MUST be unique, said the old one."))
+            .unwrap();
+        store.set_indexed_at(30).unwrap();
+        drop(store);
+        let store = Store::open_read_only(&dir.path().join("demo.db")).unwrap();
+        Fixture { store, dir }
+    }
+
+    /// Moves the corpus's `indexed_at` back by `days`.
+    fn backdate(path: &Path, days: u32) {
+        rusqlite::Connection::open(path)
+            .unwrap()
+            .execute(
+                "UPDATE meta SET value = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?1)
+                 WHERE key = 'indexed_at'",
+                [format!("-{days} days")],
+            )
+            .unwrap();
+    }
+
+    #[test]
+    fn an_exact_symbol_name_comes_first() {
+        let f = fixture();
+        let results = f.search("validate");
+        assert_eq!(results[0].symbol.as_deref(), Some("validate"));
+        assert_eq!(
+            results[0].citation,
+            format!("app@{HEAD}:src/validate.rs#L5-L9")
+        );
+        // Without the boost, BM25 alone ranks a chunk above it.
+        assert!(results[1..].iter().any(|r| r.score > results[0].score));
+        let scores: Vec<f64> = results[1..].iter().map(|r| r.score).collect();
+        assert!(scores.windows(2).all(|w| w[0] >= w[1]), "{scores:?}");
+    }
+
+    #[test]
+    fn a_symbol_match_outranks_the_same_words_in_text() {
+        let f = fixture();
+        let results = f.search("validate token");
+        assert_eq!(results[0].symbol.as_deref(), Some("validateToken"));
+    }
+
+    #[test]
+    fn results_carry_their_citation_license_and_freshness() {
+        let f = fixture();
+        let results = f.search("validate");
+        let code = results
+            .iter()
+            .find(|r| r.symbol.as_deref() == Some("validate"))
+            .unwrap();
+        assert_eq!(code.kind, "function");
+        assert_eq!(code.license_class, "adapt");
+        assert_eq!(code.dirty, Some(true));
+        assert_eq!(code.snippet, VALIDATE);
+        assert_eq!(code.indexed_at.len(), "2026-10-20T06:00:00Z".len());
+        assert!(matches!(code.citation.parse(), Ok(Citation::Code(_))));
+
+        let results = f.search("jti");
+        assert_eq!(results.len(), 1, "only the current version");
+        let step = &results[0];
+        assert_eq!(
+            step.citation,
+            "rfc9449@sha256:1a2b3c4d5e6f#section-4.3/step-2"
+        );
+        assert!(matches!(step.citation.parse(), Ok(Citation::Spec(_))));
+        assert_eq!(step.kind, "step");
+        assert_eq!(step.license_class, "quote");
+        assert_eq!(step.dirty, None);
+        assert_eq!(step.snippet, "The jti MUST be unique.");
+        let section = &f.search("checking dpop")[0];
+        assert_eq!(section.citation, "rfc9449@sha256:1a2b3c4d5e6f#section-4.3");
+    }
+
+    #[test]
+    fn json_has_the_documented_shape() {
+        let f = fixture();
+        let code = serde_json::to_value(&f.search("validate")[0]).unwrap();
+        assert_eq!(
+            code["citation"],
+            format!("app@{HEAD}:src/validate.rs#L5-L9")
+        );
+        assert_eq!(code["symbol"], "validate");
+        assert_eq!(code["license_class"], "adapt");
+        assert_eq!(code["dirty"], true);
+        assert!(code["score"].is_f64());
+        let spec = serde_json::to_value(&f.search("jti")[0]).unwrap();
+        // A `Value`'s keys are sorted. A spec result has no symbol or dirty.
+        let keys: Vec<&String> = spec.as_object().unwrap().keys().collect();
+        assert_eq!(
+            keys,
+            [
+                "citation",
+                "indexed_at",
+                "kind",
+                "license_class",
+                "score",
+                "snippet"
+            ]
+        );
+    }
+
+    #[test]
+    fn filters_narrow_the_results() {
+        let f = fixture();
+        let by_language = SearchQuery {
+            language: Some("typescript"),
+            ..query("validate")
+        };
+        assert_eq!(
+            f.citations(&by_language),
+            [format!("app@{HEAD}:web/validate_token.ts#L1-L9")]
+        );
+        let by_path = SearchQuery {
+            path: Some("src/*"),
+            ..query("validate")
+        };
+        let paths = f.citations(&by_path);
+        assert_eq!(paths.len(), 2, "{paths:?}");
+        assert!(paths.iter().all(|c| c.contains(":src/")), "{paths:?}");
+        let limited = SearchQuery {
+            limit: 1,
+            ..query("validate")
+        };
+        assert_eq!(f.citations(&limited).len(), 1);
+        let nothing = SearchQuery {
+            language: Some("go"),
+            ..query("validate")
+        };
+        assert!(f.citations(&nothing).is_empty());
+    }
+
+    #[test]
+    fn query_syntax_is_searched_as_words() {
+        let f = fixture();
+        for text in [
+            "validate AND",
+            "NEAR(validate",
+            "path:validate",
+            "\"validate*",
+        ] {
+            let results = f.store.search(&query(text)).unwrap();
+            assert!(!results.is_empty(), "{text}");
+        }
+        assert_eq!(
+            fts_query("say \"hi\" NEAR x*").as_deref(),
+            Some("\"say\" OR \"hi\" OR \"near\" OR \"x\"")
+        );
+    }
+
+    #[test]
+    fn an_empty_query_is_an_error() {
+        let f = fixture();
+        for text in ["", "   ", "-- !!"] {
+            let err = f.store.search(&query(text)).unwrap_err();
+            assert!(matches!(err, Error::EmptyQuery), "{text:?}: {err:?}");
+        }
+    }
+
+    #[test]
+    fn a_stale_index_is_refused_unless_allowed() {
+        let f = fixture();
+        backdate(&f.path(), 31);
+        let err = f.store.search(&query("validate")).unwrap_err();
+        assert!(
+            matches!(
+                &err,
+                Error::StaleIndex { corpus, max_age_days: 30, .. } if corpus == "demo"
+            ),
+            "{err:?}"
+        );
+        let stale = SearchQuery {
+            allow_stale: true,
+            ..query("validate")
+        };
+        let results = f.store.search(&stale).unwrap();
+        assert!(!results.is_empty());
+
+        backdate(&f.path(), 29);
+        assert!(f.store.search(&query("validate")).is_ok());
+    }
+
+    #[test]
+    fn the_stored_max_age_wins_and_defaults_to_30() {
+        let f = fixture();
+        backdate(&f.path(), 8);
+        f.raw()
+            .execute("UPDATE meta SET value = '7' WHERE key = 'max_age_days'", [])
+            .unwrap();
+        assert!(f.store.freshness().unwrap().is_stale());
+        f.raw()
+            .execute("DELETE FROM meta WHERE key = 'max_age_days'", [])
+            .unwrap();
+        let freshness = f.store.freshness().unwrap();
+        assert_eq!(freshness.max_age_days, DEFAULT_MAX_AGE_DAYS);
+        assert!(!freshness.is_stale());
+    }
+
+    #[test]
+    fn a_corpus_never_indexed_is_invalid() {
+        let f = fixture();
+        f.raw()
+            .execute("DELETE FROM meta WHERE key = 'indexed_at'", [])
+            .unwrap();
+        let err = f.store.search(&query("validate")).unwrap_err();
+        assert!(matches!(err, Error::InvalidCorpus { .. }), "{err:?}");
+    }
+
+    #[test]
+    fn snippets_stop_at_40_lines() {
+        let text: String = (1..=50).map(|i| format!("line {i}\n")).collect();
+        let s = snippet(&text);
+        assert_eq!(s.lines().count(), MAX_SNIPPET_LINES);
+        assert!(s.ends_with("line 40"));
+    }
+}
diff --git a/crates/codecite-core/src/store/write.rs b/crates/codecite-core/src/store/write.rs
index f2c80c2..4719e54 100644
--- a/crates/codecite-core/src/store/write.rs
+++ b/crates/codecite-core/src/store/write.rs
@@ -538,16 +538,18 @@ mod tests {
             .unwrap();
         store.set_dirty(id, "src/lib.rs", true).unwrap();
         assert_eq!(count(&store, "SELECT dirty FROM document"), 1);
-        store.set_indexed_at().unwrap();
-        let at: String = store
-            .conn
-            .query_row(
-                "SELECT value FROM meta WHERE key = 'indexed_at'",
-                [],
-                |row| row.get(0),
-            )
-            .unwrap();
+        store.set_indexed_at(7).unwrap();
+        let meta = |key: &str| -> String {
+            store
+                .conn
+                .query_row("SELECT value FROM meta WHERE key = ?1", [key], |row| {
+                    row.get(0)
+                })
+                .unwrap()
+        };
+        let at = meta("indexed_at");
         assert_eq!(at.len(), "2026-10-20T06:00:00Z".len(), "{at}");
+        assert_eq!(meta("max_age_days"), "7");
         drop(store);
         fs::remove_dir_all(&dir).unwrap();
     }
```

> **Learn: building test fixtures with struct update syntax.** `ChunkRecord { kind: "step", number: Some("2"), ..chunk("step", (12, 13), text, None) }` takes every field not named from the value after `..`. The tests use it for `SearchQuery` too: `SearchQuery { limit: 1, ..query("validate") }` changes one filter and keeps the rest.

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test -p codecite-core`
Expected: compile errors such as:

```text
error[E0061]: this method takes 0 arguments but 1 argument was supplied   (set_indexed_at(7))
error[E0599]: no variant named `UnknownCorpus` found for enum `error::Error`
error[E0599]: no associated function or constant named `open_corpus` found for struct `store::Store` in the current scope
error[E0425]: cannot find function `is_corpus_name` in this scope
error[E0422]: cannot find struct, variant or union type `SearchQuery` in this scope
error[E0425]: cannot find function `fts_query` in this scope
error: could not compile `codecite-core` (lib test) due to 32 previous errors
```

- [ ] **Step 3: Add the errors, `open_corpus`, the stored age and the search**

```diff
diff --git a/crates/codecite-core/src/error.rs b/crates/codecite-core/src/error.rs
index 365ab1d..8b3f97a 100644
--- a/crates/codecite-core/src/error.rs
+++ b/crates/codecite-core/src/error.rs
@@ -37,6 +37,51 @@ pub enum Error {
         /// The newest schema version this build supports.
         supported: u32,
     },
+
+    /// A corpus name that isn't a plain file name.
+    #[error(
+        "invalid corpus name {name:?}: use ASCII letters, digits, `-`, `_` and `.`, not starting with `.`"
+    )]
+    InvalidCorpusName {
+        /// The name as given.
+        name: String,
+    },
+
+    /// No corpus file has this name.
+    #[error("unknown corpus `{corpus}`; {}", known_corpora(known))]
+    UnknownCorpus {
+        /// The name asked for.
+        corpus: String,
+        /// The corpora that do exist, sorted.
+        known: Vec<String>,
+    },
+
+    /// The corpus was last indexed longer ago than its manifest's
+    /// `max_age_days` allows.
+    #[error(
+        "corpus `{corpus}` was indexed at {indexed_at}, more than {max_age_days} days ago; re-run codecite index, or allow a stale index"
+    )]
+    StaleIndex {
+        /// The corpus's name.
+        corpus: String,
+        /// When it was last indexed.
+        indexed_at: String,
+        /// Its maximum age.
+        max_age_days: u32,
+    },
+
+    /// The query has no letter or digit to search for.
+    #[error("the query has no words to search for")]
+    EmptyQuery,
+}
+
+/// The end of an [`Error::UnknownCorpus`] message.
+fn known_corpora(known: &[String]) -> String {
+    if known.is_empty() {
+        "no corpus has been indexed yet; run codecite index".into()
+    } else {
+        format!("known corpora: {}", known.join(", "))
+    }
 }
 
 /// `Result` with [`Error`] as the default error type.
diff --git a/crates/codecite-core/src/lib.rs b/crates/codecite-core/src/lib.rs
index cab028b..e5a1eb8 100644
--- a/crates/codecite-core/src/lib.rs
+++ b/crates/codecite-core/src/lib.rs
@@ -18,7 +18,10 @@ mod keyword;
 pub use keyword::{Keyword, UnknownKeyword, requirement_keywords};
 
 mod store;
-pub use store::{ChunkRecord, DocumentRecord, SCHEMA_VERSION, SourceRecord, Store, SymbolRecord};
+pub use store::{
+    ChunkRecord, DEFAULT_MAX_AGE_DAYS, DocumentRecord, Freshness, MAX_SNIPPET_LINES,
+    SCHEMA_VERSION, SearchQuery, SearchResult, SourceRecord, Store, SymbolRecord, is_corpus_name,
+};
 
 mod terms;
 pub use terms::{identifier_terms, split_identifier, text_terms};
diff --git a/crates/codecite-core/src/store.rs b/crates/codecite-core/src/store.rs
index 7b674fa..cd56cfc 100644
--- a/crates/codecite-core/src/store.rs
+++ b/crates/codecite-core/src/store.rs
@@ -10,6 +10,7 @@ use rusqlite::{Connection, ErrorCode, OpenFlags, OptionalExtension, TransactionB
 use crate::{Error, Result};
 
 mod search;
+pub use search::{DEFAULT_MAX_AGE_DAYS, Freshness, MAX_SNIPPET_LINES, SearchQuery, SearchResult};
 
 mod write;
 pub use write::{ChunkRecord, DocumentRecord, SourceRecord, SymbolRecord};
@@ -148,6 +149,28 @@ impl Store {
         })
     }
 
+    /// Opens the corpus named `corpus` in `dir`, the file
+    /// `<dir>/<corpus>.db`, for queries.
+    ///
+    /// A name that isn't a plain file name is [`Error::InvalidCorpusName`],
+    /// and one with no file is [`Error::UnknownCorpus`], listing the corpora
+    /// `dir` does hold.
+    pub fn open_corpus(dir: &Path, corpus: &str) -> Result<Store> {
+        if !is_corpus_name(corpus) {
+            return Err(Error::InvalidCorpusName {
+                name: corpus.into(),
+            });
+        }
+        let path = dir.join(format!("{corpus}.db"));
+        if !path.exists() {
+            return Err(Error::UnknownCorpus {
+                corpus: corpus.into(),
+                known: known_corpora(dir)?,
+            });
+        }
+        Store::open_read_only(&path)
+    }
+
     /// The schema version stored in the file.
     pub fn schema_version(&self) -> Result<u32> {
         read_version(&self.conn, &self.path)
@@ -159,6 +182,36 @@ impl Store {
     }
 }
 
+/// Whether `name` can name a corpus. It becomes a file name, so it may
+/// hold only ASCII letters, digits, `-`, `_` and `.`, and may not start
+/// with `.`.
+pub fn is_corpus_name(name: &str) -> bool {
+    let allowed = |c: char| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.');
+    !name.is_empty() && !name.starts_with('.') && name.chars().all(allowed)
+}
+
+/// The corpora in `dir`: the names of its `.db` files, sorted. A missing
+/// directory holds none.
+fn known_corpora(dir: &Path) -> Result<Vec<String>> {
+    let entries = match std::fs::read_dir(dir) {
+        Ok(entries) => entries,
+        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
+        Err(e) => return Err(e.into()),
+    };
+    let mut names = Vec::new();
+    for entry in entries {
+        let path = entry?.path();
+        if path.extension().is_some_and(|ext| ext == "db")
+            && let Some(name) = path.file_stem().and_then(|s| s.to_str())
+            && is_corpus_name(name)
+        {
+            names.push(name.to_owned());
+        }
+    }
+    names.sort();
+    Ok(names)
+}
+
 /// Puts the file in WAL mode.
 ///
 /// The switch takes a read lock and then upgrades it to a write lock. SQLite
diff --git a/crates/codecite-core/src/store/search.rs b/crates/codecite-core/src/store/search.rs
index 8aee166..4d50ad9 100644
--- a/crates/codecite-core/src/store/search.rs
+++ b/crates/codecite-core/src/store/search.rs
@@ -1,6 +1,271 @@
 //! Searching a corpus: BM25 over the full-text index, with an exact symbol
 //! name first, and each result cited.
 
+use rusqlite::{OptionalExtension, params};
+use serde::Serialize;
+
+use super::Store;
+use crate::{Error, Result, text_terms};
+
+/// The most lines of a chunk a result's snippet holds. Agents read the
+/// rest through its citation.
+pub const MAX_SNIPPET_LINES: usize = 40;
+
+/// How many days an index stays fresh when its manifest doesn't say.
+pub const DEFAULT_MAX_AGE_DAYS: u32 = 30;
+
+/// The `bm25()` weight of each full-text column, in the table's order:
+/// a match in a symbol's name counts most, then one in the path, then one
+/// in the text.
+const COLUMN_WEIGHTS: (f64, f64, f64) = (10.0, 5.0, 1.0);
+
+/// A search: the text, the filters and how many results to return.
+#[derive(Debug, Clone, PartialEq, Eq)]
+pub struct SearchQuery<'a> {
+    /// What to search for: words, identifiers or both.
+    pub text: &'a str,
+    /// Only documents in this language, as the index names it (`rust`).
+    pub language: Option<&'a str>,
+    /// Only documents whose path matches this glob (SQLite `GLOB`: `*`
+    /// matches any run of characters, `/` included, and case matters).
+    pub path: Option<&'a str>,
+    /// The most results to return.
+    pub limit: u32,
+    /// Search an index older than its maximum age instead of refusing it.
+    pub allow_stale: bool,
+}
+
+/// One ranked result, in the shape the CLI prints as JSON and the MCP
+/// server returns.
+#[derive(Debug, Clone, PartialEq, Serialize)]
+pub struct SearchResult {
+    /// Where the chunk is: `repo@sha:path#L10-L42`, or
+    /// `doc@sha256:<12 hex>#section-4.3[/step-2]` for a spec.
+    pub citation: String,
+    /// `function`, `class`, `section`, `step` or `window`.
+    pub kind: String,
+    /// The name of the definition the chunk holds, if any.
+    #[serde(skip_serializing_if = "Option::is_none")]
+    pub symbol: Option<String>,
+    /// The BM25 score: higher is a better match.
+    pub score: f64,
+    /// How agents may use the source: `adapt`, `approach_only` or `quote`.
+    pub license_class: String,
+    /// When the chunk's source was last indexed.
+    pub indexed_at: String,
+    /// For code, whether the file differed from the cited commit when it
+    /// was indexed. Spec results have none.
+    #[serde(skip_serializing_if = "Option::is_none")]
+    pub dirty: Option<bool>,
+    /// The chunk's first [`MAX_SNIPPET_LINES`] lines.
+    pub snippet: String,
+}
+
+/// When a corpus was indexed, and whether that is too long ago.
+#[derive(Debug, Clone, PartialEq)]
+pub struct Freshness {
+    /// When the last index run finished.
+    pub indexed_at: String,
+    /// The manifest's `max_age_days` at that run.
+    pub max_age_days: u32,
+    /// Days since the run, fractional.
+    pub age_days: f64,
+}
+
+impl Freshness {
+    /// Whether the index is older than its maximum age.
+    pub fn is_stale(&self) -> bool {
+        self.age_days > f64::from(self.max_age_days)
+    }
+}
+
+impl Store {
+    /// When the corpus was last indexed, and its maximum age.
+    ///
+    /// A corpus whose first index run never finished has no `indexed_at`
+    /// and is an [`Error::InvalidCorpus`]. One indexed before the maximum
+    /// age was stored gets [`DEFAULT_MAX_AGE_DAYS`].
+    pub fn freshness(&self) -> Result<Freshness> {
+        let row: Option<(String, Option<String>, f64)> = self
+            .conn
+            .query_row(
+                "SELECT i.value, (SELECT value FROM meta WHERE key = 'max_age_days'),
+                   julianday('now') - julianday(i.value)
+                 FROM meta i WHERE i.key = 'indexed_at'",
+                [],
+                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
+            )
+            .optional()?;
+        let Some((indexed_at, max_age_days, age_days)) = row else {
+            return Err(Error::InvalidCorpus {
+                path: self.path.clone(),
+                reason: "it was never fully indexed; run codecite index".into(),
+            });
+        };
+        let max_age_days = match max_age_days {
+            None => DEFAULT_MAX_AGE_DAYS,
+            Some(days) => days.parse().map_err(|_| Error::InvalidCorpus {
+                path: self.path.clone(),
+                reason: format!("unreadable max_age_days {days:?}"),
+            })?,
+        };
+        Ok(Freshness {
+            indexed_at,
+            max_age_days,
+            age_days,
+        })
+    }
+
+    /// Searches the corpus and returns at most `query.limit` results, best
+    /// first.
+    ///
+    /// Each word of the query is split into identifier terms as the index
+    /// splits text, and a chunk matches when it holds any of them; BM25
+    /// ranks the matches, weighting the symbol column, then the path, then
+    /// the text. A chunk whose symbol's name is exactly the query comes
+    /// first. Only the current version of each document is searched.
+    ///
+    /// A query with no letter or digit is [`Error::EmptyQuery`]. An index
+    /// older than its maximum age is [`Error::StaleIndex`] unless
+    /// `query.allow_stale` is set.
+    pub fn search(&self, query: &SearchQuery<'_>) -> Result<Vec<SearchResult>> {
+        let fts = fts_query(query.text).ok_or(Error::EmptyQuery)?;
+        let freshness = self.freshness()?;
+        if freshness.is_stale() && !query.allow_stale {
+            return Err(Error::StaleIndex {
+                corpus: self.corpus_name(),
+                indexed_at: freshness.indexed_at,
+                max_age_days: freshness.max_age_days,
+            });
+        }
+
+        let (w_symbol, w_path, w_text) = COLUMN_WEIGHTS;
+        let mut stmt = self.conn.prepare_cached(
+            "SELECT c.kind, c.start_line, c.end_line, c.number, p.number, c.text,
+               d.path, d.dirty, s.name, s.kind, s.revision, s.license_class, s.indexed_at,
+               sym.display_name,
+               -bm25(chunk_fts, ?3, ?4, ?5) AS score,
+               coalesce(sym.display_name = ?2, 0) AS exact
+             FROM chunk_fts
+             JOIN chunk c ON c.id = chunk_fts.rowid
+             JOIN document d ON d.id = c.document_id
+             JOIN source s ON s.id = d.source_id
+             LEFT JOIN chunk p ON p.id = c.parent_id
+             LEFT JOIN symbol sym ON sym.id = c.symbol_id
+             WHERE chunk_fts MATCH ?1 AND s.is_current
+               AND (?6 IS NULL OR d.language = ?6)
+               AND (?7 IS NULL OR d.path GLOB ?7)
+             ORDER BY exact DESC, score DESC, c.id
+             LIMIT ?8",
+        )?;
+        let rows = stmt.query_map(
+            params![
+                fts,
+                query.text.trim(),
+                w_symbol,
+                w_path,
+                w_text,
+                query.language,
+                query.path,
+                query.limit,
+            ],
+            |row| {
+                let kind: String = row.get(0)?;
+                let start: u32 = row.get(1)?;
+                let end: u32 = row.get(2)?;
+                let number: Option<String> = row.get(3)?;
+                let parent_number: Option<String> = row.get(4)?;
+                let text: String = row.get(5)?;
+                let path: String = row.get(6)?;
+                let dirty: bool = row.get(7)?;
+                let source: String = row.get(8)?;
+                let source_kind: String = row.get(9)?;
+                let revision: String = row.get(10)?;
+                let is_spec = source_kind == "document";
+                let citation = if is_spec {
+                    spec_citation(&source, &revision, &kind, number, parent_number)
+                } else {
+                    code_citation(&source, &revision, &path, start, end)
+                };
+                Ok(SearchResult {
+                    citation,
+                    kind,
+                    symbol: row.get(13)?,
+                    score: row.get(14)?,
+                    license_class: row.get(11)?,
+                    indexed_at: row.get(12)?,
+                    dirty: (!is_spec).then_some(dirty),
+                    snippet: snippet(&text),
+                })
+            },
+        )?;
+        Ok(rows.collect::<rusqlite::Result<_>>()?)
+    }
+
+    /// The corpus's name: its file's name without `.db`.
+    fn corpus_name(&self) -> String {
+        self.path
+            .file_stem()
+            .map_or_else(String::new, |s| s.to_string_lossy().into_owned())
+    }
+}
+
+/// Turns the user's text into an FTS5 query, or `None` when it has no
+/// terms.
+///
+/// Each term is quoted, so FTS5 reads it as a plain string and never as
+/// syntax (`AND`, `NEAR`, `col:`, `*`), and the terms are joined with `OR`.
+fn fts_query(text: &str) -> Option<String> {
+    // The tokenizer folds case, so `NEAR` and `near` are one term.
+    let terms = text_terms(text).to_lowercase();
+    let mut seen: Vec<&str> = Vec::new();
+    for term in terms.split(' ').filter(|t| !t.is_empty()) {
+        if !seen.contains(&term) {
+            seen.push(term);
+        }
+    }
+    if seen.is_empty() {
+        return None;
+    }
+    let quoted: Vec<String> = seen
+        .iter()
+        .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
+        .collect();
+    Some(quoted.join(" OR "))
+}
+
+/// `repo@sha:path#L10-L42`, or `#L10` for one line.
+fn code_citation(source: &str, revision: &str, path: &str, start: u32, end: u32) -> String {
+    if start == end {
+        format!("{source}@{revision}:{path}#L{start}")
+    } else {
+        format!("{source}@{revision}:{path}#L{start}-L{end}")
+    }
+}
+
+/// `doc@sha256:<12 hex>#section-<n>`, or `…#section-<n>/step-<k>` for a step,
+/// whose section is its parent's number.
+fn spec_citation(
+    doc: &str,
+    sha256: &str,
+    kind: &str,
+    number: Option<String>,
+    parent_number: Option<String>,
+) -> String {
+    let hash = sha256.get(..12).unwrap_or(sha256);
+    let number = number.unwrap_or_default();
+    match (kind, parent_number) {
+        ("step", Some(section)) => format!("{doc}@sha256:{hash}#section-{section}/step-{number}"),
+        _ => format!("{doc}@sha256:{hash}#section-{number}"),
+    }
+}
+
+/// The first [`MAX_SNIPPET_LINES`] lines of a chunk's text.
+fn snippet(text: &str) -> String {
+    let lines: Vec<&str> = text.lines().take(MAX_SNIPPET_LINES).collect();
+    lines.join("\n")
+}
+
 #[cfg(test)]
 mod tests {
     use super::*;
diff --git a/crates/codecite-core/src/store/write.rs b/crates/codecite-core/src/store/write.rs
index 4719e54..8d3d860 100644
--- a/crates/codecite-core/src/store/write.rs
+++ b/crates/codecite-core/src/store/write.rs
@@ -219,13 +219,16 @@ impl Store {
         Ok(sources.into_iter().map(|(_, label)| label).collect())
     }
 
-    /// Records when the corpus was last indexed, in `meta.indexed_at`.
-    pub fn set_indexed_at(&mut self) -> Result<()> {
+    /// Records when the corpus was last indexed, in `meta.indexed_at`, and
+    /// the manifest's `max_age_days`, so queries can refuse a stale index
+    /// without reading the manifest.
+    pub fn set_indexed_at(&mut self, max_age_days: u32) -> Result<()> {
         self.conn.execute(
             "INSERT INTO meta (key, value)
-             VALUES ('indexed_at', strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
+             VALUES ('indexed_at', strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
+                    ('max_age_days', ?1)
              ON CONFLICT (key) DO UPDATE SET value = excluded.value",
-            [],
+            [max_age_days.to_string()],
         )?;
         Ok(())
     }
diff --git a/crates/codecite-index/src/index.rs b/crates/codecite-index/src/index.rs
index db43c7a..cc1b1af 100644
--- a/crates/codecite-index/src/index.rs
+++ b/crates/codecite-index/src/index.rs
@@ -131,7 +131,7 @@ pub fn index_corpus(
         keep.push(index_document(store, doc, bytes, &mut summary)?);
     }
     summary.removed_sources = store.delete_sources_except(&keep)?;
-    store.set_indexed_at()?;
+    store.set_indexed_at(manifest.max_age_days)?;
     summary.elapsed = started.elapsed();
     Ok(summary)
 }
diff --git a/crates/codecite-index/src/manifest.rs b/crates/codecite-index/src/manifest.rs
index 6046e32..d9395cc 100644
--- a/crates/codecite-index/src/manifest.rs
+++ b/crates/codecite-index/src/manifest.rs
@@ -222,7 +222,7 @@ struct RawManifest {
 }
 
 fn default_max_age_days() -> u32 {
-    30
+    codecite_core::DEFAULT_MAX_AGE_DAYS
 }
 
 impl Manifest {
```

> **Learn: FTS5 query syntax and escaping user input.** The right side of `MATCH` is a small query language of its own: `a b` means both, `a OR b`, `NEAR(a b)`, `col:term` restricts to a column, `term*` is a prefix, and `"…"` is a phrase. Passing user text straight in makes `token AND` a syntax error and `path:secret` a column filter the user never meant. Binding it as a parameter (`?1`) doesn't help, since the parameter *is* the query. Every term is therefore wrapped in double quotes, with any `"` inside doubled, which is the only escape FTS5 strings have. A quoted term is always a plain string (a one-word phrase), and `fts_query` joins them with `OR` itself. `bm25()` returns lower-is-better values (negative), which is why the score is negated. Its arguments after the table are one weight per column, in the order the table declares them.

> **Learn: `serde::Serialize` and optional fields.** `#[derive(Serialize)]` writes a struct as a JSON object whose keys are the field names, in declaration order. `#[serde(skip_serializing_if = "Option::is_none")]` leaves a `None` field out entirely, instead of writing `null`. That is how a spec result has no `dirty` and a chunk with no symbol has no `symbol`. Core depends only on `serde`, and the format (`serde_json`) is the binary's choice. `serde_json::Value`'s map sorts its keys, which is why the shape test compares sorted keys.

> **Learn: `bool::then_some` and `let … else`.** `(!is_spec).then_some(dirty)` is `Some(dirty)` for code and `None` for a spec. `let Some(x) = row else { return Err(…) };` binds on a match and must diverge otherwise. `freshness` uses it to turn a missing `indexed_at` row into an error without nesting.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p codecite-core && cargo test -p codecite-index --lib manifest`
Expected: all pass (core: 109 passed).

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock crates/codecite-core crates/codecite-index/src/index.rs crates/codecite-index/src/manifest.rs
git commit -m "feat(core): search a corpus with ranked, cited results (#26)" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: `codecite search` (`codecite` binary)

**Files:**
- Create: `crates/codecite/tests/search.rs`
- Modify: `crates/codecite/src/main.rs`, `crates/codecite/Cargo.toml`

**Interfaces:**
- Consumes: Task 1's `Store::open_corpus`, `Store::search`, `SearchQuery`, `SearchResult`, `is_corpus_name` and `Error::InvalidCorpusName`.

- [ ] **Step 1: Write the failing tests**

The unit tests cover parsing and the human form, and `tests/search.rs` covers the binary end to end. `Cargo.toml` gains `serde_json` and the `rusqlite` dev-dependency, which the stale test uses to backdate `indexed_at`:

```diff
diff --git a/crates/codecite/Cargo.toml b/crates/codecite/Cargo.toml
index 0e01a31..e766f23 100644
--- a/crates/codecite/Cargo.toml
+++ b/crates/codecite/Cargo.toml
@@ -18,6 +18,8 @@ codecite-core.workspace = true
 codecite-index.workspace = true
 codecite-lsp.workspace = true
 codecite-mcp.workspace = true
+serde_json.workspace = true
 
 [dev-dependencies]
+rusqlite.workspace = true
 tempfile.workspace = true
diff --git a/crates/codecite/src/main.rs b/crates/codecite/src/main.rs
index 126cf35..7e0a1e1 100644
--- a/crates/codecite/src/main.rs
+++ b/crates/codecite/src/main.rs
@@ -138,13 +138,13 @@ mod tests {
     }
 
     #[test]
-    fn corpus_names_are_plain_file_names() {
-        for good in ["identity", "oidc-impls", "rfc_9449", "v1.2"] {
-            assert!(check_corpus_name(good).is_ok(), "{good}");
-        }
-        for bad in ["", ".hidden", "..", "../x", "a/b", "a\\b", "名前", "a b"] {
-            assert!(check_corpus_name(bad).is_err(), "{bad:?}");
-        }
+    fn a_bad_corpus_name_is_a_typed_error() {
+        assert!(check_corpus_name("identity").is_ok());
+        let err = check_corpus_name("../x").unwrap_err();
+        assert!(matches!(
+            err.downcast_ref(),
+            Some(codecite_core::Error::InvalidCorpusName { .. })
+        ));
     }
 
     #[test]
@@ -156,12 +156,98 @@ mod tests {
             corpus,
             manifest,
             print_hashes,
-        } = cli.command;
+        } = cli.command
+        else {
+            panic!("not index");
+        };
         assert_eq!(corpus, "identity");
         assert_eq!(manifest, Some(PathBuf::from("m.toml")));
         assert!(!print_hashes);
         let cli = Cli::try_parse_from(["codecite", "index", "identity", "--print-hashes"]).unwrap();
-        let Command::Index { print_hashes, .. } = cli.command;
+        let Command::Index { print_hashes, .. } = cli.command else {
+            panic!("not index");
+        };
         assert!(print_hashes);
     }
+
+    #[test]
+    fn search_takes_a_corpus_a_query_and_filters() {
+        assert!(Cli::try_parse_from(["codecite", "search", "identity"]).is_err());
+        let cli =
+            Cli::try_parse_from(["codecite", "search", "identity", "validate token"]).unwrap();
+        let Command::Search {
+            corpus,
+            query,
+            language,
+            path,
+            limit,
+            json,
+            allow_stale,
+        } = cli.command
+        else {
+            panic!("not search");
+        };
+        assert_eq!(
+            (corpus.as_str(), query.as_str()),
+            ("identity", "validate token")
+        );
+        assert_eq!((language, path, limit), (None, None, 10));
+        assert!(!json && !allow_stale);
+        let cli = Cli::try_parse_from([
+            "codecite",
+            "search",
+            "identity",
+            "jti",
+            "--language",
+            "rust",
+            "--path",
+            "src/*",
+            "--limit",
+            "3",
+            "--json",
+            "--allow-stale",
+        ])
+        .unwrap();
+        let Command::Search {
+            language,
+            path,
+            limit,
+            json,
+            allow_stale,
+            ..
+        } = cli.command
+        else {
+            panic!("not search");
+        };
+        assert_eq!(language.as_deref(), Some("rust"));
+        assert_eq!(path.as_deref(), Some("src/*"));
+        assert_eq!(limit, 3);
+        assert!(json && allow_stale);
+        let zero = ["codecite", "search", "identity", "jti", "--limit", "0"];
+        assert!(Cli::try_parse_from(zero).is_err());
+    }
+
+    #[test]
+    fn the_human_form_shows_rank_citation_and_snippet() {
+        let result = SearchResult {
+            citation: "app@0123456:src/lib.rs#L1-L5".into(),
+            kind: "function".into(),
+            symbol: Some("validate".into()),
+            score: 2.5,
+            license_class: "adapt".into(),
+            indexed_at: "2026-10-20T06:00:00Z".into(),
+            dirty: Some(true),
+            snippet: "fn validate() {\n    a();\n    b();\n    c();\n}".into(),
+        };
+        assert_eq!(
+            human(&[result]),
+            "1. app@0123456:src/lib.rs#L1-L5
+   function validate · score 2.50 · adapt · indexed 2026-10-20T06:00:00Z · dirty
+     fn validate() {
+         a();
+         b();
+"
+        );
+        assert_eq!(human(&[]), "no results\n");
+    }
 }
diff --git a/crates/codecite/tests/search.rs b/crates/codecite/tests/search.rs
new file mode 100644
index 0000000..5809a6f
--- /dev/null
+++ b/crates/codecite/tests/search.rs
@@ -0,0 +1,202 @@
+//! `codecite search` end to end: a corpus indexed from a real git checkout
+//! under a temporary `XDG_DATA_HOME`, then searched. Linux only, since other
+//! platforms ignore `XDG_DATA_HOME`.
+#![cfg(target_os = "linux")]
+
+use std::fs;
+use std::path::{Path, PathBuf};
+use std::process::{Command, Output};
+
+fn git(dir: &Path, args: &[&str]) {
+    let status = Command::new("git")
+        .arg("-C")
+        .arg(dir)
+        .args([
+            "-c",
+            "user.name=codecite",
+            "-c",
+            "user.email=codecite@example.com",
+        ])
+        .args(["-c", "commit.gpgsign=false"])
+        .args(args)
+        .env_remove("GIT_DIR")
+        .env_remove("GIT_WORK_TREE")
+        .env_remove("GIT_INDEX_FILE")
+        .status()
+        .unwrap();
+    assert!(status.success(), "git {args:?}");
+}
+
+/// A corpus `demo` indexed from a two-file checkout, one file dirty.
+struct Demo {
+    tmp: tempfile::TempDir,
+}
+
+impl Demo {
+    fn new() -> Self {
+        let tmp = tempfile::tempdir().unwrap();
+        let repo = tmp.path().join("app");
+        fs::create_dir_all(repo.join("src")).unwrap();
+        fs::create_dir_all(repo.join("web")).unwrap();
+        git(&repo, &["init", "-q"]);
+        fs::write(
+            repo.join("src/lib.rs"),
+            "pub fn validate(token: &str) -> bool {\n    !token.is_empty()\n}\n",
+        )
+        .unwrap();
+        fs::write(
+            repo.join("web/check.ts"),
+            "export function checkToken(token) {\n  return validate(token);\n}\n",
+        )
+        .unwrap();
+        git(&repo, &["add", "."]);
+        git(&repo, &["commit", "-q", "-m", "init"]);
+        // An uncommitted edit: results from this file carry `dirty`.
+        fs::write(
+            repo.join("src/lib.rs"),
+            "pub fn validate(token: &str) -> bool {\n    token.len() > 8\n}\n",
+        )
+        .unwrap();
+
+        let work = tmp.path().join("work");
+        fs::create_dir_all(work.join("corpora")).unwrap();
+        fs::write(
+            work.join("corpora/demo.toml"),
+            format!(
+                "name = \"demo\"\nmax_age_days = 7\n\n[[source]]\nkind = \"local\"\npath = {:?}\nuse = \"adapt\"\n",
+                repo.to_str().unwrap()
+            ),
+        )
+        .unwrap();
+        let demo = Demo { tmp };
+        let indexed = demo.run(&["index", "demo"]);
+        assert!(indexed.status.success(), "{}", stderr(&indexed));
+        demo
+    }
+
+    /// Runs the binary with its data directory in the scratch directory.
+    fn run(&self, args: &[&str]) -> Output {
+        Command::new(env!("CARGO_BIN_EXE_codecite"))
+            .current_dir(self.tmp.path().join("work"))
+            .env("HOME", self.tmp.path())
+            .env("XDG_DATA_HOME", self.tmp.path().join("data"))
+            .env("XDG_CACHE_HOME", self.tmp.path().join("cache"))
+            .args(args)
+            .output()
+            .unwrap()
+    }
+
+    /// Runs `codecite search demo <args> --json` and parses its output.
+    fn json(&self, args: &[&str]) -> Vec<serde_json::Value> {
+        let mut all = vec!["search", "demo"];
+        all.extend(args);
+        all.push("--json");
+        let output = self.run(&all);
+        assert!(output.status.success(), "{}", stderr(&output));
+        serde_json::from_slice(&output.stdout).unwrap()
+    }
+
+    fn db(&self) -> PathBuf {
+        self.tmp.path().join("data/codecite/demo.db")
+    }
+}
+
+fn stderr(output: &Output) -> String {
+    String::from_utf8_lossy(&output.stderr).into_owned()
+}
+
+fn citations(results: &[serde_json::Value]) -> Vec<&str> {
+    results
+        .iter()
+        .map(|r| r["citation"].as_str().unwrap())
+        .collect()
+}
+
+#[test]
+fn search_prints_cited_results_as_json() {
+    let demo = Demo::new();
+    let results = demo.json(&["validate"]);
+    assert_eq!(results.len(), 2, "{results:#?}");
+    let top = &results[0];
+    assert_eq!(top["symbol"], "validate");
+    assert_eq!(top["kind"], "function");
+    assert_eq!(top["license_class"], "adapt");
+    assert_eq!(top["dirty"], true);
+    assert!(top["indexed_at"].as_str().unwrap().ends_with('Z'));
+    assert!(top["snippet"].as_str().unwrap().contains("token.len() > 8"));
+    let citation = top["citation"].as_str().unwrap();
+    assert!(citation.starts_with("app@"), "{citation}");
+    assert!(citation.ends_with(":src/lib.rs#L1-L3"), "{citation}");
+    assert_eq!(results[1]["dirty"], false);
+}
+
+#[test]
+fn filters_narrow_the_results() {
+    let demo = Demo::new();
+    let ts = demo.json(&["validate", "--language", "typescript"]);
+    assert_eq!(citations(&ts).len(), 1);
+    assert!(citations(&ts)[0].contains(":web/check.ts#"));
+    let src = demo.json(&["validate", "--path", "src/*"]);
+    assert_eq!(citations(&src).len(), 1);
+    assert!(citations(&src)[0].contains(":src/lib.rs#"));
+    assert_eq!(demo.json(&["validate", "--limit", "1"]).len(), 1);
+    assert!(demo.json(&["nothing-matches-this"]).is_empty());
+}
+
+#[test]
+fn the_human_form_is_short() {
+    let demo = Demo::new();
+    let output = demo.run(&["search", "demo", "check token"]);
+    assert!(output.status.success(), "{}", stderr(&output));
+    let text = String::from_utf8(output.stdout).unwrap();
+    let lines: Vec<&str> = text.lines().collect();
+    assert!(lines[0].starts_with("1. app@"), "{text}");
+    assert!(lines[0].ends_with(":web/check.ts#L1-L3"), "{text}");
+    assert!(lines[1].contains("function checkToken · score "), "{text}");
+    assert_eq!(lines[2].trim(), "export function checkToken(token) {");
+}
+
+#[test]
+fn a_stale_index_is_refused_unless_allowed() {
+    let demo = Demo::new();
+    rusqlite::Connection::open(demo.db())
+        .unwrap()
+        .execute(
+            "UPDATE meta SET value = '2026-01-01T00:00:00Z' WHERE key = 'indexed_at'",
+            [],
+        )
+        .unwrap();
+    let output = demo.run(&["search", "demo", "validate"]);
+    assert!(!output.status.success());
+    let err = stderr(&output);
+    assert!(
+        err.contains("corpus `demo` was indexed at 2026-01-01T00:00:00Z, more than 7 days ago"),
+        "{err}"
+    );
+    assert_eq!(demo.json(&["validate", "--allow-stale"]).len(), 2);
+}
+
+#[test]
+fn errors_name_the_problem() {
+    let demo = Demo::new();
+    let cases = [
+        (
+            vec!["search", "dmeo", "validate"],
+            "unknown corpus `dmeo`; known corpora: demo",
+        ),
+        (vec!["search", "../demo", "x"], "invalid corpus name"),
+        (
+            vec!["search", "demo", "  !! "],
+            "the query has no words to search for",
+        ),
+    ];
+    for (args, want) in cases {
+        let output = demo.run(&args);
+        assert!(!output.status.success(), "{args:?}");
+        assert!(
+            stderr(&output).contains(want),
+            "{args:?}: {}",
+            stderr(&output)
+        );
+    }
+}
```

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test -p codecite`
Expected: compile errors such as:

```text
error: could not compile `codecite` (bin "codecite" test) due to 5 previous errors; 2 warnings emitted
error[E0422]: cannot find struct, variant or union type `SearchResult` in this scope
error[E0425]: cannot find function `human` in this scope
error[E0599]: no variant named `Search` found for enum `Command`
```

- [ ] **Step 3: Add the subcommand**

```diff
diff --git a/crates/codecite/src/main.rs b/crates/codecite/src/main.rs
index 7e0a1e1..cbdb199 100644
--- a/crates/codecite/src/main.rs
+++ b/crates/codecite/src/main.rs
@@ -5,7 +5,7 @@ use std::path::{Path, PathBuf};
 
 use anyhow::{Context as _, bail};
 use clap::{Parser, Subcommand};
-use codecite_core::Store;
+use codecite_core::{SearchQuery, SearchResult, Store, is_corpus_name};
 use codecite_index::{Fetcher, LicenseClass, Manifest, Source, WalkOptions, index_corpus};
 
 /// Command-line arguments.
@@ -31,6 +31,28 @@ enum Command {
         #[arg(long)]
         print_hashes: bool,
     },
+    /// Search a corpus and print ranked, cited results.
+    Search {
+        /// The corpus to search.
+        corpus: String,
+        /// What to search for: words, identifiers or both.
+        query: String,
+        /// Only files in this language, such as `rust` or `markdown`.
+        #[arg(long)]
+        language: Option<String>,
+        /// Only files whose path matches this glob; `*` matches `/` too.
+        #[arg(long)]
+        path: Option<String>,
+        /// The most results to print.
+        #[arg(long, default_value_t = 10, value_parser = clap::value_parser!(u32).range(1..))]
+        limit: u32,
+        /// Print the results as a JSON array.
+        #[arg(long)]
+        json: bool,
+        /// Search an index older than its manifest's `max_age_days`.
+        #[arg(long)]
+        allow_stale: bool,
+    },
 }
 
 fn main() -> anyhow::Result<()> {
@@ -40,6 +62,24 @@ fn main() -> anyhow::Result<()> {
             manifest,
             print_hashes,
         } => index(&corpus, manifest, print_hashes),
+        Command::Search {
+            corpus,
+            query,
+            language,
+            path,
+            limit,
+            json,
+            allow_stale,
+        } => {
+            let query = SearchQuery {
+                text: &query,
+                language: language.as_deref(),
+                path: path.as_deref(),
+                limit,
+                allow_stale,
+            };
+            search(&corpus, &query, json)
+        }
     }
 }
 
@@ -82,15 +122,64 @@ fn index(corpus: &str, manifest: Option<PathBuf>, print_hashes: bool) -> anyhow:
 /// A corpus name becomes a file name, so it may hold only ASCII letters,
 /// digits, `-`, `_` and `.`, and may not start with `.`.
 fn check_corpus_name(corpus: &str) -> anyhow::Result<()> {
-    let allowed = |c: char| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.');
-    if corpus.is_empty() || corpus.starts_with('.') || !corpus.chars().all(allowed) {
-        bail!(
-            "invalid corpus name {corpus:?}: use ASCII letters, digits, `-`, `_` and `.`, not starting with `.`"
-        );
+    if !is_corpus_name(corpus) {
+        return Err(codecite_core::Error::InvalidCorpusName {
+            name: corpus.into(),
+        }
+        .into());
+    }
+    Ok(())
+}
+
+/// `codecite search`: opens the corpus read-only and prints its results,
+/// as JSON or in a short human form.
+fn search(corpus: &str, query: &SearchQuery<'_>, json: bool) -> anyhow::Result<()> {
+    let dirs = project_dirs()?;
+    let store = Store::open_corpus(dirs.data_dir(), corpus)?;
+    let results = store.search(query)?;
+    if json {
+        println!("{}", serde_json::to_string_pretty(&results)?);
+    } else {
+        print!("{}", human(&results));
     }
     Ok(())
 }
 
+/// How many lines of each snippet the human form shows.
+const HUMAN_SNIPPET_LINES: usize = 3;
+
+/// The short human form of the results: per result, its rank and citation,
+/// then what it is, then the start of its snippet.
+fn human(results: &[SearchResult]) -> String {
+    use std::fmt::Write as _;
+
+    if results.is_empty() {
+        return "no results\n".into();
+    }
+    let mut out = String::new();
+    for (rank, r) in (1..).zip(results) {
+        // `write!` to a `String` can't fail.
+        let _ = writeln!(out, "{rank}. {}", r.citation);
+        let mut about = r.kind.clone();
+        if let Some(symbol) = &r.symbol {
+            let _ = write!(about, " {symbol}");
+        }
+        let _ = write!(
+            about,
+            " · score {:.2} · {} · indexed {}",
+            r.score, r.license_class, r.indexed_at
+        );
+        if r.dirty == Some(true) {
+            about.push_str(" · dirty");
+        }
+        let _ = writeln!(out, "   {about}");
+        for line in r.snippet.lines().take(HUMAN_SNIPPET_LINES) {
+            let _ = writeln!(out, "     {line}");
+        }
+    }
+    out
+}
+
 /// `codecite index --print-hashes`: downloads each document source that is
 /// not `excluded` and prints `<sha256>  <name>  <url>`, plus `(pinned)` when
 /// the hash is the one the manifest pins, so the owner can pin a new or
```

> **Learn: `serde_json` output.** `serde_json::to_string_pretty(&results)` serializes a `Vec<SearchResult>` as a JSON array with two-space indentation; `to_string` would put it on one line. It returns a `Result`, because a type's `Serialize` impl may fail (a map with non-string keys, say), so `?` passes that on. `print!` (not `println!`) writes the human form, since `human` already ends every line with `\n`.

> **Learn: clap's value parsers.** `#[arg(long, default_value_t = 10, value_parser = clap::value_parser!(u32).range(1..))]` makes clap parse `--limit` as a `u32` and reject anything below 1 with a usage error (exit 2) before `main` runs, so the search never sees a zero limit.

- [ ] **Step 4: Run the gates**

Run: `cargo fmt --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked && cargo deny check`
Expected: all green; 245 passed, 6 ignored; `advisories ok, bans ok, licenses ok, sources ok`.

- [ ] **Step 5: Exercise the binary by hand**

Run the scenarios in Step 6, and also check that `codecite search --help` lists every flag.

- [ ] **Step 6: Search a real corpus through the CLI**

`/var/tmp/search-proto/real-run.sh <worktree>` serves the seven real spec pages (from `/var/tmp/steps-proto/docs`) on `127.0.0.1:8766`. It indexes them, together with the worktree itself as a local source, into a scratch `XDG_DATA_HOME`. Then it runs exact-symbol, spec, JSON, filtered and FTS-syntax searches, an empty query, `--limit 0`, a misspelled corpus, and a backdated index with and without `--allow-stale`. The prototype's run, with long lines cut at 160 characters:

```text
skipped: 1 ignored
elapsed: 1.26s
(exit 0)
$ codecite search real upsert_source --limit 3
1. codecite@a1f9a6539bb09cc1bc34b53fc186aeb00b66c819:crates/codecite-core/src/store/write.rs#L97-L131
   function upsert_source · score 23.04 · adapt · indexed 2026-10-20T18:47:24Z · dirty
         pub fn upsert_source(&mut self, source: &SourceRecord<'_>) -> Result<i64> {
             // An upsert names the unique index it conflicts on, and a partial
             // index is named by its columns plus its `WHERE`.
2. codecite@a1f9a6539bb09cc1bc34b53fc186aeb00b66c819:crates/codecite-core/src/store/write.rs#L410-L426
   function upsert_source_keeps_the_id · score 23.81 · adapt · indexed 2026-10-20T18:47:24Z · dirty
         fn upsert_source_keeps_the_id() {
             let (mut store, dir) = store("upsert");
             let id = store.upsert_source(&source("app")).unwrap();
3. codecite@a1f9a6539bb09cc1bc34b53fc186aeb00b66c819:crates/codecite-core/src/store/write.rs#L599-L644
   function each_document_version_is_its_own_source · score 21.42 · adapt · indexed 2026-10-20T18:47:24Z · dirty
         fn each_document_version_is_its_own_source() {
             let (mut store, dir) = store("versions");
             let old = "1".repeat(64);
(exit 0)
$ codecite search real DPoP proof jti replay --limit 3
1. rfc9449@sha256:2d8657f07291#section-11.1
   section · score 41.94 · quote · indexed 2026-10-20T18:47:25Z
     11.1. DPoP Proof Replay

     If an adversary is able to get hold of a DPoP proof JWT, the adversary could replay that token at the same endpoint (the HTTP endpoint and method are enfor
2. rfc9449@sha256:2d8657f07291#section-4.2
   section · score 33.72 · quote · indexed 2026-10-20T18:47:25Z
     4.2. DPoP Proof JWT Syntax

     A DPoP proof is a JWT [RFC7519] that is signed (using JSON Web Signature (JWS) [RFC7515]) with a private key chosen by the client (see below). The JOSE Hea
3. rfc9449@sha256:2d8657f07291#section-7.3
   section · score 30.76 · quote · indexed 2026-10-20T18:47:25Z
     7.3. Client Considerations

     Authorization including a DPoP proof may not be idempotent (depending on server enforcement of jti, iat, and nonce claims). Consequently, all previously id
(exit 0)
$ codecite search real ID Token validation nonce --limit 2 --json
[
  {
    "citation": "openid-connect-core-1_0@sha256:4d016752a645#section-3.2.2.11",
    "kind": "section",
    "score": 15.166682739141311,
    "license_class": "quote",
    "indexed_at": "2026-10-20T18:47:25Z",
    "snippet": "3.2.2.11. ID Token Validation\n\nWhen using the Implicit Flow, the contents of the ID Token MUST be validated in the same manner as for the Auth
  },
  {
    "citation": "codecite@a1f9a6539bb09cc1bc34b53fc186aeb00b66c819:crates/codecite-index/src/spec.rs#L869-L891",
    "kind": "function",
    "symbol": "real_openid_connect_backchannel_logout",
    "score": 12.377789069432586,
    "license_class": "adapt",
    "indexed_at": "2026-10-20T18:47:24Z",
    "dirty": false,
    "snippet": "    fn real_openid_connect_backchannel_logout() {\n        let html = fetch_real(\n            \"openid-connect-backchannel-1_0\",\n
  }
]
(exit 0)
$ codecite search real foreign keys --language rust --path crates/codecite-core/* --limit 3
1. codecite@a1f9a6539bb09cc1bc34b53fc186aeb00b66c819:crates/codecite-core/src/store.rs#L686-L705
   function foreign_keys_are_enforced · score 17.89 · adapt · indexed 2026-10-20T18:47:24Z · dirty
         fn foreign_keys_are_enforced() {
             let dir = scratch("fk");
             let store = Store::open_read_write(&dir.join("corpus.db")).unwrap();
2. codecite@a1f9a6539bb09cc1bc34b53fc186aeb00b66c819:crates/codecite-core/src/store.rs#L45-L107
   function open_read_write · score 13.61 · adapt · indexed 2026-10-20T18:47:24Z · dirty
         pub fn open_read_write(path: &Path) -> Result<Store> {
             let mut conn = Connection::open(path)?;
             conn.busy_timeout(BUSY_TIMEOUT)?;
3. codecite@a1f9a6539bb09cc1bc34b53fc186aeb00b66c819:crates/codecite-core/src/store.rs#L661-L683
   function a_migration_that_leaves_orphans_is_refused_unchanged · score 11.93 · adapt · indexed 2026-10-20T18:47:24Z · dirty
         fn a_migration_that_leaves_orphans_is_refused_unchanged() {
             let dir = scratch("orphans");
             let path = dir.join("corpus.db");
(exit 0)
$ codecite search real NEAR("token" OR --limit 1
1. codecite@a1f9a6539bb09cc1bc34b53fc186aeb00b66c819:crates/codecite-core/src/store/search.rs#L603-L618
   function query_syntax_is_searched_as_words · score 12.32 · adapt · indexed 2026-10-20T18:47:24Z · dirty
         fn query_syntax_is_searched_as_words() {
             let f = fixture();
             for text in [
(exit 0)
$ codecite search real   ¶ --  --limit 1
Error: the query has no words to search for
(exit 1)
$ codecite search real x --limit 0
error: invalid value '0' for '--limit <LIMIT>': 0 is not in 1..=4294967295

For more information, try '--help'.
(exit 2)
$ codecite search raal token
Error: unknown corpus `raal`; known corpora: real
(exit 1)
$ codecite search real token --limit 1
Error: corpus `real` was indexed at 2026-09-01T00:00:00Z, more than 7 days ago; re-run codecite index, or allow a stale index
(exit 1)
$ codecite search real token --limit 1 --allow-stale
1. openid-connect-core-1_0@sha256:4d016752a645#section-3.3.3.6
   section · score 2.91 · quote · indexed 2026-10-20T18:47:25Z
     3.3.3.6. ID Token

     When using the Hybrid Flow, the contents of an ID Token returned from the Token Endpoint are the same as for an ID Token returned from the Authorization En
(exit 0)
```

- [ ] **Step 7: Commit**

```bash
git add crates/codecite/src/main.rs crates/codecite/tests/search.rs crates/codecite/Cargo.toml Cargo.lock
git commit -m "feat: search a corpus with codecite search (#26)" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: Describe search in the design

**Files:**
- Modify: `docs/design.md`

- [ ] **Step 1: Update ranking, freshness and the interface**

```diff
diff --git a/docs/design.md b/docs/design.md
index 6130fce..4d26144 100644
--- a/docs/design.md
+++ b/docs/design.md
@@ -76,7 +76,7 @@ The workspace has five crates: `codecite-core` (schema and query engine), `codec
 Each corpus is one SQLite file in WAL mode, holding sources, chunks, a full-text index, optional vectors and a provider-tagged symbol graph.
 
 ```sql
-CREATE TABLE meta     (key TEXT PRIMARY KEY, value TEXT);          -- schema_version, indexed_at, manifest_hash
+CREATE TABLE meta     (key TEXT PRIMARY KEY, value TEXT);          -- schema_version, indexed_at, max_age_days, manifest_hash
 
 CREATE TABLE source (
   id INTEGER PRIMARY KEY, name TEXT NOT NULL,                       -- a document's versions share its name
@@ -198,15 +198,17 @@ BM25 over code-aware terms does most of the work; vectors and graph signals refi
 
 **Ranking**
 
-- FTS5 `bm25()` with column weights: symbol name highest, then path, then body.
+- The query is split into terms as indexed text is (step 4), and each term is quoted, so FTS5 never reads the user's text as query syntax. A chunk matches when it holds any term (`OR`), and BM25 ranks the chunks that hold more of them, and rarer ones, higher.
+- FTS5 `bm25()` with column weights: symbol name 10, path 5, body 1.
 - With embeddings on, nearest neighbors from `chunk_vec`, merged with BM25 by reciprocal rank fusion (k = 60).
-- An exact symbol-name match is boosted to the top.
-- Filters by corpus, repo, language and path glob are plain SQL `WHERE` clauses.
+- A chunk whose symbol's name is exactly the query, as written, comes first; the rest follow by score.
+- Only the current version of each document is searched; an older pinned version is reached by its citation.
+- Filters by corpus, repo, language and path glob are plain SQL `WHERE` clauses. The path glob is SQLite's `GLOB`: `*` matches any run of characters, `/` included, and case matters.
 
 **Freshness**
 
 - Each document stores a content hash. Re-indexing a git source diffs the old commit against the new one and rewrites only changed documents.
-- Each corpus has a maximum age (default 30 days). Older indexes are refused unless the caller passes `--allow-stale`, and every result carries its `indexed_at`.
+- Each corpus has a maximum age (default 30 days). An index run stores it in `meta.max_age_days` beside `meta.indexed_at`, so a query needs no manifest. Older indexes are refused unless the caller passes `--allow-stale`, and every result carries its source's `indexed_at`.
 
 **Initial targets, to validate in milestone 4:** index 100k lines of code in under 60 seconds without SCIP on a laptop, and answer BM25 queries over 1 million chunks in under 100 ms at p95.
 
@@ -264,7 +266,7 @@ The CLI and the MCP server expose the same operations and return the same JSON,
 | --- | --- |
 | `codecite add <corpus> <url>...` | Add sources to a manifest, detect licenses, pin the current commit |
 | `codecite index <corpus> [--manifest <path>] [--print-hashes]` | Fetch, index and run providers; incremental by default. Reads `corpora/<corpus>.toml` unless `--manifest` names another file. `--print-hashes` downloads each document source and prints its sha256, without checking or indexing, so a new or changed document is pinned on purpose |
-| `codecite search <corpus> "<query>"` | Ranked chunks with citations |
+| `codecite search <corpus> "<query>" [--language <name>] [--path <glob>] [--limit <n>] [--json] [--allow-stale]` | Ranked chunks with citations, at most `--limit` (default 10). `--json` prints the result objects as a JSON array; otherwise each result is its rank and citation, a line naming its kind, symbol, score, license class and `indexed_at`, and its snippet's first 3 lines |
 | `codecite def <symbol>` | Where a symbol is defined |
 | `codecite refs <symbol>` | Every reference to a symbol |
 | `codecite graph <callers\|callees\|impact\|path> ...` | Graph queries with `--min-confidence` |
@@ -300,10 +302,11 @@ Every result has the same shape:
 }
 ```
 
+- Search results also carry the chunk's `kind`; a code result carries `dirty`, and a spec result its pinned spec citation and no `symbol`. Fields with no value are left out, and graph fields (`providers`, `confidence`) appear only on graph results.
 - Snippets are capped at 40 lines; agents call `read` for more, which keeps token use predictable.
 - Code citations look like `repo@sha:path#L10-L42`; spec citations like `rfc9449@sha256:1a2b3c4d5e6f#section-4.3` and `openid-connect-core-1_0@sha256:…#section-3.1.3.7/step-13`. The canonical forms parse strictly, so that each span has exactly one spelling: a commit is 7 to 64 lowercase hex digits, a spec hash is 12, numbers have no leading zeros, and one line is `#L10`, not `#L10-L10`. `read` and `cite` also accept the forms people write (`RFC 9449 §4.3`, `OIDC Core §3.1.3.7 step 13`, `Core 3.1.3.7(13)`, a spec URL with its anchor). These parse to an unresolved reference (a document or none, a section number or the document's own anchor, and an optional step), with document aliases taken from the manifest. A string that matches no form is no match rather than an error, and one string holds one citation: step ranges such as `steps 4-6` and lists such as `§2 / §3.1.3.7` are errors. A bare `§` resolves only when exactly one current document has that section and step.
 - `cite` findings: `no_such_section`, `no_such_step`, `anchor_label_mismatch`, `strength_mismatch`, `superseded_document`, `topic_mismatch` (only when exactly one sibling step or section holds the claim's protocol term) and `ambiguous_document`. codecite supplies facts; the agent judges meaning.
-- Errors are explicit: an unknown corpus, a stale index or an ambiguous symbol name returns a typed error with candidates, never an empty success.
+- Errors are explicit: an unknown corpus (with the corpora that exist), a stale index, an empty query or an ambiguous symbol name returns a typed error with candidates, never an empty success. A search that matches nothing is an empty list.
 
 ## Security and privacy
```

- [ ] **Step 2: Commit**

```bash
git add docs/design.md
git commit -m "docs: describe how search matches, ranks and refuses stale indexes (#26)" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```
