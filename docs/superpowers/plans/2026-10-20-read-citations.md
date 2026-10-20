# Read Citations with `codecite read` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement #27: `codecite read <corpus> <citation>` prints the exact text a citation points at. A code citation gives the cited lines of the indexed file, capped by `--max-lines` (default 80). A spec citation, canonical (#15) or written as people write it (#16), gives the section or step text with its canonical pinned citation, title and strength, and one without a version reads the current version. A bare § resolves only when exactly one current document has that section and step; otherwise it is a typed `AmbiguousDocument` error with candidates. A missing section, step, file or version is a typed error with the nearest candidates. Nothing is guessed, nothing is an empty success, and the output always names the pinned version.

**Architecture:** As with search, the logic lives in `codecite-core`, so the MCP `read` tool (#32) reuses it. A new `store/read.rs` adds `Store::read(&self, citation, &ReadOptions) -> Result<ReadResult>`. `ReadResult` derives `Serialize` for `--json` and implements `Display` for the human form. `read` first tries the written forms (`SpecReference::parse` with the corpus's aliases), then the canonical ones (`Citation::from_str`). It resolves the document version through `source` (by name, then a hash or commit prefix, else `is_current`), and the section by `chunk.number` or `chunk.anchor`, then the step by `parent_id` and `number`. A bare § is checked against every current document. Two things a query needs were never stored, so migration 0004 adds them:
- The manifest's document `aliases` go in a new `alias` table, written each index run, just as `max_age_days` went into `meta` (DEC-016), so `read` needs no manifest.
- Code files keep their whole text in a new `document.text` column. The chunker drops lines with no word between definitions (a lone `}`), so the chunks cannot rebuild the exact lines. The migration empties code documents' `content_hash`, so the next index run rewrites them with their text.

The errors with candidates are new `codecite_core::Error` variants. The binary adds a `Read` subcommand.

**Tech Stack:** Rust 1.99.1 (edition 2024), rusqlite 0.40 (bundled SQLite), serde and serde_json (already in core), thiserror. No new dependencies; `cargo deny check` is unchanged. Prototyped: every gate passes (271 passed, 6 ignored), and the real CLI reads this repo and the seven real spec pages (output in Task 2, Step 6).

**Spec:** Issue #27; `docs/design.md#interfaces` (citations, errors) and `docs/superpowers/specs/2026-10-17-identity-corpus-design.md` (bare §, versions, and the anti-goal "spec text without its pinned version"). The decisions the issue and design.md leave open are in the loop's DEC-017 and restated below; Task 3 writes them into design.md.

## Global Constraints

- CLI: `codecite read <corpus> <citation> [--max-lines <n>] [--json] [--allow-stale]`. The corpus is positional, as in `search`; how the MCP tool picks a corpus is #32's question. `--max-lines` is a `u32` of at least 1 (clap rejects 0; the core reads 0 as 1). An index older than `max_age_days` is refused unless `--allow-stale`, through the same `check_fresh` that `search` now calls.
- Parsing order: trim, then `SpecReference::parse(text, &store.aliases()?)`. `Ok(Some)` is a written spec citation. Its `CitationError::UnknownDocument` becomes `Error::UnknownSource` with the corpus's document names, and any other error is `Error::BadCitation`. `Ok(None)` falls through to `text.parse::<Citation>()`, where `MissingAt` is `Error::NotACitation` and any other error is `BadCitation`.
- Versions: a canonical spec citation picks the version whose `revision` starts with its 12-hex hash. A written one picks the `is_current` version. A code citation picks the non-document source of that name whose revision starts with its commit, which may be abbreviated, and its result cites the full revision. A spec result cites `<doc>@sha256:<first 12 hex>#section-<n>[/step-<k>]`.
- Targets: `SpecTarget::Section(n)` is the first `section` chunk with `number = n`, and `SpecTarget::Anchor(a)` the first with `anchor = a`. A step is the `step` chunk whose `parent_id` is that section and whose `number` is `k`. A step's `title` is its section's (step chunks store none). `strength` is the chunk's keywords as a list, and an empty list when it has none.
- Bare §: list the current documents with the section. If exactly one also has the step (or no step was asked for), it resolves. If none has the step and only one has the section, that document reports `NoSuchStep`. Otherwise it is `AmbiguousDocument { reference, candidates }`, with the citations of the documents that have the step (when several do) or the section, and an empty list when none has it.
- Candidates (at most 5): `NoSuchSection` ranks sections by the most leading numbers shared, then the distance in the first number that differs, then fewer parts, then document order. `NoSuchStep` ranks steps by distance and then number, or gives the section itself when it has no steps. `NoSuchFile` gives indexed paths with the same file name. `UnknownVersion` gives every indexed version, and `UnknownSource` every name of the same kind (documents for a spec, other sources for code).
- Code lines are counted as the chunker counts them (`Lines::new`): split at `\n`, a final newline ends the last line, and a `\r` before `\n` is dropped. A range whose end is past the last line is `LinesPastEnd { citation, lines }`. A code document with no stored text is `InvalidCorpus` ("re-run codecite index").
- Text is the first `max_lines` lines of the span, joined with `\n`. `truncated` says whether any were left out. The citation is still the one resolved, not narrowed.
- Aliases: `index_corpus` builds a `SpecAliases` from every non-excluded document's `aliases` before writing anything. An alias already naming another document is `codecite_index::Error::DuplicateAlias`. `Store::set_aliases` replaces the `alias` table in one transaction, next to `set_indexed_at`. Aliases are stored normalized (`SpecAliases::iter`).
- Human form: the citation, then `<kind>[ · <title>][ · <KEYWORD, …>] · <license_class> · indexed <indexed_at>[ · dirty]`, a blank line, the text, and `… (truncated)` when cut. The binary escapes control characters line by line, as search does.
- The workspace forbids `unsafe_code` and warns on `missing_docs`; every public item and field gets a doc comment.
- Gates: `cargo fmt --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked && cargo deny check`. Before the PR, also run `/var/tmp/read-proto/real-run.sh <worktree>` (Task 2, Step 6) and put its output in the PR body.

## Review Focus

- Never a guess: `a_bare_section_in_two_documents_is_ambiguous` (§4.3 is in both fixture specs; `§4.3 step 2` resolves because only one has that step), `a_missing_step_or_section_names_the_nearest`, and `a_missing_file_commit_or_source_is_an_error_with_candidates`.
- Exact text: `a_code_citation_reads_the_exact_lines` reads a `}` line that no chunk holds. The CLI test `read_returns_what_a_search_result_cites` reads back a citation printed by `search`.
- The pinned version: every case in `written_forms_read_the_current_version_and_name_it` comes back as `…@sha256:<hash>…`, and `a_canonical_spec_citation_reads_its_version` reads an older pinned version by its hash.
- Migration 0004 (`version_4_makes_the_next_run_rewrite_code_files`) empties only code documents' hashes, so specs are not re-split.

---

### Task 1: Read citations from the store (`codecite-core`, `codecite-index`)

**Files:**
- Create: `crates/codecite-core/migrations/0004_read.sql`, `crates/codecite-core/src/store/read.rs`
- Modify: `crates/codecite-core/src/citation/written.rs`, `crates/codecite-core/src/error.rs`, `crates/codecite-core/src/lib.rs`, `crates/codecite-core/src/store.rs`, `crates/codecite-core/src/store/search.rs`, `crates/codecite-core/src/store/write.rs`
- Modify: `crates/codecite-index/src/error.rs`, `crates/codecite-index/src/index.rs`

**Interfaces:**
- Produces (for Task 2 and the MCP server): `Store::read(&self, &str, &ReadOptions) -> Result<ReadResult>`, `Store::aliases(&self) -> Result<SpecAliases>`, `Store::set_aliases(&mut self, &SpecAliases) -> Result<()>`, `ReadOptions { max_lines, allow_stale }` (`Default`: 80, false), `ReadResult` (`Serialize`, `Display`), `DEFAULT_MAX_LINES`, `SpecAliases::iter`. New `Error` variants: `NotACitation`, `BadCitation`, `UnknownSource`, `UnknownVersion`, `AmbiguousDocument`, `NoSuchSection`, `NoSuchStep`, `NoSuchFile`, `LinesPastEnd`. New `codecite_index::Error::DuplicateAlias`.
- Changes: `DocumentRecord` gains `text: Option<&str>`, which is `Some` for code and `None` for specs, so every struct literal names it. `SCHEMA_VERSION` becomes 4.

- [ ] **Step 1: Write the failing tests**

This adds the tests for the new errors, the alias iterator, the stored text and aliases, migration 0004 and the index run. It creates `store/read.rs` holding only its test module, which is compiled once Step 3 adds `mod read;`. The existing `DocumentRecord` literals in tests gain `text`.

```diff
diff --git a/crates/codecite-core/src/citation/written.rs b/crates/codecite-core/src/citation/written.rs
--- a/crates/codecite-core/src/citation/written.rs
+++ b/crates/codecite-core/src/citation/written.rs
@@ -342,6 +342,17 @@
     }
 
     #[test]
+    fn iter_yields_normalized_aliases() {
+        let a: SpecAliases = [("OIDC  Core", "openid-connect-core-1_0")]
+            .into_iter()
+            .collect();
+        let pairs: Vec<(&str, &str)> = a.iter().collect();
+        assert_eq!(pairs, [("oidc core", "openid-connect-core-1_0")]);
+        let again: SpecAliases = a.iter().collect();
+        assert_eq!(again, a);
+    }
+
+    #[test]
     fn an_alias_wins_over_the_rfc_rule() {
         let a: SpecAliases = [("RFC 6749bis", "draft-ietf-oauth-v2-1")]
             .into_iter()
diff --git a/crates/codecite-core/src/error.rs b/crates/codecite-core/src/error.rs
--- a/crates/codecite-core/src/error.rs
+++ b/crates/codecite-core/src/error.rs
@@ -172,6 +172,59 @@
     }
 
     #[test]
+    fn read_errors_list_their_candidates() {
+        let err = Error::AmbiguousDocument {
+            reference: "§4.3".into(),
+            candidates: vec![
+                "rfc9449@sha256:1a2b3c4d5e6f#section-4.3".into(),
+                "rfc6749@sha256:abcdefabcdef#section-4.3".into(),
+            ],
+        };
+        assert_eq!(
+            err.to_string(),
+            "§4.3 names no document and is not in exactly one current document; name one: rfc9449@sha256:1a2b3c4d5e6f#section-4.3, rfc6749@sha256:abcdefabcdef#section-4.3"
+        );
+        let err = Error::AmbiguousDocument {
+            reference: "§99".into(),
+            candidates: vec![],
+        };
+        assert_eq!(err.to_string(), "no current document has §99");
+        let err = Error::NoSuchStep {
+            section: "rfc9449@sha256:1a2b3c4d5e6f#section-4.3".into(),
+            step: 14,
+            candidates: vec!["rfc9449@sha256:1a2b3c4d5e6f#section-4.3/step-12".into()],
+        };
+        assert_eq!(
+            err.to_string(),
+            "rfc9449@sha256:1a2b3c4d5e6f#section-4.3 has no step 14; nearest: rfc9449@sha256:1a2b3c4d5e6f#section-4.3/step-12"
+        );
+        let err = Error::NoSuchFile {
+            repo: "app@0123abc".into(),
+            path: "src/nope.rs".into(),
+            candidates: vec![],
+        };
+        assert_eq!(
+            err.to_string(),
+            "app@0123abc has no indexed file \"src/nope.rs\"; no candidates"
+        );
+    }
+
+    #[test]
+    fn a_bad_citation_keeps_the_parse_error_as_its_source() {
+        let err = Error::BadCitation {
+            text: "§04.3".into(),
+            source: CitationError::BadSection {
+                found: "04.3".into(),
+            },
+        };
+        assert_eq!(err.to_string(), "invalid citation \"§04.3\"");
+        assert_eq!(
+            err.source().unwrap().to_string(),
+            "section \"04.3\" is not dot-separated numbers like 3.1.3.7"
+        );
+    }
+
+    #[test]
     fn schema_too_new_display_names_path_and_versions() {
         let err = Error::SchemaTooNew {
             path: "/tmp/corpus.db".into(),
diff --git a/crates/codecite-core/src/store.rs b/crates/codecite-core/src/store.rs
--- a/crates/codecite-core/src/store.rs
+++ b/crates/codecite-core/src/store.rs
@@ -658,6 +658,50 @@
     }
 
     #[test]
+    fn version_4_makes_the_next_run_rewrite_code_files() {
+        let dir = scratch("v3");
+        let path = dir.join("corpus.db");
+        // A version 3 corpus with a code file and a spec version.
+        raw(&path)
+            .execute_batch(&format!(
+                "{}
+                 {}
+                 {}
+                 INSERT INTO meta (key, value) VALUES ('schema_version', '3');
+                 INSERT INTO source (id, name, kind, url, revision, license_class, indexed_at)
+                   VALUES (1, 'app', 'local', '/src/app', 'abc', 'adapt', 'then'),
+                          (2, 'rfc9449', 'document', 'https://x', 'def', 'quote', 'then');
+                 INSERT INTO document (id, source_id, path, content_hash)
+                   VALUES (1, 1, 'src/lib.rs', x'01'), (2, 2, 'rfc9449', x'02');",
+                MIGRATIONS[0], MIGRATIONS[1], MIGRATIONS[2]
+            ))
+            .unwrap();
+
+        let store = Store::open_read_write(&path).unwrap();
+        assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
+        let rows: Vec<(String, Vec<u8>, Option<String>)> = {
+            let mut stmt = store
+                .conn
+                .prepare("SELECT path, content_hash, text FROM document ORDER BY id")
+                .unwrap();
+            stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
+                .unwrap()
+                .collect::<rusqlite::Result<_>>()
+                .unwrap()
+        };
+        assert_eq!(
+            rows,
+            [
+                ("src/lib.rs".to_owned(), vec![], None),
+                ("rfc9449".to_owned(), vec![2], None)
+            ]
+        );
+        assert!(table_names(&store.conn).iter().any(|t| t == "alias"));
+        drop(store);
+        fs::remove_dir_all(&dir).unwrap();
+    }
+
+    #[test]
     fn a_migration_that_leaves_orphans_is_refused_unchanged() {
         let dir = scratch("orphans");
         let path = dir.join("corpus.db");
diff --git a/crates/codecite-core/src/store/read.rs b/crates/codecite-core/src/store/read.rs
new file mode 100644
--- /dev/null
+++ b/crates/codecite-core/src/store/read.rs
@@ -0,0 +1,533 @@
+#[cfg(test)]
+mod tests {
+    use super::*;
+    use crate::{ChunkRecord, DocumentRecord, Keyword, SourceRecord};
+
+    const HEAD: &str = "0123456789abcdef0123456789abcdef01234567";
+    const RFC9449: &str = "1a2b3c4d5e6f00000000000000000000000000000000000000000000000000ff";
+    const CORE: &str = "c0de00000000000000000000000000000000000000000000000000000000c0de";
+    /// Five lines; the chunker gives the lone `}` no chunk.
+    const LIB: &str = "pub fn validate() {\n    check();\n}\n\npub fn check() {}\n";
+
+    fn source<'a>(
+        name: &'a str,
+        kind: &'a str,
+        revision: &'a str,
+        current: bool,
+    ) -> SourceRecord<'a> {
+        SourceRecord {
+            name,
+            kind,
+            url: "https://example.com",
+            revision,
+            license_spdx: None,
+            license_class: if kind == "document" { "quote" } else { "adapt" },
+            is_current: current,
+            obsoleted_by: None,
+            updated_by: None,
+        }
+    }
+
+    fn section<'a>(
+        number: &'a str,
+        anchor: &'a str,
+        title: &'a str,
+        text: &'a str,
+    ) -> ChunkRecord<'a> {
+        ChunkRecord {
+            kind: "section",
+            start_line: 1,
+            end_line: 1,
+            text,
+            number: Some(number),
+            anchor: Some(anchor),
+            title: Some(title),
+            parent: None,
+            strength: &[],
+            symbol: None,
+        }
+    }
+
+    fn step<'a>(number: &'a str, parent: usize, text: &'a str) -> ChunkRecord<'a> {
+        ChunkRecord {
+            kind: "step",
+            number: Some(number),
+            anchor: None,
+            title: None,
+            parent: Some(parent),
+            strength: &[Keyword::Must],
+            ..section(number, "", "", text)
+        }
+    }
+
+    fn spec<'a>(path: &'a str, chunks: Vec<ChunkRecord<'a>>) -> DocumentRecord<'a> {
+        DocumentRecord {
+            path,
+            language: None,
+            content_hash: path.as_bytes(),
+            size_bytes: 1,
+            dirty: false,
+            text: None,
+            chunks,
+        }
+    }
+
+    fn code<'a>(path: &'a str, text: &'a str) -> DocumentRecord<'a> {
+        DocumentRecord {
+            path,
+            language: Some("rust"),
+            content_hash: path.as_bytes(),
+            size_bytes: 1,
+            dirty: true,
+            text: Some(text),
+            chunks: Vec::new(),
+        }
+    }
+
+    /// A corpus with a code source, RFC 9449 at two versions and OIDC Core.
+    /// Both specs have a §4.3; only Core has §3.1.3.7.
+    struct Fixture {
+        store: Store,
+        _dir: tempfile::TempDir,
+    }
+
+    impl Fixture {
+        fn read(&self, citation: &str) -> Result<ReadResult> {
+            self.store.read(citation, &ReadOptions::default())
+        }
+
+        fn ok(&self, citation: &str) -> ReadResult {
+            self.read(citation)
+                .unwrap_or_else(|e| panic!("{citation}: {e}"))
+        }
+    }
+
+    fn fixture() -> Fixture {
+        let dir = tempfile::tempdir().unwrap();
+        let path = dir.path().join("demo.db");
+        let mut store = Store::open_read_write(&path).unwrap();
+        let app = store
+            .upsert_source(&source("app", "local", HEAD, true))
+            .unwrap();
+        store.write_document(app, &code("src/lib.rs", LIB)).unwrap();
+        store
+            .write_document(app, &code("tests/lib.rs", "fn t() {}\n"))
+            .unwrap();
+
+        let rfc = store
+            .upsert_source(&source("rfc9449", "document", RFC9449, true))
+            .unwrap();
+        let checking = "4.3.  Checking DPoP Proofs\nTo validate a DPoP proof, the receiving server MUST ensure:";
+        store
+            .write_document(
+                rfc,
+                &spec(
+                    "rfc9449",
+                    vec![
+                        section("4.3", "section-4.3", "Checking DPoP Proofs", checking),
+                        step("1", 0, "1. There is not more than one DPoP HTTP request header field."),
+                        step("2", 0, "2. The DPoP HTTP request header field value is a single well-formed JWT."),
+                        step("12", 0, "12. The jti is unique."),
+                        section("4.4", "section-4.4", "Signing", "4.4.  Signing"),
+                        section("5", "section-5", "Protected Resources", "5.  Protected Resources"),
+                    ],
+                ),
+            )
+            .unwrap();
+        let old = "a".repeat(64);
+        let older = store
+            .upsert_source(&source("rfc9449", "document", &old, false))
+            .unwrap();
+        store
+            .write_document(
+                older,
+                &spec(
+                    "rfc9449",
+                    vec![section(
+                        "4.3",
+                        "section-4.3",
+                        "Checking DPoP Proofs",
+                        "the old text",
+                    )],
+                ),
+            )
+            .unwrap();
+
+        let core = store
+            .upsert_source(&source("openid-connect-core-1_0", "document", CORE, true))
+            .unwrap();
+        store
+            .write_document(
+                core,
+                &spec(
+                    "openid-connect-core-1_0",
+                    vec![
+                        section(
+                            "3.1.3.7",
+                            "IDTokenValidation",
+                            "ID Token Validation",
+                            "3.1.3.7.  ID Token Validation",
+                        ),
+                        step("12", 0, "12. If the acr Claim was requested, ..."),
+                        step("13", 0, "13. If the auth_time Claim was requested, ..."),
+                        section("4.3", "section-4.3", "Unrelated", "4.3.  Unrelated"),
+                    ],
+                ),
+            )
+            .unwrap();
+        let aliases: SpecAliases = [
+            ("OIDC Core", "openid-connect-core-1_0"),
+            ("Core", "openid-connect-core-1_0"),
+        ]
+        .into_iter()
+        .collect();
+        store.set_aliases(&aliases).unwrap();
+        store.set_indexed_at(30).unwrap();
+        drop(store);
+        Fixture {
+            store: Store::open_read_only(&path).unwrap(),
+            _dir: dir,
+        }
+    }
+
+    #[test]
+    fn a_code_citation_reads_the_exact_lines() {
+        let f = fixture();
+        // An abbreviated commit, and a `}` line that no chunk holds.
+        let r = f.ok("app@0123456:src/lib.rs#L2-L3");
+        assert_eq!(r.citation, format!("app@{HEAD}:src/lib.rs#L2-L3"));
+        assert_eq!(r.kind, "code");
+        assert_eq!(r.text, "    check();\n}");
+        assert_eq!((r.dirty, r.truncated), (Some(true), false));
+        assert_eq!(r.license_class, "adapt");
+        assert_eq!((r.title, r.strength), (None, None));
+        let r = f.ok(&format!("app@{HEAD}:src/lib.rs#L5"));
+        assert_eq!(r.text, "pub fn check() {}");
+    }
+
+    #[test]
+    fn a_code_citation_past_the_end_of_the_file_is_an_error() {
+        let f = fixture();
+        for lines in ["L5-L6", "L9"] {
+            let err = f
+                .read(&format!("app@0123456:src/lib.rs#{lines}"))
+                .unwrap_err();
+            assert!(
+                matches!(err, Error::LinesPastEnd { lines: 5, .. }),
+                "{lines}: {err:?}"
+            );
+        }
+    }
+
+    #[test]
+    fn max_lines_caps_the_text() {
+        let f = fixture();
+        let options = ReadOptions {
+            max_lines: 2,
+            ..ReadOptions::default()
+        };
+        let r = f
+            .store
+            .read("app@0123456:src/lib.rs#L1-L5", &options)
+            .unwrap();
+        assert_eq!(r.text, "pub fn validate() {\n    check();");
+        assert!(r.truncated);
+        assert_eq!(r.citation, format!("app@{HEAD}:src/lib.rs#L1-L5"));
+        let r = f
+            .store
+            .read(
+                "RFC 9449 §4.3",
+                &ReadOptions {
+                    max_lines: 1,
+                    ..options
+                },
+            )
+            .unwrap();
+        assert_eq!(r.text, "4.3.  Checking DPoP Proofs");
+        assert!(r.truncated);
+    }
+
+    #[test]
+    fn a_missing_file_commit_or_source_is_an_error_with_candidates() {
+        let f = fixture();
+        let err = f.read("app@0123456:src/nested/lib.rs#L1").unwrap_err();
+        assert!(
+            matches!(&err, Error::NoSuchFile { candidates, .. }
+                if candidates == &["src/lib.rs", "tests/lib.rs"]),
+            "{err:?}"
+        );
+        let err = f.read("app@fedcba9:src/lib.rs#L1").unwrap_err();
+        assert!(
+            matches!(&err, Error::UnknownVersion { known, .. } if known == &[format!("app@{HEAD}")]),
+            "{err:?}"
+        );
+        let err = f.read("other@0123456:src/lib.rs#L1").unwrap_err();
+        assert!(
+            matches!(&err, Error::UnknownSource { known, .. } if known == &["app"]),
+            "{err:?}"
+        );
+    }
+
+    #[test]
+    fn a_canonical_spec_citation_reads_its_version() {
+        let f = fixture();
+        let r = f.ok("rfc9449@sha256:1a2b3c4d5e6f#section-4.3");
+        assert_eq!(r.kind, "section");
+        assert_eq!(r.title.as_deref(), Some("Checking DPoP Proofs"));
+        assert!(r.text.starts_with("4.3.  Checking DPoP Proofs\n"));
+        assert_eq!(r.strength, Some(vec![]));
+        assert_eq!((r.license_class.as_str(), r.dirty), ("quote", None));
+        let r = f.ok("rfc9449@sha256:1a2b3c4d5e6f#section-4.3/step-12");
+        assert_eq!(
+            r.citation,
+            "rfc9449@sha256:1a2b3c4d5e6f#section-4.3/step-12"
+        );
+        assert_eq!(r.kind, "step");
+        assert_eq!(r.text, "12. The jti is unique.");
+        assert_eq!(r.title.as_deref(), Some("Checking DPoP Proofs"));
+        assert_eq!(r.strength, Some(vec!["MUST".to_owned()]));
+        // An older pinned version is read by its hash.
+        let r = f.ok("rfc9449@sha256:aaaaaaaaaaaa#section-4.3");
+        assert_eq!(r.text, "the old text");
+        let err = f
+            .read("rfc9449@sha256:bbbbbbbbbbbb#section-4.3")
+            .unwrap_err();
+        assert!(
+            matches!(&err, Error::UnknownVersion { known, .. }
+                if known == &["rfc9449@sha256:1a2b3c4d5e6f", "rfc9449@sha256:aaaaaaaaaaaa"]),
+            "{err:?}"
+        );
+    }
+
+    #[test]
+    fn written_forms_read_the_current_version_and_name_it() {
+        let f = fixture();
+        let core = "openid-connect-core-1_0@sha256:c0de00000000#section-3.1.3.7";
+        for (written, citation) in [
+            (
+                "RFC 9449 §4.3",
+                "rfc9449@sha256:1a2b3c4d5e6f#section-4.3".to_owned(),
+            ),
+            (
+                "RFC9449 Section 4.3",
+                "rfc9449@sha256:1a2b3c4d5e6f#section-4.3".to_owned(),
+            ),
+            (
+                "RFC 9449 §4.3 step 2",
+                "rfc9449@sha256:1a2b3c4d5e6f#section-4.3/step-2".to_owned(),
+            ),
+            ("OIDC Core §3.1.3.7 step 13", format!("{core}/step-13")),
+            ("Core 3.1.3.7(13)", format!("{core}/step-13")),
+            (
+                "https://www.rfc-editor.org/rfc/rfc9449.html#section-4.3",
+                "rfc9449@sha256:1a2b3c4d5e6f#section-4.3".to_owned(),
+            ),
+            (
+                "https://openid.net/specs/openid-connect-core-1_0.html#IDTokenValidation",
+                core.to_owned(),
+            ),
+            ("§3.1.3.7 step 13", format!("{core}/step-13")),
+        ] {
+            assert_eq!(f.ok(written).citation, citation, "{written}");
+        }
+        assert_eq!(
+            f.ok("Core 3.1.3.7(13)").text,
+            "13. If the auth_time Claim was requested, ..."
+        );
+    }
+
+    #[test]
+    fn a_bare_section_in_two_documents_is_ambiguous() {
+        let f = fixture();
+        let err = f.read("§4.3").unwrap_err();
+        assert!(
+            matches!(&err, Error::AmbiguousDocument { reference, candidates }
+            if reference == "§4.3" && candidates == &[
+                "openid-connect-core-1_0@sha256:c0de00000000#section-4.3",
+                "rfc9449@sha256:1a2b3c4d5e6f#section-4.3",
+            ]),
+            "{err:?}"
+        );
+        // Only RFC 9449's §4.3 has a step 2, so it is the one document.
+        assert_eq!(
+            f.ok("§4.3 step 2").citation,
+            "rfc9449@sha256:1a2b3c4d5e6f#section-4.3/step-2"
+        );
+        let err = f.read("§4.3 step 7").unwrap_err();
+        assert!(
+            matches!(&err, Error::AmbiguousDocument { candidates, .. } if candidates.len() == 2),
+            "{err:?}"
+        );
+        let err = f.read("§9.9").unwrap_err();
+        assert!(
+            matches!(&err, Error::AmbiguousDocument { candidates, .. } if candidates.is_empty()),
+            "{err:?}"
+        );
+    }
+
+    #[test]
+    fn a_missing_step_or_section_names_the_nearest() {
+        let f = fixture();
+        let err = f.read("RFC 9449 §4.3 step 14").unwrap_err();
+        let pinned = "rfc9449@sha256:1a2b3c4d5e6f#section-4.3";
+        assert!(
+            matches!(&err, Error::NoSuchStep { section, step: 14, candidates }
+                if section == pinned
+                && candidates == &[format!("{pinned}/step-12"), format!("{pinned}/step-2"), format!("{pinned}/step-1")]),
+            "{err:?}"
+        );
+        // The only document with §3.1.3.7 reports its missing step.
+        let err = f.read("§3.1.3.7 step 14").unwrap_err();
+        assert!(
+            matches!(&err, Error::NoSuchStep { candidates, .. } if candidates[0].ends_with("/step-13")),
+            "{err:?}"
+        );
+        let err = f.read("RFC 9449 §4.9").unwrap_err();
+        assert!(
+            matches!(&err, Error::NoSuchSection { document, section, candidates }
+            if document == "rfc9449@sha256:1a2b3c4d5e6f" && section == "4.9"
+            && candidates == &[
+                "rfc9449@sha256:1a2b3c4d5e6f#section-4.4",
+                "rfc9449@sha256:1a2b3c4d5e6f#section-4.3",
+                "rfc9449@sha256:1a2b3c4d5e6f#section-5",
+            ]),
+            "{err:?}"
+        );
+        let err = f
+            .read("https://openid.net/specs/openid-connect-core-1_0.html#NoSuchAnchor")
+            .unwrap_err();
+        assert!(
+            matches!(&err, Error::NoSuchSection { section, .. } if section == "#NoSuchAnchor"),
+            "{err:?}"
+        );
+    }
+
+    #[test]
+    fn text_that_is_no_citation_or_a_bad_one_is_an_error() {
+        let f = fixture();
+        assert!(matches!(
+            f.read("hello world"),
+            Err(Error::NotACitation { .. })
+        ));
+        assert!(matches!(
+            f.read("RFC 9449 §04.3"),
+            Err(Error::BadCitation {
+                source: CitationError::BadSection { .. },
+                ..
+            })
+        ));
+        assert!(matches!(
+            f.read("rfc9449@sha256:1a2b#section-4.3"),
+            Err(Error::BadCitation {
+                source: CitationError::BadHash { .. },
+                ..
+            })
+        ));
+        let err = f.read("Foo §3.1").unwrap_err();
+        assert!(
+            matches!(&err, Error::UnknownSource { name, known }
+                if name == "Foo" && known == &["openid-connect-core-1_0", "rfc9449"]),
+            "{err:?}"
+        );
+        assert!(matches!(
+            f.read("RFC 9999 §1"),
+            Err(Error::UnknownSource { .. })
+        ));
+    }
+
+    #[test]
+    fn a_stale_index_is_refused_unless_allowed() {
+        let f = fixture();
+        rusqlite::Connection::open(f.store.path())
+            .unwrap()
+            .execute(
+                "UPDATE meta SET value = '2000-01-01T00:00:00Z' WHERE key = 'indexed_at'",
+                [],
+            )
+            .unwrap();
+        assert!(matches!(
+            f.read("RFC 9449 §4.3"),
+            Err(Error::StaleIndex { .. })
+        ));
+        let options = ReadOptions {
+            allow_stale: true,
+            ..ReadOptions::default()
+        };
+        assert!(f.store.read("RFC 9449 §4.3", &options).is_ok());
+    }
+
+    #[test]
+    fn a_code_file_without_text_asks_for_a_reindex() {
+        let f = fixture();
+        rusqlite::Connection::open(f.store.path())
+            .unwrap()
+            .execute("UPDATE document SET text = NULL", [])
+            .unwrap();
+        let err = f.read("app@0123456:src/lib.rs#L1").unwrap_err();
+        assert!(matches!(err, Error::InvalidCorpus { .. }), "{err:?}");
+    }
+
+    #[test]
+    fn json_and_human_forms() {
+        let f = fixture();
+        let spec = serde_json::to_value(f.ok("Core 3.1.3.7(13)")).unwrap();
+        let keys: Vec<&String> = spec.as_object().unwrap().keys().collect();
+        assert_eq!(
+            keys,
+            [
+                "citation",
+                "indexed_at",
+                "kind",
+                "license_class",
+                "strength",
+                "text",
+                "title",
+                "truncated"
+            ]
+        );
+        let code = serde_json::to_value(f.ok("app@0123456:src/lib.rs#L1")).unwrap();
+        assert_eq!(code["dirty"], true);
+        assert!(code.get("strength").is_none());
+
+        let mut r = f.ok("Core 3.1.3.7(13)");
+        r.indexed_at = "2026-10-20T06:00:00Z".into();
+        assert_eq!(
+            r.to_string(),
+            "openid-connect-core-1_0@sha256:c0de00000000#section-3.1.3.7/step-13
+step · ID Token Validation · MUST · quote · indexed 2026-10-20T06:00:00Z
+
+13. If the auth_time Claim was requested, ...
+"
+        );
+        let mut r = f
+            .store
+            .read(
+                "app@0123456:src/lib.rs#L1-L2",
+                &ReadOptions {
+                    max_lines: 1,
+                    allow_stale: false,
+                },
+            )
+            .unwrap();
+        r.indexed_at = "2026-10-20T06:00:00Z".into();
+        assert_eq!(
+            r.to_string(),
+            format!(
+                "app@{HEAD}:src/lib.rs#L1-L2
+code · adapt · indexed 2026-10-20T06:00:00Z · dirty
+
+pub fn validate() {{
+… (truncated)
+"
+            )
+        );
+    }
+
+    #[test]
+    fn file_lines_count_as_the_chunker_does() {
+        assert_eq!(file_lines("a\r\nb\n"), ["a", "b"]);
+        assert_eq!(file_lines("a\n\n"), ["a", ""]);
+        assert_eq!(file_lines("a"), ["a"]);
+        assert!(file_lines("").is_empty());
+    }
+}
diff --git a/crates/codecite-core/src/store/search.rs b/crates/codecite-core/src/store/search.rs
--- a/crates/codecite-core/src/store/search.rs
+++ b/crates/codecite-core/src/store/search.rs
@@ -367,6 +367,7 @@
             content_hash: path.as_bytes(),
             size_bytes: 1,
             dirty,
+            text: None,
             chunks,
         }
     }
@@ -394,6 +395,7 @@
             content_hash: text.as_bytes(),
             size_bytes: 1,
             dirty: false,
+            text: None,
             chunks: vec![section, step],
         }
     }
diff --git a/crates/codecite-core/src/store/write.rs b/crates/codecite-core/src/store/write.rs
--- a/crates/codecite-core/src/store/write.rs
+++ b/crates/codecite-core/src/store/write.rs
@@ -372,6 +372,7 @@
             content_hash: text.as_bytes(),
             size_bytes: text.len() as u64,
             dirty: false,
+            text: Some(text),
             chunks: vec![ChunkRecord {
                 kind: "function",
                 start_line: 1,
@@ -444,6 +445,11 @@
         assert_eq!(matches(&store, "terms:token"), 1);
         let hashes = store.document_hashes(id).unwrap();
         assert_eq!(hashes["src/lib.rs"], text.as_bytes());
+        let stored: String = store
+            .conn
+            .query_row("SELECT text FROM document", [], |row| row.get(0))
+            .unwrap();
+        assert_eq!(stored, text);
         drop(store);
         fs::remove_dir_all(&dir).unwrap();
     }
@@ -644,6 +650,30 @@
     }
 
     #[test]
+    fn set_aliases_replaces_the_stored_ones() {
+        let (mut store, dir) = store("aliases");
+        let first: SpecAliases = [("OIDC Core", "openid-connect-core-1_0"), ("Core", "x")]
+            .into_iter()
+            .collect();
+        store.set_aliases(&first).unwrap();
+        let second: SpecAliases = [("Core", "openid-connect-core-1_0")].into_iter().collect();
+        store.set_aliases(&second).unwrap();
+        let rows: Vec<(String, String)> = {
+            let mut stmt = store.conn.prepare("SELECT alias, name FROM alias").unwrap();
+            stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
+                .unwrap()
+                .collect::<rusqlite::Result<_>>()
+                .unwrap()
+        };
+        assert_eq!(
+            rows,
+            [("core".to_owned(), "openid-connect-core-1_0".to_owned())]
+        );
+        drop(store);
+        fs::remove_dir_all(&dir).unwrap();
+    }
+
+    #[test]
     fn spec_chunks_store_their_number_parent_and_strength() {
         let (mut store, dir) = store("spec");
         let sha = "1".repeat(64);
@@ -668,6 +698,7 @@
             content_hash: sha.as_bytes(),
             size_bytes: 10,
             dirty: false,
+            text: None,
             chunks: vec![
                 chunk(
                     "section",
diff --git a/crates/codecite-index/src/index.rs b/crates/codecite-index/src/index.rs
--- a/crates/codecite-index/src/index.rs
+++ b/crates/codecite-index/src/index.rs
@@ -553,6 +553,16 @@
         );
         assert_eq!(corpus.matches("symbol:validate"), 1);
         assert_eq!(corpus.matches("token"), 2);
+        // The file's text is kept whole, the blank line no chunk holds too.
+        let text: String = corpus
+            .sql()
+            .query_row(
+                "SELECT text FROM document WHERE path = 'src/lib.rs'",
+                [],
+                |row| row.get(0),
+            )
+            .unwrap();
+        assert_eq!(text, "pub fn validate_token() {}\n\npub struct Client;\n");
         let (name, kind, revision, class): (String, String, String, String) = corpus
             .sql()
             .query_row(
@@ -922,6 +932,35 @@
     }
 
     #[test]
+    fn aliases_are_stored_and_name_one_document() {
+        let server = Server::start();
+        server.set("/rfc9449.html", SPEC);
+        let corpus = Corpus::new();
+        let mut doc = document(server.url("/rfc9449.html"), &sha256(SPEC));
+        if let Source::Document(d) = &mut doc {
+            d.aliases = vec!["DPoP".into(), "OAuth  DPoP".into()];
+        }
+        corpus.index(&manifest(vec![doc.clone()]));
+        let store = Store::open_read_only(&corpus.path()).unwrap();
+        let aliases = store.aliases().unwrap();
+        assert_eq!(aliases.get("dpop"), Some("rfc9449"));
+        assert_eq!(aliases.get("OAuth DPoP"), Some("rfc9449"));
+        drop(store);
+
+        let mut other = doc.clone();
+        if let Source::Document(d) = &mut other {
+            d.name = "rfc9450".into();
+            d.aliases = vec!["dpop".into()];
+        }
+        let err = corpus.try_index(&manifest(vec![doc, other])).unwrap_err();
+        assert!(
+            matches!(&err, Error::DuplicateAlias { alias, first, second }
+                if alias == "dpop" && first == "rfc9449" && second == "rfc9450"),
+            "{err:?}"
+        );
+    }
+
+    #[test]
     fn excluded_documents_are_not_fetched() {
         let server = Server::start();
         let corpus = Corpus::new();
```

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test -p codecite-core --no-run`
Expected: compile errors for what Step 3 adds, among them `cannot find type SpecAliases in this scope` (write.rs), `struct DocumentRecord<'_> has no field named text`, `no method named iter found for struct SpecAliases`, `no method named set_aliases`, and `no variant named AmbiguousDocument` / `BadCitation` / `NoSuchFile` / `NoSuchStep` (16 errors).

- [ ] **Step 3: Add the migration, the stored text and aliases, the errors and `Store::read`**

> **Learn: error types that carry candidates.** Each variant holds the data a caller needs to act, as fields rather than a pre-formatted string: `candidates: Vec<String>` of pinned citations. The CLI prints the message, and the MCP server (#32) can return the same fields as structured content. thiserror's `#[error("…{}", list_of("nearest", candidates))]` calls a helper to format the list, so the message still reads as one line. `BadCitation` keeps the parse error as `#[source]` and leaves it out of its own message, so `anyhow` prints each cause once ("invalid citation … Caused by: section "04.3" is not …").
>
> **Learn: `impl Display` for human output.** `ReadResult` implements `fmt::Display`, so `result.to_string()` (or `{}`) is the human form, and serde's `Serialize` is the JSON form, from one struct. `write!`/`writeln!` into a `Formatter` return `fmt::Result`, and `?` passes on a failed write. The binary escapes control characters itself, since the text is untrusted and the core shouldn't know about terminals.
>
> Also used: slice patterns in `match (with_step.as_slice(), with_section.len())`, where `(&[i], _)` matches exactly one element and binds it; `sort_by_key` with `std::cmp::Reverse` to sort one key descending; and `Option::filter` to skip an empty keyword list.

```diff
diff --git a/crates/codecite-core/migrations/0004_read.sql b/crates/codecite-core/migrations/0004_read.sql
new file mode 100644
--- /dev/null
+++ b/crates/codecite-core/migrations/0004_read.sql
@@ -0,0 +1,11 @@
+-- What `read` needs to answer from the corpus alone, without the manifest or the source tree.
+
+-- The names people write for documents (`OIDC Core`), normalized as `SpecAliases` normalizes them, and the document each names. Each index run replaces them.
+CREATE TABLE alias (alias TEXT PRIMARY KEY, name TEXT NOT NULL);
+
+-- A code file's text as indexed, so `read` returns the exact lines; chunks leave out lines without a word between definitions. NULL for a spec, whose sections and steps are its text.
+ALTER TABLE document ADD COLUMN text TEXT;
+
+-- Code files indexed before this column have no text: an empty hash never matches, so the next index run rewrites them.
+UPDATE document SET content_hash = x''
+  WHERE source_id IN (SELECT id FROM source WHERE kind <> 'document');
diff --git a/crates/codecite-core/src/citation/written.rs b/crates/codecite-core/src/citation/written.rs
--- a/crates/codecite-core/src/citation/written.rs
+++ b/crates/codecite-core/src/citation/written.rs
@@ -29,6 +29,14 @@
     /// The document `alias` names, if any.
     pub fn get(&self, alias: &str) -> Option<&str> {
         self.0.get(&normalize(alias)).map(String::as_str)
+    }
+
+    /// Each alias, as lookups normalize it, with the document it names, in
+    /// no particular order.
+    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
+        self.0
+            .iter()
+            .map(|(alias, doc)| (alias.as_str(), doc.as_str()))
     }
 }
 
diff --git a/crates/codecite-core/src/error.rs b/crates/codecite-core/src/error.rs
--- a/crates/codecite-core/src/error.rs
+++ b/crates/codecite-core/src/error.rs
@@ -1,6 +1,8 @@
 //! Errors returned by the store and the query engine.
 
 use std::path::PathBuf;
+
+use crate::CitationError;
 
 /// An error from `codecite-core`.
 #[derive(Debug, thiserror::Error)]
@@ -73,6 +75,130 @@
     /// The query has no letter or digit to search for.
     #[error("the query has no words to search for")]
     EmptyQuery,
+
+    /// The text matches none of the citation forms `read` accepts.
+    #[error(
+        "{text:?} is not a citation; write repo@sha:path#L10-L42, doc@sha256:<hash>#section-4.3, a spec URL, or a form like RFC 9449 §4.3"
+    )]
+    NotACitation {
+        /// The text as given.
+        text: String,
+    },
+
+    /// The text has the shape of a citation but is malformed.
+    #[error("invalid citation {text:?}")]
+    BadCitation {
+        /// The text as given.
+        text: String,
+        /// What is wrong with it.
+        #[source]
+        source: CitationError,
+    },
+
+    /// The corpus has no source or document with this name.
+    #[error(
+        "the corpus has no source or document `{name}`; {}",
+        list_of("known", known)
+    )]
+    UnknownSource {
+        /// The name the citation gives.
+        name: String,
+        /// The names the corpus does have, of the same kind, sorted.
+        known: Vec<String>,
+    },
+
+    /// The source or document is in the corpus, but not at the version the
+    /// citation names.
+    #[error("`{name}` is not indexed at {version}; {}", list_of("indexed", known))]
+    UnknownVersion {
+        /// The source or document.
+        name: String,
+        /// The version the citation names: a commit, or `sha256:<hash>`.
+        version: String,
+        /// The versions that are indexed, as citations name them.
+        known: Vec<String>,
+    },
+
+    /// A spec citation that names no document, like `§3.1.3.7`, is not in
+    /// exactly one current document.
+    #[error("{}", ambiguous(reference, candidates))]
+    AmbiguousDocument {
+        /// The citation as given.
+        reference: String,
+        /// The pinned citations of each current document that has it.
+        candidates: Vec<String>,
+    },
+
+    /// The document version has no such section.
+    #[error(
+        "{document} has no section {section}; {}",
+        list_of("nearest", candidates)
+    )]
+    NoSuchSection {
+        /// The pinned document: `rfc9449@sha256:1a2b3c4d5e6f`.
+        document: String,
+        /// The section as cited: `4.9`, or `#anchor` for a URL's anchor.
+        section: String,
+        /// The nearest sections that do exist, as pinned citations.
+        candidates: Vec<String>,
+    },
+
+    /// The section has no such step.
+    #[error("{section} has no step {step}; {}", list_of("nearest", candidates))]
+    NoSuchStep {
+        /// The section, as a pinned citation.
+        section: String,
+        /// The step asked for.
+        step: u32,
+        /// The nearest steps that do exist, or the section itself when it
+        /// has none, as pinned citations.
+        candidates: Vec<String>,
+    },
+
+    /// The source has no indexed file at this path.
+    #[error(
+        "{repo} has no indexed file {path:?}; {}",
+        list_of("candidates", candidates)
+    )]
+    NoSuchFile {
+        /// The source at its commit: `app@0123abc`.
+        repo: String,
+        /// The path asked for.
+        path: String,
+        /// Indexed files with the same file name.
+        candidates: Vec<String>,
+    },
+
+    /// A code citation runs past the end of its file.
+    #[error("{citation} runs past the end of the file, which has {lines} lines")]
+    LinesPastEnd {
+        /// The citation, with the source's full commit.
+        citation: String,
+        /// How many lines the file has.
+        lines: u32,
+    },
+}
+
+/// The end of an error message that lists candidates: `nearest: a, b`, or
+/// `no candidates`.
+fn list_of(label: &str, list: &[String]) -> String {
+    if list.is_empty() {
+        "no candidates".into()
+    } else {
+        format!("{label}: {}", list.join(", "))
+    }
+}
+
+/// The message of an [`Error::AmbiguousDocument`].
+fn ambiguous(reference: &str, list: &[String]) -> String {
+    if list.is_empty() {
+        format!("no current document has {reference}")
+    } else {
+        format!(
+            "{reference} names no document and is not in exactly one current document; name one: {}",
+            list.join(", ")
+        )
+    }
 }
 
 /// The end of an [`Error::UnknownCorpus`] message.
diff --git a/crates/codecite-core/src/lib.rs b/crates/codecite-core/src/lib.rs
--- a/crates/codecite-core/src/lib.rs
+++ b/crates/codecite-core/src/lib.rs
@@ -19,8 +19,9 @@
 
 mod store;
 pub use store::{
-    ChunkRecord, DEFAULT_MAX_AGE_DAYS, DocumentRecord, Freshness, MAX_SNIPPET_LINES,
-    SCHEMA_VERSION, SearchQuery, SearchResult, SourceRecord, Store, SymbolRecord, is_corpus_name,
+    ChunkRecord, DEFAULT_MAX_AGE_DAYS, DEFAULT_MAX_LINES, DocumentRecord, Freshness,
+    MAX_SNIPPET_LINES, ReadOptions, ReadResult, SCHEMA_VERSION, SearchQuery, SearchResult,
+    SourceRecord, Store, SymbolRecord, is_corpus_name,
 };
 
 mod terms;
diff --git a/crates/codecite-core/src/store.rs b/crates/codecite-core/src/store.rs
--- a/crates/codecite-core/src/store.rs
+++ b/crates/codecite-core/src/store.rs
@@ -8,6 +8,9 @@
 use rusqlite::{Connection, ErrorCode, OpenFlags, OptionalExtension, TransactionBehavior};
 
 use crate::{Error, Result};
+
+mod read;
+pub use read::{DEFAULT_MAX_LINES, ReadOptions, ReadResult};
 
 mod search;
 pub use search::{DEFAULT_MAX_AGE_DAYS, Freshness, MAX_SNIPPET_LINES, SearchQuery, SearchResult};
@@ -22,6 +25,7 @@
     include_str!("../migrations/0001_initial.sql"),
     include_str!("../migrations/0002_incremental_index.sql"),
     include_str!("../migrations/0003_document_versions.sql"),
+    include_str!("../migrations/0004_read.sql"),
 ];
 
 /// The schema version this build creates and reads.
diff --git a/crates/codecite-core/src/store/read.rs b/crates/codecite-core/src/store/read.rs
--- a/crates/codecite-core/src/store/read.rs
+++ b/crates/codecite-core/src/store/read.rs
@@ -1,3 +1,600 @@
+//! Reading a citation: the exact text it points at, with the pinned
+//! citation that names it.
+
+use std::fmt;
+
+use rusqlite::{OptionalExtension, params};
+use serde::Serialize;
+
+use super::Store;
+use super::search::code_citation;
+use crate::{
+    Citation, CitationError, CodeCitation, Error, Result, SectionNumber, SpecAliases, SpecCitation,
+    SpecReference, SpecTarget,
+};
+
+/// How many lines `read` returns when the caller doesn't say.
+pub const DEFAULT_MAX_LINES: u32 = 80;
+
+/// The most candidates an error lists.
+const MAX_CANDIDATES: usize = 5;
+
+/// How to read a citation.
+#[derive(Debug, Clone, Copy, PartialEq, Eq)]
+pub struct ReadOptions {
+    /// The most lines of text to return; 0 reads as 1.
+    pub max_lines: u32,
+    /// Read an index older than its maximum age instead of refusing it.
+    pub allow_stale: bool,
+}
+
+impl Default for ReadOptions {
+    fn default() -> Self {
+        Self {
+            max_lines: DEFAULT_MAX_LINES,
+            allow_stale: false,
+        }
+    }
+}
+
+/// The text a citation points at, in the shape the CLI prints as JSON and
+/// the MCP server returns.
+#[derive(Debug, Clone, PartialEq, Serialize)]
+pub struct ReadResult {
+    /// The canonical citation of the text, always with its pinned version:
+    /// `repo@<full sha>:path#L10-L42`, or
+    /// `doc@sha256:<12 hex>#section-4.3[/step-2]`.
+    pub citation: String,
+    /// `code` for lines of a file, `section` or `step` for a spec.
+    pub kind: String,
+    /// A spec section's heading; a step has its section's.
+    #[serde(skip_serializing_if = "Option::is_none")]
+    pub title: Option<String>,
+    /// A spec section's or step's requirement keywords, such as `MUST`.
+    /// Code has none.
+    #[serde(skip_serializing_if = "Option::is_none")]
+    pub strength: Option<Vec<String>>,
+    /// How agents may use the source: `adapt`, `approach_only` or `quote`.
+    pub license_class: String,
+    /// When the source was last indexed.
+    pub indexed_at: String,
+    /// For code, whether the file differed from the cited commit when it
+    /// was indexed. Spec results have none.
+    #[serde(skip_serializing_if = "Option::is_none")]
+    pub dirty: Option<bool>,
+    /// The exact text, at most the requested number of lines.
+    pub text: String,
+    /// Whether the text stops before the end of what was cited.
+    pub truncated: bool,
+}
+
+impl fmt::Display for ReadResult {
+    /// The human form: the citation, a line saying what it is, a blank
+    /// line, and the text.
+    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
+        writeln!(f, "{}", self.citation)?;
+        write!(f, "{}", self.kind)?;
+        if let Some(title) = &self.title {
+            write!(f, " · {title}")?;
+        }
+        if let Some(strength) = self.strength.as_ref().filter(|s| !s.is_empty()) {
+            write!(f, " · {}", strength.join(", "))?;
+        }
+        write!(f, " · {} · indexed {}", self.license_class, self.indexed_at)?;
+        if self.dirty == Some(true) {
+            write!(f, " · dirty")?;
+        }
+        writeln!(f, "\n")?;
+        writeln!(f, "{}", self.text)?;
+        if self.truncated {
+            writeln!(f, "… (truncated)")?;
+        }
+        Ok(())
+    }
+}
+
+/// A source row, as `read` needs it.
+struct SourceRow {
+    id: i64,
+    name: String,
+    revision: String,
+    license_class: String,
+    indexed_at: String,
+}
+
+/// A spec section or step row.
+struct SpecChunk {
+    id: i64,
+    number: String,
+    title: Option<String>,
+    strength: Option<String>,
+    text: String,
+}
+
+impl Store {
+    /// Reads the text `citation` points at.
+    ///
+    /// `citation` is a canonical code or spec citation, or a spec citation
+    /// as people write it (`RFC 9449 §4.3`, `OIDC Core §3.1.3.7 step 13`, a
+    /// spec URL), with the corpus's document aliases. A spec citation that
+    /// names no version reads the current one, and one that names no
+    /// document reads the one current document that has its section and
+    /// step. The result's citation always names the pinned version.
+    ///
+    /// Anything that does not resolve to exactly one span is a typed error,
+    /// most of them with candidates: never a guess, and never an empty
+    /// success. An index older than its maximum age is
+    /// [`Error::StaleIndex`] unless `options.allow_stale` is set.
+    pub fn read(&self, citation: &str, options: &ReadOptions) -> Result<ReadResult> {
+        self.check_fresh(options.allow_stale)?;
+        let text = citation.trim();
+        let bad = |source| Error::BadCitation {
+            text: text.into(),
+            source,
+        };
+        match SpecReference::parse(text, &self.aliases()?) {
+            Ok(Some(reference)) => return self.read_reference(text, &reference, options),
+            Ok(None) => {}
+            Err(CitationError::UnknownDocument { found }) => {
+                return Err(Error::UnknownSource {
+                    name: found,
+                    known: self.source_names(true)?,
+                });
+            }
+            Err(e) => return Err(bad(e)),
+        }
+        match text.parse() {
+            Ok(Citation::Code(c)) => self.read_code(&c, options),
+            Ok(Citation::Spec(c)) => self.read_spec(&c, options),
+            Err(CitationError::MissingAt) => Err(Error::NotACitation { text: text.into() }),
+            Err(e) => Err(bad(e)),
+        }
+    }
+
+    /// The document aliases the last index run stored.
+    pub fn aliases(&self) -> Result<SpecAliases> {
+        let mut stmt = self.conn.prepare_cached("SELECT alias, name FROM alias")?;
+        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
+        let mut aliases = SpecAliases::new();
+        for row in rows {
+            let (alias, name): (String, String) = row?;
+            aliases.insert(&alias, &name);
+        }
+        Ok(aliases)
+    }
+
+    /// Reads lines of a code file.
+    fn read_code(&self, c: &CodeCitation, options: &ReadOptions) -> Result<ReadResult> {
+        let versions = self.sources(c.repo(), false)?;
+        if versions.is_empty() {
+            return Err(Error::UnknownSource {
+                name: c.repo().into(),
+                known: self.source_names(false)?,
+            });
+        }
+        // A citation may abbreviate the commit, as git does.
+        let Some(source) = versions
+            .iter()
+            .find(|s| s.revision.starts_with(c.commit().as_str()))
+        else {
+            return Err(Error::UnknownVersion {
+                name: c.repo().into(),
+                version: c.commit().to_string(),
+                known: versions
+                    .iter()
+                    .map(|s| format!("{}@{}", s.name, s.revision))
+                    .collect(),
+            });
+        };
+        let row: Option<(Option<String>, bool)> = self
+            .conn
+            .query_row(
+                "SELECT text, dirty FROM document WHERE source_id = ?1 AND path = ?2",
+                params![source.id, c.path()],
+                |row| Ok((row.get(0)?, row.get(1)?)),
+            )
+            .optional()?;
+        let Some((text, dirty)) = row else {
+            return Err(Error::NoSuchFile {
+                repo: format!("{}@{}", source.name, c.commit()),
+                path: c.path().into(),
+                candidates: self.same_file_name(source.id, c.path())?,
+            });
+        };
+        let text = text.ok_or_else(|| Error::InvalidCorpus {
+            path: self.path.clone(),
+            reason: format!("it holds no text for {}; re-run codecite index", c.path()),
+        })?;
+        let (start, end) = (c.lines().start(), c.lines().end());
+        let citation = code_citation(&source.name, &source.revision, c.path(), start, end);
+        let lines = file_lines(&text);
+        let count = u32::try_from(lines.len()).unwrap_or(u32::MAX);
+        if end > count {
+            return Err(Error::LinesPastEnd {
+                citation,
+                lines: count,
+            });
+        }
+        let cited = &lines[start as usize - 1..end as usize];
+        let (text, truncated) = cap(cited, options.max_lines);
+        Ok(ReadResult {
+            citation,
+            kind: "code".into(),
+            title: None,
+            strength: None,
+            license_class: source.license_class.clone(),
+            indexed_at: source.indexed_at.clone(),
+            dirty: Some(dirty),
+            text,
+            truncated,
+        })
+    }
+
+    /// Reads a canonical spec citation, at the version its hash names.
+    fn read_spec(&self, c: &SpecCitation, options: &ReadOptions) -> Result<ReadResult> {
+        let versions = self.sources(c.doc(), true)?;
+        if versions.is_empty() {
+            return Err(Error::UnknownSource {
+                name: c.doc().into(),
+                known: self.source_names(true)?,
+            });
+        }
+        let Some(source) = versions
+            .iter()
+            .find(|s| s.revision.starts_with(c.hash().as_str()))
+        else {
+            return Err(Error::UnknownVersion {
+                name: c.doc().into(),
+                version: format!("sha256:{}", c.hash()),
+                known: versions.iter().map(pinned).collect(),
+            });
+        };
+        let target = SpecTarget::Section(c.section().clone());
+        self.read_target(source, &target, c.step(), options)
+    }
+
+    /// Reads a spec citation as people write it.
+    fn read_reference(
+        &self,
+        text: &str,
+        r: &SpecReference,
+        options: &ReadOptions,
+    ) -> Result<ReadResult> {
+        let Some(doc) = r.doc() else {
+            // Only the written forms with `§` or `Section` omit the
+            // document, and they always name a section.
+            let SpecTarget::Section(section) = r.target() else {
+                return Err(Error::NotACitation { text: text.into() });
+            };
+            let source = self.only_document_with(text, section, r.step())?;
+            return self.read_target(&source, r.target(), r.step(), options);
+        };
+        let versions = self.sources(doc, true)?;
+        if versions.is_empty() {
+            return Err(Error::UnknownSource {
+                name: doc.into(),
+                known: self.source_names(true)?,
+            });
+        }
+        let current: Option<SourceRow> = self.current_version(doc)?;
+        let Some(source) = current else {
+            return Err(Error::UnknownVersion {
+                name: doc.into(),
+                version: "the current version".into(),
+                known: versions.iter().map(pinned).collect(),
+            });
+        };
+        self.read_target(&source, r.target(), r.step(), options)
+    }
+
+    /// Reads a section, or a step in it, of one document version.
+    fn read_target(
+        &self,
+        source: &SourceRow,
+        target: &SpecTarget,
+        step: Option<u32>,
+        options: &ReadOptions,
+    ) -> Result<ReadResult> {
+        let document = pinned(source);
+        let section = match target {
+            SpecTarget::Section(n) => self.section(source.id, "number", &n.to_string())?,
+            SpecTarget::Anchor(a) => self.section(source.id, "anchor", a)?,
+        };
+        let Some(section) = section else {
+            let (asked, candidates) = match target {
+                SpecTarget::Section(n) => (n.to_string(), self.nearest_sections(source, n)?),
+                SpecTarget::Anchor(a) => (format!("#{a}"), Vec::new()),
+            };
+            return Err(Error::NoSuchSection {
+                document,
+                section: asked,
+                candidates,
+            });
+        };
+        let section_citation = format!("{document}#section-{}", section.number);
+        let (chunk, citation) = match step {
+            None => (section, section_citation),
+            Some(k) => {
+                let Some(found) = self.step(section.id, k)? else {
+                    let mut candidates = self.nearest_steps(section.id, k, &section_citation)?;
+                    if candidates.is_empty() {
+                        candidates.push(section_citation.clone());
+                    }
+                    return Err(Error::NoSuchStep {
+                        section: section_citation,
+                        step: k,
+                        candidates,
+                    });
+                };
+                let citation = format!("{section_citation}/step-{k}");
+                // A step has its section's title.
+                let title = section.title;
+                (SpecChunk { title, ..found }, citation)
+            }
+        };
+        let lines: Vec<&str> = chunk.text.lines().collect();
+        let (text, truncated) = cap(&lines, options.max_lines);
+        Ok(ReadResult {
+            citation,
+            kind: if step.is_some() { "step" } else { "section" }.into(),
+            title: chunk.title,
+            strength: Some(
+                chunk
+                    .strength
+                    .map(|s| s.split(',').map(str::to_owned).collect())
+                    .unwrap_or_default(),
+            ),
+            license_class: source.license_class.clone(),
+            indexed_at: source.indexed_at.clone(),
+            dirty: None,
+            text,
+            truncated,
+        })
+    }
+
+    /// The one current document that has `section`, and `step` in it when
+    /// one is given.
+    ///
+    /// Several such documents are [`Error::AmbiguousDocument`] with each
+    /// one's citation, and so is none. When none has the step but exactly
+    /// one has the section, the error is [`Error::NoSuchStep`] in it.
+    fn only_document_with(
+        &self,
+        text: &str,
+        section: &SectionNumber,
+        step: Option<u32>,
+    ) -> Result<SourceRow> {
+        let number = section.to_string();
+        let mut stmt = self.conn.prepare_cached(
+            "SELECT s.id, s.name, s.revision, s.license_class, s.indexed_at, c.id
+             FROM source s
+             JOIN document d ON d.source_id = s.id
+             JOIN chunk c ON c.document_id = d.id
+             WHERE s.kind = 'document' AND s.is_current
+               AND c.kind = 'section' AND c.number = ?1
+             GROUP BY s.id
+             ORDER BY s.name",
+        )?;
+        let rows = stmt.query_map([&number], |row| {
+            Ok((
+                SourceRow {
+                    id: row.get(0)?,
+                    name: row.get(1)?,
+                    revision: row.get(2)?,
+                    license_class: row.get(3)?,
+                    indexed_at: row.get(4)?,
+                },
+                row.get::<_, i64>(5)?,
+            ))
+        })?;
+        let mut with_section: Vec<(SourceRow, i64)> = rows.collect::<rusqlite::Result<_>>()?;
+        let cited = |s: &SourceRow| match step {
+            Some(k) => format!("{}#section-{number}/step-{k}", pinned(s)),
+            None => format!("{}#section-{number}", pinned(s)),
+        };
+        // The indexes, in `with_section`, of the documents that have the
+        // step too.
+        let mut with_step = Vec::new();
+        for (i, (_, section_id)) in with_section.iter().enumerate() {
+            if step.map_or(Ok(true), |k| self.step(*section_id, k).map(|s| s.is_some()))? {
+                with_step.push(i);
+            }
+        }
+        match (with_step.as_slice(), with_section.len()) {
+            (&[i], _) => Ok(with_section.swap_remove(i).0),
+            // `read_target` reports the missing step, with the nearest ones.
+            ([], 1) => Ok(with_section.swap_remove(0).0),
+            ([], _) => Err(Error::AmbiguousDocument {
+                reference: text.into(),
+                candidates: with_section
+                    .iter()
+                    .map(|(s, _)| format!("{}#section-{number}", pinned(s)))
+                    .collect(),
+            }),
+            (several, _) => Err(Error::AmbiguousDocument {
+                reference: text.into(),
+                candidates: several.iter().map(|&i| cited(&with_section[i].0)).collect(),
+            }),
+        }
+    }
+
+    /// Every version of the source or document `name`, oldest row first.
+    fn sources(&self, name: &str, document: bool) -> Result<Vec<SourceRow>> {
+        let mut stmt = self.conn.prepare_cached(
+            "SELECT id, name, revision, license_class, indexed_at FROM source
+             WHERE name = ?1 AND (kind = 'document') = ?2 ORDER BY id",
+        )?;
+        let rows = stmt.query_map(params![name, document], source_row)?;
+        Ok(rows.collect::<rusqlite::Result<_>>()?)
+    }
+
+    /// The current version of the document `name`.
+    fn current_version(&self, name: &str) -> Result<Option<SourceRow>> {
+        Ok(self
+            .conn
+            .query_row(
+                "SELECT id, name, revision, license_class, indexed_at FROM source
+                 WHERE name = ?1 AND kind = 'document' AND is_current",
+                [name],
+                source_row,
+            )
+            .optional()?)
+    }
+
+    /// The names of the corpus's documents, or of its other sources, sorted.
+    fn source_names(&self, document: bool) -> Result<Vec<String>> {
+        let mut stmt = self.conn.prepare_cached(
+            "SELECT DISTINCT name FROM source WHERE (kind = 'document') = ?1 ORDER BY name",
+        )?;
+        let rows = stmt.query_map([document], |row| row.get(0))?;
+        Ok(rows.collect::<rusqlite::Result<_>>()?)
+    }
+
+    /// The section of a document version whose `column` (`number` or
+    /// `anchor`) is `value`.
+    fn section(&self, source_id: i64, column: &str, value: &str) -> Result<Option<SpecChunk>> {
+        Ok(self
+            .conn
+            .query_row(
+                &format!(
+                    "SELECT c.id, c.number, c.title, c.strength, c.text
+                     FROM chunk c JOIN document d ON d.id = c.document_id
+                     WHERE d.source_id = ?1 AND c.kind = 'section' AND c.{column} = ?2
+                     ORDER BY c.id LIMIT 1"
+                ),
+                params![source_id, value],
+                spec_chunk,
+            )
+            .optional()?)
+    }
+
+    /// Step `k` of the section chunk `section_id`.
+    fn step(&self, section_id: i64, k: u32) -> Result<Option<SpecChunk>> {
+        Ok(self
+            .conn
+            .query_row(
+                "SELECT id, number, title, strength, text FROM chunk
+                 WHERE parent_id = ?1 AND kind = 'step' AND number = ?2
+                 ORDER BY id LIMIT 1",
+                params![section_id, k.to_string()],
+                spec_chunk,
+            )
+            .optional()?)
+    }
+
+    /// The sections of a document version nearest to `wanted`: those that
+    /// share the most leading numbers with it, then the closest in the
+    /// first number that differs.
+    fn nearest_sections(&self, source: &SourceRow, wanted: &SectionNumber) -> Result<Vec<String>> {
+        let mut stmt = self.conn.prepare_cached(
+            "SELECT c.number FROM chunk c JOIN document d ON d.id = c.document_id
+             WHERE d.source_id = ?1 AND c.kind = 'section' ORDER BY c.id",
+        )?;
+        let numbers = stmt.query_map([source.id], |row| row.get::<_, String>(0))?;
+        let want = wanted.parts();
+        let mut scored = Vec::new();
+        for number in numbers {
+            let number = number?;
+            let Ok(parsed) = number.parse::<SectionNumber>() else {
+                continue;
+            };
+            let have = parsed.parts();
+            let shared = want.iter().zip(have).take_while(|(a, b)| a == b).count();
+            let distance = match (want.get(shared), have.get(shared)) {
+                (Some(a), Some(b)) => a.abs_diff(*b),
+                _ => 0,
+            };
+            scored.push((std::cmp::Reverse(shared), distance, have.len(), number));
+        }
+        // A stable sort keeps document order among equals.
+        scored.sort_by_key(|s| (s.0, s.1, s.2));
+        let document = pinned(source);
+        Ok(scored
+            .into_iter()
+            .take(MAX_CANDIDATES)
+            .map(|(.., number)| format!("{document}#section-{number}"))
+            .collect())
+    }
+
+    /// The steps of a section nearest to step `k`.
+    fn nearest_steps(&self, section_id: i64, k: u32, section: &str) -> Result<Vec<String>> {
+        let mut stmt = self.conn.prepare_cached(
+            "SELECT number FROM chunk WHERE parent_id = ?1 AND kind = 'step' ORDER BY id",
+        )?;
+        let numbers = stmt.query_map([section_id], |row| row.get::<_, String>(0))?;
+        let mut steps = Vec::new();
+        for number in numbers {
+            if let Ok(n) = number?.parse::<u32>() {
+                steps.push(n);
+            }
+        }
+        steps.sort_by_key(|&n| (n.abs_diff(k), n));
+        Ok(steps
+            .into_iter()
+            .take(MAX_CANDIDATES)
+            .map(|n| format!("{section}/step-{n}"))
+            .collect())
+    }
+
+    /// Indexed paths in the source with the same file name as `path`.
+    fn same_file_name(&self, source_id: i64, path: &str) -> Result<Vec<String>> {
+        let name = path.rsplit('/').next().unwrap_or(path);
+        let mut stmt = self.conn.prepare_cached(
+            "SELECT path FROM document WHERE source_id = ?1
+               AND (path = ?2 OR substr(path, -length(?2) - 1) = '/' || ?2)
+             ORDER BY path",
+        )?;
+        let rows = stmt.query_map(params![source_id, name], |row| row.get(0))?;
+        Ok(rows.take(MAX_CANDIDATES).collect::<rusqlite::Result<_>>()?)
+    }
+}
+
+fn source_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SourceRow> {
+    Ok(SourceRow {
+        id: row.get(0)?,
+        name: row.get(1)?,
+        revision: row.get(2)?,
+        license_class: row.get(3)?,
+        indexed_at: row.get(4)?,
+    })
+}
+
+fn spec_chunk(row: &rusqlite::Row<'_>) -> rusqlite::Result<SpecChunk> {
+    Ok(SpecChunk {
+        id: row.get(0)?,
+        number: row.get(1)?,
+        title: row.get(2)?,
+        strength: row.get(3)?,
+        text: row.get(4)?,
+    })
+}
+
+/// A document version as citations name it: `rfc9449@sha256:1a2b3c4d5e6f`.
+fn pinned(source: &SourceRow) -> String {
+    let hash = source.revision.get(..12).unwrap_or(&source.revision);
+    format!("{}@sha256:{hash}", source.name)
+}
+
+/// A file's lines as the chunker counts them: split at `\n`, a final
+/// newline ending the last line rather than starting an empty one, and a
+/// `\r` before a `\n` dropped.
+fn file_lines(text: &str) -> Vec<&str> {
+    let text = text.strip_suffix('\n').unwrap_or(text);
+    if text.is_empty() {
+        return Vec::new();
+    }
+    text.split('\n')
+        .map(|line| line.strip_suffix('\r').unwrap_or(line))
+        .collect()
+}
+
+/// The first `max_lines` of `lines` joined, and whether any were left out.
+fn cap(lines: &[&str], max_lines: u32) -> (String, bool) {
+    let max = usize::try_from(max_lines.max(1)).unwrap_or(usize::MAX);
+    let kept = &lines[..lines.len().min(max)];
+    (kept.join("\n"), kept.len() < lines.len())
+}
+
 #[cfg(test)]
 mod tests {
     use super::*;
diff --git a/crates/codecite-core/src/store/search.rs b/crates/codecite-core/src/store/search.rs
--- a/crates/codecite-core/src/store/search.rs
+++ b/crates/codecite-core/src/store/search.rs
@@ -130,14 +130,7 @@
     /// `query.allow_stale` is set.
     pub fn search(&self, query: &SearchQuery<'_>) -> Result<Vec<SearchResult>> {
         let fts = fts_query(query.text).ok_or(Error::EmptyQuery)?;
-        let freshness = self.freshness()?;
-        if freshness.is_stale() && !query.allow_stale {
-            return Err(Error::StaleIndex {
-                corpus: self.corpus_name(),
-                indexed_at: freshness.indexed_at,
-                max_age_days: freshness.max_age_days,
-            });
-        }
+        self.check_fresh(query.allow_stale)?;
 
         let (w_symbol, w_path, w_text) = COLUMN_WEIGHTS;
         let mut stmt = self.conn.prepare_cached(
@@ -202,6 +195,20 @@
         Ok(rows.collect::<rusqlite::Result<_>>()?)
     }
 
+    /// Refuses an index older than its maximum age with
+    /// [`Error::StaleIndex`], unless `allow_stale` is set.
+    pub(super) fn check_fresh(&self, allow_stale: bool) -> Result<()> {
+        let freshness = self.freshness()?;
+        if freshness.is_stale() && !allow_stale {
+            return Err(Error::StaleIndex {
+                corpus: self.corpus_name(),
+                indexed_at: freshness.indexed_at,
+                max_age_days: freshness.max_age_days,
+            });
+        }
+        Ok(())
+    }
+
     /// The corpus's name: its file's name without `.db`.
     fn corpus_name(&self) -> String {
         self.path
@@ -235,7 +242,13 @@
 }
 
 /// `repo@sha:path#L10-L42`, or `#L10` for one line.
-fn code_citation(source: &str, revision: &str, path: &str, start: u32, end: u32) -> String {
+pub(super) fn code_citation(
+    source: &str,
+    revision: &str,
+    path: &str,
+    start: u32,
+    end: u32,
+) -> String {
     if start == end {
         format!("{source}@{revision}:{path}#L{start}")
     } else {
diff --git a/crates/codecite-core/src/store/write.rs b/crates/codecite-core/src/store/write.rs
--- a/crates/codecite-core/src/store/write.rs
+++ b/crates/codecite-core/src/store/write.rs
@@ -6,7 +6,7 @@
 use rusqlite::{Transaction, params};
 
 use super::Store;
-use crate::{Keyword, Result, identifier_terms, text_terms};
+use crate::{Keyword, Result, SpecAliases, identifier_terms, text_terms};
 
 /// A source to create or update. A document is matched by its name and
 /// revision, since each pinned version is its own source; any other source
@@ -47,6 +47,9 @@
     pub size_bytes: u64,
     /// Whether the file differs from the source's revision.
     pub dirty: bool,
+    /// The file's text, which `read` cites lines of. `None` for a spec,
+    /// whose sections and steps are its text.
+    pub text: Option<&'a str>,
     /// The chunks, in order.
     pub chunks: Vec<ChunkRecord<'a>>,
 }
@@ -149,11 +152,11 @@
     ) -> Result<usize> {
         let tx = self.conn.transaction()?;
         let document_id: i64 = tx.query_row(
-            "INSERT INTO document (source_id, path, language, content_hash, size_bytes, dirty)
-             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
+            "INSERT INTO document (source_id, path, language, content_hash, size_bytes, dirty, text)
+             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
              ON CONFLICT (source_id, path) DO UPDATE SET
                language = excluded.language, content_hash = excluded.content_hash,
-               size_bytes = excluded.size_bytes, dirty = excluded.dirty
+               size_bytes = excluded.size_bytes, dirty = excluded.dirty, text = excluded.text
              RETURNING id",
             params![
                 source_id,
@@ -163,6 +166,7 @@
                 // SQLite integers are i64; no file comes near i64::MAX bytes.
                 i64::try_from(document.size_bytes).unwrap_or(i64::MAX),
                 document.dirty,
+                document.text,
             ],
             |row| row.get(0),
         )?;
@@ -230,6 +234,20 @@
              ON CONFLICT (key) DO UPDATE SET value = excluded.value",
             [max_age_days.to_string()],
         )?;
+        Ok(())
+    }
+
+    /// Replaces the stored document aliases with `aliases`, so `read` can
+    /// resolve `OIDC Core §3.1` without the manifest.
+    pub fn set_aliases(&mut self, aliases: &SpecAliases) -> Result<()> {
+        let tx = self.conn.transaction()?;
+        tx.execute("DELETE FROM alias", [])?;
+        let mut insert = tx.prepare("INSERT INTO alias (alias, name) VALUES (?1, ?2)")?;
+        for (alias, name) in aliases.iter() {
+            insert.execute([alias, name])?;
+        }
+        drop(insert);
+        tx.commit()?;
         Ok(())
     }
 }
diff --git a/crates/codecite-index/src/error.rs b/crates/codecite-index/src/error.rs
--- a/crates/codecite-index/src/error.rs
+++ b/crates/codecite-index/src/error.rs
@@ -44,6 +44,18 @@
     DuplicateSource {
         /// The shared name.
         name: String,
+    },
+
+    /// One alias names two documents, so a citation using it would be a
+    /// guess.
+    #[error("alias {alias:?} names both `{first}` and `{second}`; keep it on one")]
+    DuplicateAlias {
+        /// The alias as the manifest writes it.
+        alias: String,
+        /// The document it names first.
+        first: String,
+        /// The other document it names.
+        second: String,
     },
 
     /// A document source's bytes are not UTF-8, so they can't be HTML a
diff --git a/crates/codecite-index/src/index.rs b/crates/codecite-index/src/index.rs
--- a/crates/codecite-index/src/index.rs
+++ b/crates/codecite-index/src/index.rs
@@ -8,7 +8,7 @@
 use std::path::Path;
 use std::time::{Duration, Instant};
 
-use codecite_core::{ChunkRecord, DocumentRecord, SourceRecord, Store, SymbolRecord};
+use codecite_core::{ChunkRecord, DocumentRecord, SourceRecord, SpecAliases, Store, SymbolRecord};
 use sha2::{Digest, Sha256};
 
 use crate::{
@@ -76,6 +76,7 @@
             documents.push((doc, fetched.bytes));
         }
     }
+    let aliases = document_aliases(manifest)?;
     let mut chunker = Chunker::new()?;
     let mut indexed: Vec<String> = Vec::new();
     let mut keep: Vec<i64> = Vec::new();
@@ -131,9 +132,37 @@
         keep.push(index_document(store, doc, bytes, &mut summary)?);
     }
     summary.removed_sources = store.delete_sources_except(&keep)?;
+    store.set_aliases(&aliases)?;
     store.set_indexed_at(manifest.max_age_days)?;
     summary.elapsed = started.elapsed();
     Ok(summary)
+}
+
+/// The aliases of every document the run indexes. An alias that names two
+/// documents is [`Error::DuplicateAlias`], found before anything is written.
+fn document_aliases(manifest: &Manifest) -> Result<SpecAliases> {
+    let mut aliases = SpecAliases::new();
+    for source in &manifest.sources {
+        let Source::Document(doc) = source else {
+            continue;
+        };
+        if doc.license_class == LicenseClass::Excluded {
+            continue;
+        }
+        for alias in &doc.aliases {
+            match aliases.get(alias) {
+                Some(first) if first != doc.name => {
+                    return Err(Error::DuplicateAlias {
+                        alias: alias.clone(),
+                        first: first.into(),
+                        second: doc.name.clone(),
+                    });
+                }
+                _ => aliases.insert(alias, &doc.name),
+            }
+        }
+    }
+    Ok(aliases)
 }
 
 /// The manifest's `name`, else the checkout directory's name.
@@ -208,6 +237,7 @@
             content_hash: hash.as_slice(),
             size_bytes: text.len() as u64,
             dirty: file.dirty,
+            text: Some(&text),
             chunks: chunks
                 .iter()
                 .zip(symbol_ids)
@@ -295,6 +325,8 @@
         content_hash: hash.as_slice(),
         size_bytes: bytes.len() as u64,
         dirty: false,
+        // A spec's text is its sections and steps.
+        text: None,
         chunks: spec_chunks(&spec, &step_numbers),
     };
     summary.chunks += store.write_document(source_id, &document)?;
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p codecite-core && cargo test -p codecite-index`
Expected: core 127 passed; index 116 passed, 6 ignored.

- [ ] **Step 5: Commit**

```bash
git add crates/codecite-core/migrations/0004_read.sql crates/codecite-core/src/store/read.rs crates/codecite-core/src/citation/written.rs crates/codecite-core/src/error.rs crates/codecite-core/src/lib.rs crates/codecite-core/src/store.rs crates/codecite-core/src/store/search.rs crates/codecite-core/src/store/write.rs crates/codecite-index/src/error.rs crates/codecite-index/src/index.rs
git commit -m "feat(core): read code and spec citations from the store (#27)" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: `codecite read` (`codecite` binary)

**Files:**
- Create: `crates/codecite/tests/read.rs`
- Modify: `crates/codecite/src/main.rs`

**Interfaces:**
- Consumes: `Store::open_corpus`, `Store::read`, `ReadOptions`, `ReadResult`, `DEFAULT_MAX_LINES` from Task 1.
- Produces: the `Read` subcommand, `write_read(&mut impl Write, &ReadResult, json)`, and `to_stdout`, which `search` now uses too.

- [ ] **Step 1: Write the failing tests**

These add the argument and output tests to `main.rs`, plus an end-to-end test that indexes a real git checkout, searches it, and reads back the citations search printed:

```diff
diff --git a/crates/codecite/src/main.rs b/crates/codecite/src/main.rs
--- a/crates/codecite/src/main.rs
+++ b/crates/codecite/src/main.rs
@@ -347,6 +347,80 @@
     }
 
     #[test]
+    fn read_takes_a_corpus_a_citation_and_a_line_cap() {
+        assert!(Cli::try_parse_from(["codecite", "read", "identity"]).is_err());
+        let cli = Cli::try_parse_from(["codecite", "read", "identity", "RFC 9449 §4.3"]).unwrap();
+        let Command::Read {
+            corpus,
+            citation,
+            max_lines,
+            json,
+            allow_stale,
+        } = cli.command
+        else {
+            panic!("not read");
+        };
+        assert_eq!(
+            (corpus.as_str(), citation.as_str()),
+            ("identity", "RFC 9449 §4.3")
+        );
+        assert_eq!(max_lines, 80);
+        assert!(!json && !allow_stale);
+        let cli = Cli::try_parse_from([
+            "codecite",
+            "read",
+            "identity",
+            "§4.3",
+            "--max-lines",
+            "5",
+            "--json",
+            "--allow-stale",
+        ])
+        .unwrap();
+        let Command::Read {
+            max_lines,
+            json,
+            allow_stale,
+            ..
+        } = cli.command
+        else {
+            panic!("not read");
+        };
+        assert_eq!(max_lines, 5);
+        assert!(json && allow_stale);
+        let zero = ["codecite", "read", "identity", "§4.3", "--max-lines", "0"];
+        assert!(Cli::try_parse_from(zero).is_err());
+    }
+
+    #[test]
+    fn the_read_form_escapes_control_characters() {
+        let result = ReadResult {
+            citation: "app@0123456:a.md#L1-L2".into(),
+            kind: "code".into(),
+            title: None,
+            strength: None,
+            license_class: "adapt".into(),
+            indexed_at: "2026-10-20T06:00:00Z".into(),
+            dirty: Some(false),
+            text: "# Title\u{1b}]52;c;ZWNobyBoaQ==\u{7}\n\tend".into(),
+            truncated: false,
+        };
+        let mut out = Vec::new();
+        write_read(&mut out, &result, false).unwrap();
+        assert_eq!(
+            String::from_utf8(out).unwrap(),
+            "app@0123456:a.md#L1-L2
+code · adapt · indexed 2026-10-20T06:00:00Z
+
+# Title\\u{1b}]52;c;ZWNobyBoaQ==\\u{7}
+\tend
+"
+        );
+        let err = write_read(&mut ClosedPipe, &result, true).unwrap_err();
+        assert_eq!(err.kind(), std::io::ErrorKind::BrokenPipe);
+    }
+
+    #[test]
     fn the_human_form_shows_rank_citation_and_snippet() {
         let result = SearchResult {
             citation: "app@0123456:src/lib.rs#L1-L5".into(),
diff --git a/crates/codecite/tests/read.rs b/crates/codecite/tests/read.rs
new file mode 100644
--- /dev/null
+++ b/crates/codecite/tests/read.rs
@@ -0,0 +1,173 @@
+//! `codecite read` end to end: a corpus indexed from a real git checkout
+//! under a temporary `XDG_DATA_HOME`, searched, and its citations read
+//! back. Linux only, since other platforms ignore `XDG_DATA_HOME`.
+#![cfg(target_os = "linux")]
+
+use std::fs;
+use std::path::Path;
+use std::process::{Command, Output};
+
+/// The one indexed file: five lines, the third a lone `}` that no chunk
+/// holds.
+const LIB: &str =
+    "pub fn validate(token: &str) -> bool {\n    !token.is_empty()\n}\n\npub fn check() {}\n";
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
+/// A corpus `demo` indexed from a one-file checkout.
+struct Demo {
+    tmp: tempfile::TempDir,
+}
+
+impl Demo {
+    fn new() -> Self {
+        let tmp = tempfile::tempdir().unwrap();
+        let repo = tmp.path().join("app");
+        fs::create_dir_all(repo.join("src")).unwrap();
+        git(&repo, &["init", "-q"]);
+        fs::write(repo.join("src/lib.rs"), LIB).unwrap();
+        git(&repo, &["add", "."]);
+        git(&repo, &["commit", "-q", "-m", "init"]);
+
+        let work = tmp.path().join("work");
+        fs::create_dir_all(work.join("corpora")).unwrap();
+        fs::write(
+            work.join("corpora/demo.toml"),
+            format!(
+                "name = \"demo\"\n\n[[source]]\nkind = \"local\"\npath = {:?}\nuse = \"adapt\"\n",
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
+    /// Runs the binary, expects success and parses its JSON output.
+    fn json(&self, args: &[&str]) -> serde_json::Value {
+        let output = self.run(args);
+        assert!(output.status.success(), "{}", stderr(&output));
+        serde_json::from_slice(&output.stdout).unwrap()
+    }
+}
+
+fn stderr(output: &Output) -> String {
+    String::from_utf8_lossy(&output.stderr).into_owned()
+}
+
+#[test]
+fn read_returns_what_a_search_result_cites() {
+    let demo = Demo::new();
+    let results = demo.json(&["search", "demo", "validate", "--json"]);
+    let citation = results[0]["citation"].as_str().unwrap();
+    assert!(citation.ends_with(":src/lib.rs#L1-L3"), "{citation}");
+    let read = demo.json(&["read", "demo", citation, "--json"]);
+    assert_eq!(read["citation"], citation);
+    assert_eq!(read["kind"], "code");
+    assert_eq!(read["dirty"], false);
+    assert_eq!(read["truncated"], false);
+    assert_eq!(
+        read["text"],
+        "pub fn validate(token: &str) -> bool {\n    !token.is_empty()\n}"
+    );
+}
+
+#[test]
+fn the_human_form_and_the_line_cap() {
+    let demo = Demo::new();
+    let results = demo.json(&["search", "demo", "validate", "--json"]);
+    let citation = results[0]["citation"].as_str().unwrap();
+    // Lines 1 to 5, as an abbreviated commit cites them.
+    let (repo, rest) = citation.split_once('@').unwrap();
+    let short = format!("{repo}@{}:src/lib.rs#L1-L5", &rest[..7]);
+    let output = demo.run(&["read", "demo", &short, "--max-lines", "3"]);
+    assert!(output.status.success(), "{}", stderr(&output));
+    let text = String::from_utf8(output.stdout).unwrap();
+    let lines: Vec<&str> = text.lines().collect();
+    assert!(
+        lines[0].starts_with("app@") && lines[0].ends_with("#L1-L5"),
+        "{text}"
+    );
+    assert!(lines[1].starts_with("code · adapt · indexed "), "{text}");
+    assert_eq!(
+        &lines[2..],
+        [
+            "",
+            "pub fn validate(token: &str) -> bool {",
+            "    !token.is_empty()",
+            "}",
+            "… (truncated)"
+        ]
+    );
+}
+
+#[test]
+fn errors_name_the_problem() {
+    let demo = Demo::new();
+    let results = demo.json(&["search", "demo", "validate", "--json"]);
+    let citation = results[0]["citation"].as_str().unwrap();
+    let past_end = citation.replace("#L1-L3", "#L4-L9");
+    let cases = [
+        (
+            vec!["read", "demo", past_end.as_str()],
+            "runs past the end of the file, which has 5 lines".to_owned(),
+        ),
+        (
+            vec!["read", "demo", "hello world"],
+            "\"hello world\" is not a citation".to_owned(),
+        ),
+        (
+            vec!["read", "demo", "RFC 9449 §4.3"],
+            "the corpus has no source or document `rfc9449`; no candidates".to_owned(),
+        ),
+        (
+            vec!["read", "demo", "RFC 9449 §04.3"],
+            "section \"04.3\" is not dot-separated numbers".to_owned(),
+        ),
+        (
+            vec!["read", "dmeo", "§4.3"],
+            "unknown corpus `dmeo`; known corpora: demo".to_owned(),
+        ),
+    ];
+    for (args, want) in cases {
+        let output = demo.run(&args);
+        assert!(!output.status.success(), "{args:?}");
+        assert!(
+            stderr(&output).contains(&want),
+            "{args:?}: {}",
+            stderr(&output)
+        );
+    }
+}
```

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test -p codecite`
Expected: compile errors `no variant named Read found for enum Command`, `cannot find function write_read`, and `cannot find struct ReadResult` (5 errors).

- [ ] **Step 3: Add the subcommand**

> **Learn:** `to_stdout` takes `impl FnOnce(&mut StdoutLock<'static>) -> io::Result<()>`, a closure called once with stdout's lock, so `search` and `read` share the rule that a closed pipe (`| head`) is not a failure. `impl Trait` in argument position is a generic parameter you don't name.

```diff
diff --git a/crates/codecite/src/main.rs b/crates/codecite/src/main.rs
--- a/crates/codecite/src/main.rs
+++ b/crates/codecite/src/main.rs
@@ -5,7 +5,9 @@
 
 use anyhow::{Context as _, bail};
 use clap::{Parser, Subcommand};
-use codecite_core::{SearchQuery, SearchResult, Store, is_corpus_name};
+use codecite_core::{
+    DEFAULT_MAX_LINES, ReadOptions, ReadResult, SearchQuery, SearchResult, Store, is_corpus_name,
+};
 use codecite_index::{Fetcher, LicenseClass, Manifest, Source, WalkOptions, index_corpus};
 
 /// Command-line arguments.
@@ -50,6 +52,23 @@
         #[arg(long)]
         json: bool,
         /// Search an index older than its manifest's `max_age_days`.
+        #[arg(long)]
+        allow_stale: bool,
+    },
+    /// Print the exact text a citation points at, with its pinned citation.
+    Read {
+        /// The corpus to read from.
+        corpus: String,
+        /// A code or spec citation, as search prints it, as a spec URL, or
+        /// as people write it: `RFC 9449 §4.3`, `OIDC Core §3.1.3.7 step 13`.
+        citation: String,
+        /// The most lines of text to print.
+        #[arg(long, default_value_t = DEFAULT_MAX_LINES, value_parser = clap::value_parser!(u32).range(1..))]
+        max_lines: u32,
+        /// Print the result as a JSON object.
+        #[arg(long)]
+        json: bool,
+        /// Read an index older than its manifest's `max_age_days`.
         #[arg(long)]
         allow_stale: bool,
     },
@@ -80,6 +99,19 @@
             };
             search(&corpus, &query, json)
         }
+        Command::Read {
+            corpus,
+            citation,
+            max_lines,
+            json,
+            allow_stale,
+        } => {
+            let options = ReadOptions {
+                max_lines,
+                allow_stale,
+            };
+            read(&corpus, &citation, &options, json)
+        }
     }
 }
 
@@ -137,11 +169,45 @@
     let dirs = project_dirs()?;
     let store = Store::open_corpus(dirs.data_dir(), corpus)?;
     let results = store.search(query)?;
-    match write_results(&mut std::io::stdout().lock(), &results, json) {
-        // A reader that stops early, like `| head`, is not a failure.
+    to_stdout(|out| write_results(out, &results, json))
+}
+
+/// `codecite read`: opens the corpus read-only and prints the cited text,
+/// as JSON or in its human form.
+fn read(corpus: &str, citation: &str, options: &ReadOptions, json: bool) -> anyhow::Result<()> {
+    let dirs = project_dirs()?;
+    let store = Store::open_corpus(dirs.data_dir(), corpus)?;
+    let result = store.read(citation, options)?;
+    to_stdout(|out| write_read(out, &result, json))
+}
+
+/// Runs `write` on stdout. A reader that stops early, like `| head`, is
+/// not a failure.
+fn to_stdout(
+    write: impl FnOnce(&mut std::io::StdoutLock<'static>) -> std::io::Result<()>,
+) -> anyhow::Result<()> {
+    match write(&mut std::io::stdout().lock()) {
         Err(err) if err.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
-        written => written.context("writing the results"),
-    }
+        written => written.context("writing to stdout"),
+    }
+}
+
+/// Writes a read result to `out`: as JSON, or its `Display` form with
+/// control characters escaped line by line.
+fn write_read(
+    out: &mut impl std::io::Write,
+    result: &ReadResult,
+    json: bool,
+) -> std::io::Result<()> {
+    if json {
+        serde_json::to_writer_pretty(&mut *out, result)?;
+        writeln!(out)?;
+    } else {
+        for line in result.to_string().lines() {
+            writeln!(out, "{}", escape_controls(line))?;
+        }
+    }
+    out.flush()
 }
 
 /// Writes the results to `out`. Unlike `println!`, which panics when stdout
```

- [ ] **Step 4: Run the gates**

Run: `cargo fmt --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked && cargo deny check`
Expected: all pass (271 passed, 6 ignored; `advisories ok, bans ok, licenses ok, sources ok`).

- [ ] **Step 5: Exercise the binary by hand**

Run `cargo run -q -- read <corpus> '<citation>'` against a corpus of your own, then once piped into `head -c 20`, which exits 0.

- [ ] **Step 6: Read a real corpus through the CLI**

`/var/tmp/read-proto/real-run.sh <worktree>` serves the seven real spec pages locally and indexes them with this repo. Core and Back-Channel Logout get aliases. It then reads written, URL, bare and canonical citations, the error cases, and a code citation that search printed. The prototype's output, with long lines cut:

```text
skipped: 1 ignored
elapsed: 1.31s
(exit 0)
$ codecite read real OIDC Core §3.1.3.7 step 13
openid-connect-core-1_0@sha256:4d016752a645#section-3.1.3.7/step-13
step · ID Token Validation · SHOULD · quote · indexed 2026-10-20T19:15:31Z

13. If the auth_time Claim was requested, either through a specific request for this Claim or by using the max_age parameter, the Client SHOULD check the auth_time Claim value and request re-authentic
(exit 0)
$ codecite read real Core 3.1.3.7(13) --json
{
  "citation": "openid-connect-core-1_0@sha256:4d016752a645#section-3.1.3.7/step-13",
  "kind": "step",
  "title": "ID Token Validation",
  "strength": [
    "SHOULD"
  ],
  "license_class": "quote",
  "indexed_at": "2026-10-20T19:15:31Z",
  "text": "13. If the auth_time Claim was requested, either through a specific request for this Claim or by using the max_age parameter, the Client SHOULD check the auth_time Claim value and request r
  "truncated": false
}
(exit 0)
$ codecite read real RFC 9449 §4.3 --max-lines 6
rfc9449@sha256:2d8657f07291#section-4.3
section · Checking DPoP Proofs · MUST, SHOULD · quote · indexed 2026-10-20T19:15:32Z

4.3. Checking DPoP Proofs

To validate a DPoP proof, the receiving server MUST ensure the following:

1. There is not more than one DPoP HTTP request header field.
2. The DPoP HTTP request header field value is a single and well-formed JWT.
… (truncated)
(exit 0)
$ codecite read real RFC9449 Section 4.3 step 12
rfc9449@sha256:2d8657f07291#section-4.3/step-12
step · Checking DPoP Proofs · quote · indexed 2026-10-20T19:15:32Z

12. If presented to a protected resource in conjunction with an access token,

- ensure that the value of the ath claim equals the hash of that access token, and
- confirm that the public key to which the access token is bound matches the public key from the DPoP proof.
(exit 0)
$ codecite read real https://openid.net/specs/openid-connect-core-1_0.html#IDTokenValidation --max-lines 4
openid-connect-core-1_0@sha256:4d016752a645#section-3.1.3.7
section · ID Token Validation · MUST, SHOULD, MAY · quote · indexed 2026-10-20T19:15:31Z

3.1.3.7. ID Token Validation

Clients MUST validate the ID Token in the Token Response in the following manner:

… (truncated)
(exit 0)
$ codecite read real https://www.rfc-editor.org/rfc/rfc9449.html#section-4.3 --max-lines 2
rfc9449@sha256:2d8657f07291#section-4.3
section · Checking DPoP Proofs · MUST, SHOULD · quote · indexed 2026-10-20T19:15:32Z

4.3. Checking DPoP Proofs

… (truncated)
(exit 0)
$ codecite read real §3.1.3.7 step 12 --max-lines 3
openid-connect-core-1_0@sha256:4d016752a645#section-3.1.3.7/step-12
step · ID Token Validation · SHOULD · quote · indexed 2026-10-20T19:15:31Z

12. If the acr Claim was requested, the Client SHOULD check that the asserted Claim Value is appropriate. The meaning and processing of acr Claim Values is out of scope for this specification.
(exit 0)
$ codecite read real Back-Channel Logout 1.0 §2.6 step 2
openid-connect-backchannel-1_0@sha256:3b5c933d5269#section-2.6/step-2
step · Logout Token Validation · quote · indexed 2026-10-20T19:15:31Z

2. Validate the Logout Token signature in the same way that an ID Token signature is validated, with the following refinements.
(exit 0)
$ codecite read real §1
Error: §1 names no document and is not in exactly one current document; name one: openid-connect-backchannel-1_0@sha256:3b5c933d5269#section-1, openid-connect-core-1_0@sha256:4d016752a645#section-1, 
(exit 1)
$ codecite read real §4.3
Error: §4.3 names no document and is not in exactly one current document; name one: rfc6749@sha256:535362fa3b4c#section-4.3, rfc9449@sha256:2d8657f07291#section-4.3, rfc9700@sha256:c00e2234fcd7#secti
(exit 1)
$ codecite read real RFC 9449 §4.3 step 14
Error: rfc9449@sha256:2d8657f07291#section-4.3 has no step 14; nearest: rfc9449@sha256:2d8657f07291#section-4.3/step-12, rfc9449@sha256:2d8657f07291#section-4.3/step-11, rfc9449@sha256:2d8657f07291#se
(exit 1)
$ codecite read real RFC 9449 §4.9
Error: rfc9449@sha256:2d8657f07291 has no section 4.9; nearest: rfc9449@sha256:2d8657f07291#section-4, rfc9449@sha256:2d8657f07291#section-4.3, rfc9449@sha256:2d8657f07291#section-4.2, rfc9449@sha256:
(exit 1)
$ codecite read real §3.1.3.7 step 99
Error: openid-connect-core-1_0@sha256:4d016752a645#section-3.1.3.7 has no step 99; nearest: openid-connect-core-1_0@sha256:4d016752a645#section-3.1.3.7/step-13, openid-connect-core-1_0@sha256:4d016752
(exit 1)
$ codecite read real RFC 6749 §2.3.1 --max-lines 3
rfc6749@sha256:535362fa3b4c#section-2.3.1
section · Client Password · MUST, MUST NOT, REQUIRED, SHOULD, NOT RECOMMENDED, MAY · quote · indexed 2026-10-20T19:15:31Z

2.3.1.  Client Password

   Clients in possession of a client password MAY use the HTTP Basic
… (truncated)
(exit 0)
$ codecite read real RFC 7636 §4.1
Error: the corpus has no source or document `rfc7636`; known: openid-connect-backchannel-1_0, openid-connect-core-1_0, rfc6749, rfc8252, rfc9126, rfc9449, rfc9700
(exit 1)
$ codecite read real Foo §3.1
Error: the corpus has no source or document `Foo`; known: openid-connect-backchannel-1_0, openid-connect-core-1_0, rfc6749, rfc8252, rfc9126, rfc9449, rfc9700
(exit 1)
$ codecite read real §3.1.3.7 steps 4-6
Error: invalid citation "§3.1.3.7 steps 4-6"

Caused by:
    unexpected "steps 4-6" after the citation
(exit 1)
$ codecite read real hello
Error: "hello" is not a citation; write repo@sha:path#L10-L42, doc@sha256:<hash>#section-4.3, a spec URL, or a form like RFC 9449 §4.3
(exit 1)
$ codecite read real codecite@078fa4171a7a24acba8f824a184ff73b0c9e0df4:crates/codecite-core/src/store/write.rs#L100-L134 --max-lines 5
codecite@078fa4171a7a24acba8f824a184ff73b0c9e0df4:crates/codecite-core/src/store/write.rs#L100-L134
code · adapt · indexed 2026-10-20T19:15:31Z · dirty

    pub fn upsert_source(&mut self, source: &SourceRecord<'_>) -> Result<i64> {
        // An upsert names the unique index it conflicts on, and a partial
        // index is named by its columns plus its `WHERE`.
        let conflict = if source.kind == "document" {
            "(name, revision) WHERE kind = 'document'"
… (truncated)
(exit 0)
$ codecite read real codecite@078fa4171a7a24acba8f824a184ff73b0c9e0df4:crates/codecite-core/src/store/write.rs#L1-L999999
Error: codecite@078fa4171a7a24acba8f824a184ff73b0c9e0df4:crates/codecite-core/src/store/write.rs#L1-L999999 runs past the end of the file, which has 784 lines
(exit 1)
$ codecite read real rfc9449@sha256:2d8657f07291#section-4.3 --max-lines 1
rfc9449@sha256:2d8657f07291#section-4.3
section · Checking DPoP Proofs · MUST, SHOULD · quote · indexed 2026-10-20T19:15:32Z

4.3. Checking DPoP Proofs
… (truncated)
(exit 0)
$ codecite read real rfc9449@sha256:02d8657f07291#section-4.3
Error: invalid citation "rfc9449@sha256:02d8657f07291#section-4.3"

Caused by:
    content hash "02d8657f07291" is not 12 lowercase hex digits
(exit 1)
$ codecite read real rfc9449@sha256:000000000000#section-4.3
Error: `rfc9449` is not indexed at sha256:000000000000; indexed: rfc9449@sha256:2d8657f07291
(exit 1)
rfc9449@sha256:2d865
(head exit 0)
```

- [ ] **Step 7: Commit**

```bash
git add crates/codecite/src/main.rs crates/codecite/tests/read.rs
git commit -m "feat: print cited text with codecite read (#27)" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: Describe `read` in the design

**Files:**
- Modify: `docs/design.md`, `README.md`

- [ ] **Step 1: Update the schema, the CLI row, the read rules and the README status**

````diff
diff --git a/docs/design.md b/docs/design.md
--- a/docs/design.md
+++ b/docs/design.md
@@ -95,8 +95,11 @@
   id INTEGER PRIMARY KEY, source_id INTEGER NOT NULL REFERENCES source(id) ON DELETE CASCADE,
   path TEXT NOT NULL, language TEXT, content_hash BLOB NOT NULL, size_bytes INTEGER,  -- content_hash: sha256 of the bytes
   dirty INTEGER NOT NULL DEFAULT 0,                                 -- a local file that differed from HEAD
+  text TEXT,                                                        -- a code file's text, which read cites lines of; NULL for a spec
   UNIQUE (source_id, path)
 );
+
+CREATE TABLE alias (alias TEXT PRIMARY KEY, name TEXT NOT NULL);    -- "oidc core" -> openid-connect-core-1_0, from the manifest
 
 CREATE TABLE chunk (
   id INTEGER PRIMARY KEY, document_id INTEGER NOT NULL REFERENCES document(id) ON DELETE CASCADE,
@@ -144,6 +147,7 @@
 ```
 
 - **Document versions** are separate sources: each pinned version of a document is its own `source` row, with the document's name, its `sha256` as `revision`, and `is_current`, `obsoleted_by` and `updated_by` from the manifest. A version is unique by name and revision, any other source by name. Results name a version as citations do, `rfc9449@sha256:1a2b3c4d5e6f`.
+- **A query needs no manifest.** An index run stores what queries need from the manifest in the corpus: `max_age_days` in `meta`, and each document's `aliases` in `alias`, normalized (ASCII lowercase, whitespace collapsed) and replaced on every run. One alias naming two documents stops the run before anything is written. A code file's whole text is kept in `document.text`, because chunks leave out the lines with no letter or digit between definitions and `read` must return the exact lines cited.
 - **Deletes cascade.** Every connection that writes turns `PRAGMA foreign_keys` on, so deleting a source deletes its documents, and deleting a document its chunks, symbols and the edges it is evidence for; the trigger takes each chunk's full-text row with it. Migrations run with the keys off, since rebuilding a table to change its keys drops the old one, and check every key (`PRAGMA foreign_key_check`) before they commit.
 
 - **Symbol IDs** are SCIP symbol strings, so precise indexers drop in without remapping. Symbols found only by tree-sitter get SCIP-shaped local IDs, `codecite . <source> . <path segments>/<descriptors>` (e.g. `` codecite . identity-model . src/`client.rs`/Client#validate(). ``), with a SCIP disambiguator on a later same-named function in the file (`new(+1).`); they are replaced when a SCIP index for the same source arrives.
@@ -270,7 +274,7 @@
 | `codecite def <symbol>` | Where a symbol is defined |
 | `codecite refs <symbol>` | Every reference to a symbol |
 | `codecite graph <callers\|callees\|impact\|path> ...` | Graph queries with `--min-confidence` |
-| `codecite read <citation>` | Print a cited span, capped in lines |
+| `codecite read <corpus> <citation> [--max-lines <n>] [--json] [--allow-stale]` | The exact text a citation points at, at most `--max-lines` lines (default 80), under its pinned citation. `--json` prints the result object; otherwise the citation, a line naming its kind, title, strength, license class and `indexed_at`, a blank line and the text |
 | `codecite cite <citation> [--claim <text>]` | Check a spec citation and the claim beside it |
 | `codecite corpora` | List corpora with age, size and staleness |
 | `codecite mcp` | Run the stdio MCP server |
@@ -305,8 +309,9 @@
 - Search results also carry the chunk's `kind`; a code result carries `dirty`, and a spec result its pinned spec citation and no `symbol`. Fields with no value are left out, and graph fields (`providers`, `confidence`) appear only on graph results.
 - Snippets are capped at 40 lines; agents call `read` for more, which keeps token use predictable.
 - Code citations look like `repo@sha:path#L10-L42`; spec citations like `rfc9449@sha256:1a2b3c4d5e6f#section-4.3` and `openid-connect-core-1_0@sha256:…#section-3.1.3.7/step-13`. The canonical forms parse strictly, so that each span has exactly one spelling: a commit is 7 to 64 lowercase hex digits, a spec hash is 12, numbers have no leading zeros, and one line is `#L10`, not `#L10-L10`. `read` and `cite` also accept the forms people write (`RFC 9449 §4.3`, `OIDC Core §3.1.3.7 step 13`, `Core 3.1.3.7(13)`, a spec URL with its anchor). These parse to an unresolved reference (a document or none, a section number or the document's own anchor, and an optional step), with document aliases taken from the manifest. A string that matches no form is no match rather than an error, and one string holds one citation: step ranges such as `steps 4-6` and lists such as `§2 / §3.1.3.7` are errors. A bare `§` resolves only when exactly one current document has that section and step.
+- `read` returns `citation`, `kind` (`code`, `section` or `step`), `text`, `truncated`, `license_class` and `indexed_at`; a spec result adds `title` and `strength` (a list of keywords), a code result `dirty`. Its citation always names the pinned version: a spec citation without one reads the current version, and a code citation's commit, which may be abbreviated, comes back in full. A step has its section's title. A code citation reads lines of the file's stored text, counted as the chunker counts them, and one that runs past the end of the file is an error naming its length. A citation that is not in the corpus is a typed error (`codecite_core::Error`) with candidates, as pinned citations, never the nearest guess: `UnknownSource` (the corpus's sources or documents), `UnknownVersion` (the versions indexed), `AmbiguousDocument` (each current document with the bare §, and when none has its step but one has the section, the missing step is reported in that one), `NoSuchSection` (the sections sharing the most leading numbers with it, then the closest), `NoSuchStep` (the closest step numbers, or the section when it has none) and `NoSuchFile` (indexed files with the same file name). Text that matches no citation form is `NotACitation`, and a malformed one is `BadCitation` with the parse error. `read` refuses a stale index as `search` does.
 - `cite` findings: `no_such_section`, `no_such_step`, `anchor_label_mismatch`, `strength_mismatch`, `superseded_document`, `topic_mismatch` (only when exactly one sibling step or section holds the claim's protocol term) and `ambiguous_document`. codecite supplies facts; the agent judges meaning.
-- Errors are explicit: an unknown corpus (with the corpora that exist), a stale index, an empty query or an ambiguous symbol name returns a typed error with candidates, never an empty success. A search that matches nothing is an empty list.
+- Errors are explicit: an unknown corpus (with the corpora that exist), a stale index, an empty query, a citation that is not in the corpus or an ambiguous symbol name returns a typed error with candidates, never an empty success. A search that matches nothing is an empty list.
 
 ## Security and privacy
 
diff --git a/README.md b/README.md
--- a/README.md
+++ b/README.md
@@ -19,7 +19,10 @@
 for pinning. `codecite search <corpus> "<query>"` prints ranked, cited results
 (`--json` for JSON; `--language`, `--path <glob>` and `--limit` filter them),
 and refuses an index older than the manifest's `max_age_days` unless
-`--allow-stale`. The MCP server comes next.
+`--allow-stale`. `codecite read <corpus> <citation>` prints the exact text a
+citation points at under its pinned citation, for the citations search prints
+and the spec forms people write (`RFC 9449 §4.3`, `OIDC Core §3.1.3.7 step 13`,
+a spec URL). The MCP server comes next.
 
 ## Planned
 
````

- [ ] **Step 2: Commit**

```bash
git add docs/design.md README.md
git commit -m "docs: describe codecite read and what the corpus stores for it (#27)" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```
