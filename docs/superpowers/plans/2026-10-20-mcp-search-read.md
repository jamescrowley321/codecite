# MCP `search` and `read` Tools Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement #32: the MCP server gains `search` and `read` tools with the inputs in `docs/design.md#interfaces`, each returning the same JSON as `codecite search --json` and `codecite read --json`. Typed errors (unknown corpus, stale index, ambiguous citation with candidates) are tool errors with structured content, and a `read` of a `quote` source carries the instruction to quote it exactly.

**Architecture:** `codecite-core` gains what both front ends share: `Error::kind` (the variant in snake_case) and `Error::candidates` (its list), a `NoCorpusNamed` variant and `Store::open_corpus_or_only` for a call that names no corpus, `SearchResults` (`{"results": [...]}`, since MCP structured content must be an object), `JsonSchema` on `SearchResult` and `ReadResult`, and `ReadResult::instruction`, set for a `quote` source. `codecite-mcp` gains the `search` and `read` tools: typed `Parameters` structs (`deny_unknown_fields`, `NonZeroU32` limits with the CLI's defaults), a `blocking` helper that runs each store call on tokio's blocking pool and maps a `codecite_core::Error` to `Json<ToolError>`, which rmcp sends as `isError: true` with structured content `{"error": {"kind", "message", "candidates"}}`. The CLI's `search --json` and `read --json` print `serde_json::Value::to_string` of the same values, as `corpora` already does, so CLI stdout is the tool's text byte for byte.

**Tech Stack:** Rust 1.99.1 (edition 2024), rmcp 3.6, tokio 1, schemars 1. `codecite-mcp` adds the workspace's `schemars` and `serde` (already in the lock through `codecite-core`); no new crates, and `cargo deny check` passes unchanged. Prototyped: all gates pass (307 passed, 7 ignored), and the real run below drives `codecite mcp` against the identity corpus.

**Spec:** Issue #32; `docs/design.md#interfaces` (the `search` and `read` tools and commands, the result shapes, explicit errors). Decisions the issue and the design left open (DEC-023 in the loop's journal): both tools take `corpus?` and `allow_stale` beside the design's inputs; an omitted corpus is the only one, and none or several is `NoCorpusNamed` (never a cross-corpus guess); `search --json` changes from a bare array to `{"results": [...]}`.

## Global Constraints

- One JSON per operation: `SearchResults` and `ReadResult` are the result types of both the CLI and the tools, and the tools' output schemas are generated from them. The tool text and the CLI's `--json` stdout are both `serde_json::Value::to_string` (compact, keys sorted); the CLI adds a newline.
- `search` inputs: `query`, `corpus?`, `language?`, `path?`, `limit` (default 10, at least 1), `allow_stale` (default false). `read` inputs: `citation`, `corpus?`, `max_lines` (default 80, at least 1), `allow_stale` (default false). Unknown fields are refused. Arguments that don't deserialize are rmcp's tool error `failed to deserialize parameters: …`, before any corpus is opened.
- An omitted `corpus` is the data directory's only corpus. With none or several, the error is `NoCorpusNamed` listing them: "name a corpus; known corpora: a, b", or with none the `UnknownCorpus` tail "no corpus has been indexed yet; run codecite index".
- A tool error's structured content is `{"error": {"kind", "message", "candidates"?}}`: `kind` from `Error::kind`, `message` the error and its causes joined by `: ` (the CLI's message, with `BadCitation`'s parse error), `candidates` from `Error::candidates`, left out when empty. A panicked or cancelled query is kind `internal`.
- A `read` result from a `quote` source has `instruction`: "Quote this text exactly, with its citation; never paraphrase it." (`QUOTE_INSTRUCTION`). Other classes have none, and the field is left out.
- Schema descriptions come from doc comments, which agents read: no rustdoc link brackets in the fields of `SearchResult`, `ReadResult` or the params.
- `#[tool]` reads the output schema from a literal `Result<Json<T>, _>` return type, so the tools spell it out instead of using the `ToolResult` alias.
- The workspace forbids `unsafe_code` and warns on `missing_docs`; every public item gets a doc comment.
- Gates: `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo deny check`.

## Review Focus

- `codecite-mcp` `server::tests`: an in-process client on a `tokio::io::duplex` pipe calls each tool and each error case: `unknown_corpus`, `empty_query`, `ambiguous_document` with both candidates, `not_a_citation`, `bad_citation` with its cause, `stale_index` for both tools (and `allow_stale`), `no_corpus_named` with two corpora, `io` for an unreadable data directory, and bad arguments (`limit: 0`, `-1`, a missing `query`, `max_lines: 0`, an unknown field).
- `crates/codecite/tests/mcp.rs`: the real binary over stdio, each tool's text compared byte for byte with the CLI's stdout for the same query (corpora, three searches, a spec and a code read), and a `no_such_section` tool error whose message the CLI prints.
- The `search --json` shape change (`[...]` → `{"results": [...]}`) and the compact output of `search --json` and `read --json`; the CLI tests that parse them.
- `spawn_blocking` closures own their inputs (`move`), and the store is opened inside them, once per call.

---

### Task 1: Error kinds, an unnamed corpus, `SearchResults` and the quote instruction (`codecite-core`)

**Files:**
- Modify: `crates/codecite-core/src/error.rs`, `crates/codecite-core/src/store.rs`, `crates/codecite-core/src/store/search.rs`, `crates/codecite-core/src/store/read.rs`, `crates/codecite-core/src/lib.rs`
- Modify: `crates/codecite/src/main.rs` (the one `ReadResult` literal in its tests gains `instruction: None`)

> **Learn: exhaustive `match` inside the crate.** `Error` is `#[non_exhaustive]`, which only binds other crates: they must add a `_` arm. Inside `codecite-core`, `Error::kind` matches every variant with no wildcard, so adding a variant without a kind is a compile error here rather than a silent `"unknown"` in an agent's JSON. `Error::candidates` uses or-patterns (`A { known, .. } | B { known } => known`): each alternative must bind the same names with the same types.

- [ ] **Step 1: Write the failing tests.** Apply only the test hunks of the diff below: in `error.rs` `no_corpus_named_lists_the_corpora_to_choose_from` and `each_error_has_a_kind_and_its_candidates`, in `store.rs` `an_unnamed_corpus_is_the_only_one`, and in `read.rs` the `instruction` assertions and the `"instruction"` key in `json_and_human_forms`. Run `cargo test -p codecite-core`: it fails to compile (`NoCorpusNamed`, `kind`, `candidates`, `open_corpus_or_only`, `instruction` and `QUOTE_INSTRUCTION` don't exist).
- [ ] **Step 2: Implement.** Apply the rest of the diff.

`````diff
diff --git a/crates/codecite-core/src/error.rs b/crates/codecite-core/src/error.rs
index 2b87207..203bd42 100644
--- a/crates/codecite-core/src/error.rs
+++ b/crates/codecite-core/src/error.rs
@@ -58,6 +58,14 @@ pub enum Error {
         known: Vec<String>,
     },
 
+    /// The caller named no corpus, and the data directory does not hold
+    /// exactly one to use instead.
+    #[error("{}", no_corpus_named(known))]
+    NoCorpusNamed {
+        /// The corpora that do exist, sorted.
+        known: Vec<String>,
+    },
+
     /// The corpus was last indexed longer ago than its manifest's
     /// `max_age_days` allows.
     #[error(
@@ -189,6 +197,51 @@ pub enum Error {
     },
 }
 
+impl Error {
+    /// The variant's name in snake_case, such as `unknown_corpus`: what an
+    /// MCP client matches on, since the message may change.
+    pub fn kind(&self) -> &'static str {
+        match self {
+            Error::Io(_) => "io",
+            Error::InvalidCorpus { .. } => "invalid_corpus",
+            Error::Sqlite(_) => "sqlite",
+            Error::SchemaTooNew { .. } => "schema_too_new",
+            Error::InvalidCorpusName { .. } => "invalid_corpus_name",
+            Error::UnknownCorpus { .. } => "unknown_corpus",
+            Error::NoCorpusNamed { .. } => "no_corpus_named",
+            Error::StaleIndex { .. } => "stale_index",
+            Error::EmptyQuery => "empty_query",
+            Error::NotACitation { .. } => "not_a_citation",
+            Error::BadCitation { .. } => "bad_citation",
+            Error::UnknownSource { .. } => "unknown_source",
+            Error::UnknownVersion { .. } => "unknown_version",
+            Error::AmbiguousDocument { .. } => "ambiguous_document",
+            Error::NoSuchSection { .. } => "no_such_section",
+            Error::UncitableSection { .. } => "uncitable_section",
+            Error::NoSuchStep { .. } => "no_such_step",
+            Error::NoSuchFile { .. } => "no_such_file",
+            Error::LinesPastEnd { .. } => "lines_past_end",
+        }
+    }
+
+    /// What the caller could have meant instead: the corpora, names,
+    /// versions or pinned citations the error lists. Empty for an error
+    /// that lists none.
+    pub fn candidates(&self) -> &[String] {
+        match self {
+            Error::UnknownCorpus { known, .. }
+            | Error::NoCorpusNamed { known }
+            | Error::UnknownSource { known, .. }
+            | Error::UnknownVersion { known, .. } => known,
+            Error::AmbiguousDocument { candidates, .. }
+            | Error::NoSuchSection { candidates, .. }
+            | Error::NoSuchStep { candidates, .. }
+            | Error::NoSuchFile { candidates, .. } => candidates,
+            _ => &[],
+        }
+    }
+}
+
 /// The end of an error message that lists candidates: `nearest: a, b`, or
 /// `no candidates`. Candidates come from indexed text, such as file paths,
 /// so their control characters are escaped before they reach a terminal.
@@ -234,6 +287,15 @@ fn known_corpora(known: &[String]) -> String {
     }
 }
 
+/// The message of an [`Error::NoCorpusNamed`].
+fn no_corpus_named(known: &[String]) -> String {
+    if known.is_empty() {
+        known_corpora(known)
+    } else {
+        format!("name a corpus; {}", known_corpora(known))
+    }
+}
+
 /// `Result` with [`Error`] as the default error type.
 pub type Result<T, E = Error> = std::result::Result<T, E>;
 
@@ -308,6 +370,48 @@ mod tests {
         );
     }
 
+    #[test]
+    fn no_corpus_named_lists_the_corpora_to_choose_from() {
+        let err = Error::NoCorpusNamed {
+            known: vec!["identity".into(), "specs".into()],
+        };
+        assert_eq!(
+            err.to_string(),
+            "name a corpus; known corpora: identity, specs"
+        );
+        assert_eq!(err.kind(), "no_corpus_named");
+        assert_eq!(err.candidates(), ["identity", "specs"]);
+        let err = Error::NoCorpusNamed { known: vec![] };
+        assert_eq!(
+            err.to_string(),
+            "no corpus has been indexed yet; run codecite index"
+        );
+    }
+
+    #[test]
+    fn each_error_has_a_kind_and_its_candidates() {
+        let err = Error::NoSuchSection {
+            document: "rfc9449@sha256:1a2b3c4d5e6f".into(),
+            section: "4.9".into(),
+            candidates: vec!["rfc9449@sha256:1a2b3c4d5e6f#section-4.3".into()],
+        };
+        assert_eq!(err.kind(), "no_such_section");
+        assert_eq!(
+            err.candidates(),
+            ["rfc9449@sha256:1a2b3c4d5e6f#section-4.3"]
+        );
+        let err = Error::StaleIndex {
+            corpus: "demo".into(),
+            indexed_at: "2026-01-01T00:00:00Z".into(),
+            max_age_days: 7,
+        };
+        assert_eq!(err.kind(), "stale_index");
+        assert!(err.candidates().is_empty());
+        assert_eq!(Error::EmptyQuery.kind(), "empty_query");
+        let err = Error::from(io::Error::other("disk on fire"));
+        assert_eq!(err.kind(), "io");
+    }
+
     #[test]
     fn stale_index_names_the_age_limit() {
         let err = Error::StaleIndex {
diff --git a/crates/codecite-core/src/lib.rs b/crates/codecite-core/src/lib.rs
index 5744efc..1e346c9 100644
--- a/crates/codecite-core/src/lib.rs
+++ b/crates/codecite-core/src/lib.rs
@@ -20,8 +20,9 @@ pub use keyword::{Keyword, UnknownKeyword, requirement_keywords};
 mod store;
 pub use store::{
     ChunkRecord, Corpora, CorpusInfo, DEFAULT_MAX_AGE_DAYS, DEFAULT_MAX_LINES, DocumentRecord,
-    Freshness, MAX_SNIPPET_LINES, ReadOptions, ReadResult, SCHEMA_VERSION, SearchQuery,
-    SearchResult, SourceRecord, Store, SymbolRecord, corpora, is_corpus_name,
+    Freshness, MAX_SNIPPET_LINES, QUOTE_INSTRUCTION, ReadOptions, ReadResult, SCHEMA_VERSION,
+    SearchQuery, SearchResult, SearchResults, SourceRecord, Store, SymbolRecord, corpora,
+    is_corpus_name,
 };
 
 mod terms;
diff --git a/crates/codecite-core/src/store.rs b/crates/codecite-core/src/store.rs
index 8c36933..69c32d7 100644
--- a/crates/codecite-core/src/store.rs
+++ b/crates/codecite-core/src/store.rs
@@ -13,10 +13,12 @@ mod corpora;
 pub use corpora::{Corpora, CorpusInfo, corpora};
 
 mod read;
-pub use read::{DEFAULT_MAX_LINES, ReadOptions, ReadResult};
+pub use read::{DEFAULT_MAX_LINES, QUOTE_INSTRUCTION, ReadOptions, ReadResult};
 
 mod search;
-pub use search::{DEFAULT_MAX_AGE_DAYS, Freshness, MAX_SNIPPET_LINES, SearchQuery, SearchResult};
+pub use search::{
+    DEFAULT_MAX_AGE_DAYS, Freshness, MAX_SNIPPET_LINES, SearchQuery, SearchResult, SearchResults,
+};
 
 mod write;
 pub use write::{ChunkRecord, DocumentRecord, SourceRecord, SymbolRecord};
@@ -178,6 +180,21 @@ impl Store {
         Store::open_read_only(&path)
     }
 
+    /// Opens `corpus` as [`Store::open_corpus`] does or, when it is `None`,
+    /// the one corpus `dir` holds. With none or several to choose from,
+    /// that is [`Error::NoCorpusNamed`], listing them: never a guess.
+    pub fn open_corpus_or_only(dir: &Path, corpus: Option<&str>) -> Result<Store> {
+        if let Some(corpus) = corpus {
+            return Store::open_corpus(dir, corpus);
+        }
+        match known_corpora(dir)?.as_slice() {
+            [only] => Store::open_corpus(dir, only),
+            known => Err(Error::NoCorpusNamed {
+                known: known.to_vec(),
+            }),
+        }
+    }
+
     /// The schema version stored in the file.
     pub fn schema_version(&self) -> Result<u32> {
         read_version(&self.conn, &self.path)
@@ -792,6 +809,36 @@ mod tests {
         fs::remove_dir_all(&dir).unwrap();
     }
 
+    #[test]
+    fn an_unnamed_corpus_is_the_only_one() {
+        let dir = scratch("only-corpus");
+        let err = Store::open_corpus_or_only(&dir.join("missing"), None)
+            .err()
+            .unwrap();
+        assert!(
+            matches!(&err, Error::NoCorpusNamed { known } if known.is_empty()),
+            "{err:?}"
+        );
+        drop(Store::open_read_write(&dir.join("specs.db")).unwrap());
+        let store = Store::open_corpus_or_only(&dir, None).unwrap();
+        assert_eq!(store.path(), dir.join("specs.db"));
+        drop(store);
+        drop(Store::open_read_write(&dir.join("identity.db")).unwrap());
+        let err = Store::open_corpus_or_only(&dir, None).err().unwrap();
+        assert!(
+            matches!(&err, Error::NoCorpusNamed { known } if known == &["identity", "specs"]),
+            "{err:?}"
+        );
+        let store = Store::open_corpus_or_only(&dir, Some("identity")).unwrap();
+        assert_eq!(store.path(), dir.join("identity.db"));
+        drop(store);
+        let err = Store::open_corpus_or_only(&dir, Some("identty"))
+            .err()
+            .unwrap();
+        assert!(matches!(err, Error::UnknownCorpus { .. }), "{err:?}");
+        fs::remove_dir_all(&dir).unwrap();
+    }
+
     #[test]
     fn every_migration_file_is_registered() {
         let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/migrations");
diff --git a/crates/codecite-core/src/store/read.rs b/crates/codecite-core/src/store/read.rs
index 5b921a4..d5e1ecb 100644
--- a/crates/codecite-core/src/store/read.rs
+++ b/crates/codecite-core/src/store/read.rs
@@ -4,6 +4,7 @@
 use std::fmt;
 
 use rusqlite::{OptionalExtension, params};
+use schemars::JsonSchema;
 use serde::Serialize;
 
 use super::Store;
@@ -37,9 +38,13 @@ impl Default for ReadOptions {
     }
 }
 
+/// What a `quote` read tells the agent to do with its text.
+pub const QUOTE_INSTRUCTION: &str =
+    "Quote this text exactly, with its citation; never paraphrase it.";
+
 /// The text a citation points at, in the shape the CLI prints as JSON and
 /// the MCP server returns.
-#[derive(Debug, Clone, PartialEq, Serialize)]
+#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
 pub struct ReadResult {
     /// The canonical citation of the text, always with its pinned version:
     /// `repo@<full sha>:path#L10-L42`, or
@@ -66,6 +71,11 @@ pub struct ReadResult {
     pub text: String,
     /// Whether the text stops before the end of what was cited.
     pub truncated: bool,
+    /// How the license lets the agent use the text, when it says more
+    /// than `license_class` alone: a `quote` source's text is
+    /// [`QUOTE_INSTRUCTION`].
+    #[serde(skip_serializing_if = "Option::is_none")]
+    pub instruction: Option<String>,
 }
 
 impl fmt::Display for ReadResult {
@@ -229,6 +239,7 @@ impl Store {
             dirty: Some(dirty),
             text,
             truncated,
+            instruction: instruction(&source.license_class),
         })
     }
 
@@ -359,6 +370,7 @@ impl Store {
             dirty: None,
             text,
             truncated,
+            instruction: instruction(&source.license_class),
         })
     }
 
@@ -598,6 +610,11 @@ fn file_lines(text: &str) -> Vec<&str> {
         .collect()
 }
 
+/// The instruction a result from a source of `license_class` carries.
+fn instruction(license_class: &str) -> Option<String> {
+    (license_class == "quote").then(|| QUOTE_INSTRUCTION.into())
+}
+
 /// The first `max_lines` of `lines` joined, and whether any were left out.
 fn cap(lines: &[&str], max_lines: u32) -> (String, bool) {
     let max = usize::try_from(max_lines.max(1)).unwrap_or(usize::MAX);
@@ -811,6 +828,7 @@ mod tests {
         assert_eq!(r.text, "    check();\n}");
         assert_eq!((r.dirty, r.truncated), (Some(true), false));
         assert_eq!(r.license_class, "adapt");
+        assert_eq!(r.instruction, None);
         assert_eq!((r.title, r.strength), (None, None));
         let r = f.ok(&format!("app@{HEAD}:src/lib.rs#L5"));
         assert_eq!(r.text, "pub fn check() {}");
@@ -888,6 +906,8 @@ mod tests {
         assert!(r.text.starts_with("4.3.  Checking DPoP Proofs\n"));
         assert_eq!(r.strength, Some(vec![]));
         assert_eq!((r.license_class.as_str(), r.dirty), ("quote", None));
+        // A quote source's text comes with the instruction to quote it.
+        assert_eq!(r.instruction.as_deref(), Some(QUOTE_INSTRUCTION));
         let r = f.ok("rfc9449@sha256:1a2b3c4d5e6f#section-4.3/step-12");
         assert_eq!(
             r.citation,
@@ -1088,6 +1108,7 @@ mod tests {
             [
                 "citation",
                 "indexed_at",
+                "instruction",
                 "kind",
                 "license_class",
                 "strength",
@@ -1099,6 +1120,7 @@ mod tests {
         let code = serde_json::to_value(f.ok("app@0123456:src/lib.rs#L1")).unwrap();
         assert_eq!(code["dirty"], true);
         assert!(code.get("strength").is_none());
+        assert!(code.get("instruction").is_none());
 
         let mut r = f.ok("Core 3.1.3.7(13)");
         r.indexed_at = "2026-10-20T06:00:00Z".into();
diff --git a/crates/codecite-core/src/store/search.rs b/crates/codecite-core/src/store/search.rs
index 4f61170..adcb0f1 100644
--- a/crates/codecite-core/src/store/search.rs
+++ b/crates/codecite-core/src/store/search.rs
@@ -2,6 +2,7 @@
 //! name first, and each result cited.
 
 use rusqlite::{OptionalExtension, params};
+use schemars::JsonSchema;
 use serde::Serialize;
 
 use super::Store;
@@ -35,9 +36,18 @@ pub struct SearchQuery<'a> {
     pub allow_stale: bool,
 }
 
+/// The results of a search, best first: what `codecite search --json`
+/// prints and the MCP `search` tool returns, as the same JSON. An MCP
+/// tool's structured result must be an object, so the list is a field.
+#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
+pub struct SearchResults {
+    /// The ranked results; none when nothing matched.
+    pub results: Vec<SearchResult>,
+}
+
 /// One ranked result, in the shape the CLI prints as JSON and the MCP
 /// server returns.
-#[derive(Debug, Clone, PartialEq, Serialize)]
+#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
 pub struct SearchResult {
     /// Where the chunk is: `repo@sha:path#L10-L42`, or
     /// `doc@sha256:<12 hex>#section-4.3[/step-2]` for a spec.
diff --git a/crates/codecite/src/main.rs b/crates/codecite/src/main.rs
index 260c81e..fe8b14c 100644
--- a/crates/codecite/src/main.rs
+++ b/crates/codecite/src/main.rs
@@ -522,6 +522,7 @@ mod tests {
             dirty: Some(false),
             text: "# Title\u{1b}]52;c;ZWNobyBoaQ==\u{7}\n\tend".into(),
             truncated: false,
+            instruction: None,
         };
         let mut out = Vec::new();
         write_read(&mut out, &result, false).unwrap();
`````

- [ ] **Step 3: Run the tests.** `cargo test -p codecite-core` → 137 passed. Then `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test -p codecite --bin codecite`.
- [ ] **Step 4: Commit.** `git add crates/codecite-core crates/codecite/src/main.rs` then `git commit -m "feat(core): name error kinds and candidates, default to the only corpus, and mark quote text (#32)"`.

### Task 2: The `search` and `read` tools, and the CLI JSON they share

**Files:**
- Modify: `crates/codecite-mcp/Cargo.toml` (`schemars`, `serde`), `Cargo.lock`
- Modify: `crates/codecite-mcp/src/server.rs`, `crates/codecite-mcp/src/lib.rs`
- Modify: `crates/codecite/src/main.rs`, `crates/codecite/tests/mcp.rs`, `crates/codecite/tests/search.rs`, `crates/codecite/tests/read.rs`

> **Learn: sharing sync library code with an async server (`spawn_blocking`).** The store is plain synchronous code: a SQLite query blocks its thread until it returns. The server runs on a current-thread tokio runtime, so a blocking call inside an `async fn` would stall every other message, a cancellation included, until it returned. `tokio::task::spawn_blocking(f)` runs `f` on a separate pool of threads meant for blocking work and gives back a `JoinHandle` to `.await`. Because `f` runs on another thread, perhaps after the calling future is dropped, it must be `Send + 'static`: it owns everything it uses, which is why the tools `move` the data directory and the parameters into the closure and open the `Store` inside it, and why `SearchQuery`'s borrows point into the moved `params`. The `JoinHandle` yields `Err(JoinError)` if `f` panicked, so `blocking` has three outcomes: the value, a `codecite_core::Error`, or an `internal` error. The library stays synchronous and testable on its own; only the server crate knows about tokio.

> **Learn: errors as values in a protocol.** rmcp's `IntoCallToolResult` for `Result<T, E>` sends `Err(e)` as a successful JSON-RPC response whose result has `isError: true`, so an agent reads the error and its candidates instead of the call failing. Returning `Json<ToolError>` in the `Err` puts the error in the result's structured content, with the same JSON as text.

- [ ] **Step 1: Write the failing tests.** Apply the test hunks of the diff below: `server.rs`'s `mod tests` (replacing the old one), `crates/codecite/tests/mcp.rs`, the `json` helpers in `tests/search.rs` and `tests/read.rs`, and the `a_closed_stdout_is_an_error_not_a_panic` change in `main.rs`. Run `cargo test -p codecite-mcp -p codecite`: it fails to compile (`ReadParams`, `SearchParams`, `ToolError` and `write_results(_, Vec<_>, _)` don't exist).
- [ ] **Step 2: Implement.** Apply the rest of the diff (the Cargo.toml hunk first).

`````diff
diff --git a/crates/codecite-core/src/store/read.rs b/crates/codecite-core/src/store/read.rs
index d5e1ecb..311a8ff 100644
--- a/crates/codecite-core/src/store/read.rs
+++ b/crates/codecite-core/src/store/read.rs
@@ -71,9 +71,9 @@ pub struct ReadResult {
     pub text: String,
     /// Whether the text stops before the end of what was cited.
     pub truncated: bool,
-    /// How the license lets the agent use the text, when it says more
-    /// than `license_class` alone: a `quote` source's text is
-    /// [`QUOTE_INSTRUCTION`].
+    /// What the license asks of the agent, when it asks more than
+    /// `license_class` says: a `quote` source's text is to be quoted
+    /// exactly, with its citation, and never paraphrased.
     #[serde(skip_serializing_if = "Option::is_none")]
     pub instruction: Option<String>,
 }
diff --git a/crates/codecite-core/src/store/search.rs b/crates/codecite-core/src/store/search.rs
index adcb0f1..386737b 100644
--- a/crates/codecite-core/src/store/search.rs
+++ b/crates/codecite-core/src/store/search.rs
@@ -67,7 +67,8 @@ pub struct SearchResult {
     /// was indexed. Spec results have none.
     #[serde(skip_serializing_if = "Option::is_none")]
     pub dirty: Option<bool>,
-    /// The chunk's first [`MAX_SNIPPET_LINES`] lines.
+    /// The chunk's first 40 lines (`MAX_SNIPPET_LINES`); `read` its
+    /// citation for the rest.
     pub snippet: String,
 }
 
diff --git a/crates/codecite-mcp/Cargo.toml b/crates/codecite-mcp/Cargo.toml
index 922479b..1615c2b 100644
--- a/crates/codecite-mcp/Cargo.toml
+++ b/crates/codecite-mcp/Cargo.toml
@@ -16,6 +16,8 @@ workspace = true
 [dependencies]
 codecite-core.workspace = true
 rmcp.workspace = true
+schemars.workspace = true
+serde.workspace = true
 thiserror.workspace = true
 tokio.workspace = true
 
diff --git a/crates/codecite-mcp/src/lib.rs b/crates/codecite-mcp/src/lib.rs
index e3f02b1..79e7f0b 100644
--- a/crates/codecite-mcp/src/lib.rs
+++ b/crates/codecite-mcp/src/lib.rs
@@ -4,4 +4,4 @@ mod error;
 pub use error::{Error, Result};
 
 mod server;
-pub use server::{Server, serve_stdio};
+pub use server::{ErrorInfo, ReadParams, SearchParams, Server, ToolError, serve_stdio};
diff --git a/crates/codecite-mcp/src/server.rs b/crates/codecite-mcp/src/server.rs
index 9a87873..acf08a2 100644
--- a/crates/codecite-mcp/src/server.rs
+++ b/crates/codecite-mcp/src/server.rs
@@ -1,15 +1,141 @@
 //! The MCP server: one tool per operation, each returning the JSON the
 //! matching CLI command prints.
 
+use std::num::NonZeroU32;
 use std::path::PathBuf;
 
-use codecite_core::Corpora;
+use codecite_core::{
+    Corpora, DEFAULT_MAX_LINES, ReadOptions, ReadResult, SearchQuery, SearchResults, Store,
+};
 use rmcp::handler::server::router::tool::ToolRouter;
-use rmcp::handler::server::wrapper::Json;
+use rmcp::handler::server::wrapper::{Json, Parameters};
 use rmcp::{ServerHandler, ServiceExt as _, tool, tool_handler, tool_router};
+use schemars::JsonSchema;
+use serde::{Deserialize, Serialize};
 
 use crate::Result;
 
+/// The `search` tool's inputs.
+#[derive(Debug, Deserialize, JsonSchema)]
+#[serde(deny_unknown_fields)]
+pub struct SearchParams {
+    /// What to search for: words, identifiers or both.
+    pub query: String,
+    /// The corpus to search, as `corpora` lists it. Omitted, the only
+    /// corpus there is.
+    #[serde(default)]
+    pub corpus: Option<String>,
+    /// Only files in this language, such as `rust` or `markdown`.
+    #[serde(default)]
+    pub language: Option<String>,
+    /// Only files whose path matches this glob; `*` matches `/` too.
+    #[serde(default)]
+    pub path: Option<String>,
+    /// The most results to return.
+    #[serde(default = "default_limit")]
+    pub limit: NonZeroU32,
+    /// Search an index older than its maximum age instead of refusing it.
+    #[serde(default)]
+    pub allow_stale: bool,
+}
+
+/// The `read` tool's inputs.
+#[derive(Debug, Deserialize, JsonSchema)]
+#[serde(deny_unknown_fields)]
+pub struct ReadParams {
+    /// A code or spec citation, as `search` returns it, as a spec URL, or
+    /// as people write it: `RFC 9449 §4.3`, `OIDC Core §3.1.3.7 step 13`.
+    pub citation: String,
+    /// The corpus to read from, as `corpora` lists it. Omitted, the only
+    /// corpus there is.
+    #[serde(default)]
+    pub corpus: Option<String>,
+    /// The most lines of text to return.
+    #[serde(default = "default_max_lines")]
+    pub max_lines: NonZeroU32,
+    /// Read an index older than its maximum age instead of refusing it.
+    #[serde(default)]
+    pub allow_stale: bool,
+}
+
+/// `search` returns 10 results unless asked for another number, as the CLI
+/// does.
+fn default_limit() -> NonZeroU32 {
+    NonZeroU32::new(10).expect("10 is not zero")
+}
+
+/// `read` returns [`DEFAULT_MAX_LINES`] lines unless asked for another
+/// number, as the CLI does.
+fn default_max_lines() -> NonZeroU32 {
+    NonZeroU32::new(DEFAULT_MAX_LINES).expect("the default is not zero")
+}
+
+/// A failed tool call's structured content: `{"error": {...}}`.
+#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
+pub struct ToolError {
+    /// What went wrong.
+    pub error: ErrorInfo,
+}
+
+/// What went wrong in a tool call, for an agent to act on.
+#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
+pub struct ErrorInfo {
+    /// The error's kind in snake_case, such as `unknown_corpus` or
+    /// `ambiguous_document`: [`codecite_core::Error::kind`].
+    pub kind: String,
+    /// The message the CLI prints, followed by its causes.
+    pub message: String,
+    /// What the caller could have meant: corpora, names, versions or
+    /// pinned citations. Left out when there are none.
+    #[serde(skip_serializing_if = "Vec::is_empty")]
+    pub candidates: Vec<String>,
+}
+
+impl From<&codecite_core::Error> for ToolError {
+    fn from(err: &codecite_core::Error) -> Self {
+        ToolError {
+            error: ErrorInfo {
+                kind: err.kind().into(),
+                message: with_causes(err),
+                candidates: err.candidates().to_vec(),
+            },
+        }
+    }
+}
+
+/// What a tool returns: its result, or a tool error with structured
+/// content. rmcp sends an `Err` as a result with `isError: true`, so the
+/// agent sees the error instead of a failed request. The tools spell the
+/// type out, since `#[tool]` reads the output schema from the
+/// `Result<Json<T>, _>` it sees and can't look through an alias.
+type ToolResult<T> = Result<Json<T>, Json<ToolError>>;
+
+/// Runs `query`, a blocking call into the store, on tokio's blocking pool
+/// and turns its result into a tool result.
+///
+/// SQLite blocks the thread it runs on. On the blocking pool it leaves
+/// the runtime's thread free to read and answer other messages, such as a
+/// cancellation, meanwhile. The closure must own what it uses (`'static`)
+/// and be `Send`, since it runs on another thread, perhaps after the call
+/// that spawned it has returned.
+async fn blocking<T, F>(query: F) -> ToolResult<T>
+where
+    T: Send + 'static,
+    F: FnOnce() -> codecite_core::Result<T> + Send + 'static,
+{
+    match tokio::task::spawn_blocking(query).await {
+        Ok(Ok(value)) => Ok(Json(value)),
+        Ok(Err(err)) => Err(Json(ToolError::from(&err))),
+        Err(join) => Err(Json(ToolError {
+            error: ErrorInfo {
+                kind: "internal".into(),
+                message: format!("the query stopped unexpectedly: {join}"),
+                candidates: Vec::new(),
+            },
+        })),
+    }
+}
+
 /// codecite's MCP server. It answers from the corpus files in one data
 /// directory and holds no state between calls.
 #[derive(Debug, Clone)]
@@ -36,15 +162,56 @@ impl Server {
     #[tool(
         description = "List the corpora this server can search: each one's age, size, source count and whether it is stale. A stale corpus is refused unless a query allows it."
     )]
-    async fn corpora(&self) -> Result<Json<Corpora>, String> {
-        // SQLite blocks. Running it on tokio's blocking pool keeps this
-        // thread free to read and answer other messages meanwhile.
+    async fn corpora(&self) -> Result<Json<Corpora>, Json<ToolError>> {
         let dir = self.data_dir.clone();
-        match tokio::task::spawn_blocking(move || codecite_core::corpora(&dir)).await {
-            Ok(Ok(corpora)) => Ok(Json(corpora)),
-            Ok(Err(err)) => Err(with_causes(&err)),
-            Err(join) => Err(format!("listing corpora failed: {join}")),
-        }
+        blocking(move || codecite_core::corpora(&dir)).await
+    }
+
+    /// The `search` tool: the same JSON as `codecite search --json`.
+    #[tool(
+        description = "Search a corpus of code and specs. Returns ranked results, best first, each with its citation, kind, symbol, score, license class, indexed_at and a snippet of at most 40 lines; pass a result's citation to `read` for its exact text. A search that matches nothing returns no results; an unknown corpus, a stale index or a query with no words is an error."
+    )]
+    async fn search(
+        &self,
+        Parameters(params): Parameters<SearchParams>,
+    ) -> Result<Json<SearchResults>, Json<ToolError>> {
+        let dir = self.data_dir.clone();
+        // `params` moves into the closure, which owns it on the blocking
+        // pool; `SearchQuery` borrows from it there.
+        blocking(move || {
+            let store = Store::open_corpus_or_only(&dir, params.corpus.as_deref())?;
+            let results = store.search(&SearchQuery {
+                text: &params.query,
+                language: params.language.as_deref(),
+                path: params.path.as_deref(),
+                limit: params.limit.get(),
+                allow_stale: params.allow_stale,
+            })?;
+            Ok(SearchResults { results })
+        })
+        .await
+    }
+
+    /// The `read` tool: the same JSON as `codecite read --json`.
+    #[tool(
+        description = "Read the exact text a citation points at, under its pinned citation, with its license class; a spec section or step adds its title and requirement keywords (strength). Text from a `quote` source carries an instruction: quote it exactly with its citation, never paraphrase. A citation that is not in the corpus is an error with candidates, never the nearest guess."
+    )]
+    async fn read(
+        &self,
+        Parameters(params): Parameters<ReadParams>,
+    ) -> Result<Json<ReadResult>, Json<ToolError>> {
+        let dir = self.data_dir.clone();
+        blocking(move || {
+            let store = Store::open_corpus_or_only(&dir, params.corpus.as_deref())?;
+            store.read(
+                &params.citation,
+                &ReadOptions {
+                    max_lines: params.max_lines.get(),
+                    allow_stale: params.allow_stale,
+                },
+            )
+        })
+        .await
     }
 }
 
@@ -64,7 +231,7 @@ fn with_causes(err: &dyn std::error::Error) -> String {
 #[tool_handler(
     router = self.tool_router,
     name = "codecite",
-    instructions = "codecite answers from indexed code and specs, with a citation on every result. Start with `corpora` to see what is indexed and how fresh it is."
+    instructions = "codecite answers from indexed code and specs, with a citation on every result. Start with `corpora` to see what is indexed and how fresh it is, `search` a corpus, then `read` a result's citation for its exact text. Quote text from a `quote` source exactly, with its citation."
 )]
 impl ServerHandler for Server {}
 
@@ -91,9 +258,14 @@ pub fn serve_stdio(data_dir: PathBuf) -> Result<()> {
 #[cfg(test)]
 mod tests {
     use super::*;
-    use codecite_core::{SourceRecord, Store};
+    use codecite_core::{ChunkRecord, DocumentRecord, Keyword, SourceRecord};
     use rmcp::ClientHandler;
-    use rmcp::model::{CallToolRequestParams, ClientConfig};
+    use rmcp::model::{CallToolRequestParams, CallToolResult, ClientConfig};
+    use rmcp::service::{RoleClient, RunningService};
+    use serde_json::{Value, json};
+
+    const HEAD: &str = "0123456789abcdef0123456789abcdef01234567";
+    const LIB: &str = "pub fn validate(token: &str) -> bool {\n    !token.is_empty()\n}\n";
 
     /// A client that only calls tools.
     #[derive(Default)]
@@ -105,71 +277,403 @@ mod tests {
         }
     }
 
-    /// A data directory holding one corpus, `demo`, with one source.
+    fn source<'a>(name: &'a str, revision: &'a str, class: &'a str) -> SourceRecord<'a> {
+        SourceRecord {
+            name,
+            kind: if class == "quote" {
+                "document"
+            } else {
+                "local"
+            },
+            url: "https://example.com",
+            revision,
+            license_spdx: Some("MIT"),
+            license_class: class,
+            is_current: true,
+            obsoleted_by: None,
+            updated_by: None,
+        }
+    }
+
+    fn section<'a>(number: &'a str, title: &'a str, text: &'a str) -> ChunkRecord<'a> {
+        ChunkRecord {
+            kind: "section",
+            start_line: 1,
+            end_line: 1,
+            text,
+            number: Some(number),
+            anchor: None,
+            title: Some(title),
+            parent: None,
+            strength: &[Keyword::Must],
+            symbol: None,
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
+    /// Writes corpus `name` into `dir`: a code source `app` with one
+    /// function, and two specs that both have a §4.3.
+    fn write_corpus(dir: &std::path::Path, name: &str) {
+        let mut store = Store::open_read_write(&dir.join(format!("{name}.db"))).unwrap();
+        let app = store.upsert_source(&source("app", HEAD, "adapt")).unwrap();
+        let code = DocumentRecord {
+            path: "src/lib.rs",
+            language: Some("rust"),
+            content_hash: b"lib",
+            size_bytes: 1,
+            dirty: false,
+            text: Some(LIB),
+            chunks: vec![ChunkRecord {
+                kind: "function",
+                start_line: 1,
+                end_line: 3,
+                text: LIB.trim_end(),
+                number: None,
+                anchor: None,
+                title: None,
+                parent: None,
+                strength: &[],
+                symbol: None,
+            }],
+        };
+        store.write_document(app, &code).unwrap();
+        for (doc, hash) in [("rfc9449", "1a2b"), ("rfc9700", "3c4d")] {
+            let revision = format!("{hash:0<64}");
+            let id = store
+                .upsert_source(&source(doc, &revision, "quote"))
+                .unwrap();
+            let text = format!("4.3.  Checking Proofs\n\nThe server MUST check the {doc} proof.");
+            let chunks = vec![section("4.3", "Checking Proofs", &text)];
+            store.write_document(id, &spec(doc, chunks)).unwrap();
+        }
+        store.set_indexed_at(30).unwrap();
+    }
+
+    /// A data directory holding one corpus, `demo`.
     fn fixture() -> tempfile::TempDir {
         let dir = tempfile::tempdir().unwrap();
-        let mut store = Store::open_read_write(&dir.path().join("demo.db")).unwrap();
-        store
-            .upsert_source(&SourceRecord {
-                name: "app",
-                kind: "local",
-                url: "/src/app",
-                revision: "0123456789abcdef0123456789abcdef01234567",
-                license_spdx: Some("MIT"),
-                license_class: "adapt",
-                is_current: true,
-                obsoleted_by: None,
-                updated_by: None,
+        write_corpus(dir.path(), "demo");
+        dir
+    }
+
+    /// A server on `dir` and a client connected to it over an in-memory
+    /// pipe, which stands in for stdio: the server reads one end, the
+    /// client the other.
+    struct Session {
+        client: RunningService<RoleClient, Client>,
+        server: tokio::task::JoinHandle<()>,
+    }
+
+    impl Session {
+        async fn start(dir: &std::path::Path) -> Session {
+            let (server_end, client_end) = tokio::io::duplex(64 * 1024);
+            let server = Server::new(dir.to_path_buf());
+            let server = tokio::spawn(async move {
+                let running = server.serve(server_end).await.unwrap();
+                running.waiting().await.unwrap();
+            });
+            let client = Client.serve(client_end).await.unwrap();
+            Session { client, server }
+        }
+
+        /// Calls `tool` with `arguments`, a JSON object.
+        async fn call(&self, tool: &'static str, arguments: Value) -> CallToolResult {
+            let Value::Object(arguments) = arguments else {
+                panic!("arguments must be an object");
+            };
+            self.client
+                .call_tool(CallToolRequestParams::new(tool).with_arguments(arguments))
+                .await
+                .unwrap()
+        }
+
+        /// Calls `tool`, checks it succeeded, and returns its structured
+        /// content, after checking that the text is the same JSON.
+        async fn ok(&self, tool: &'static str, arguments: Value) -> Value {
+            let result = self.call(tool, arguments).await;
+            assert_eq!(result.is_error, Some(false), "{result:?}");
+            structured(result)
+        }
+
+        /// Calls `tool`, checks it failed, and returns its `error` object.
+        async fn err(&self, tool: &'static str, arguments: Value) -> Value {
+            let result = self.call(tool, arguments).await;
+            assert_eq!(result.is_error, Some(true), "{result:?}");
+            structured(result)["error"].clone()
+        }
+
+        async fn stop(self) {
+            self.client.cancel().await.unwrap();
+            self.server.await.unwrap();
+        }
+    }
+
+    /// The structured content, after checking that the text is the same
+    /// JSON, for clients that read only text.
+    fn structured(result: CallToolResult) -> Value {
+        let structured = result.structured_content.unwrap();
+        let text = &result.content[0].as_text().unwrap().text;
+        assert_eq!(text, &structured.to_string());
+        structured
+    }
+
+    /// What the store answers for the same query, as the CLI prints it.
+    fn store_search(dir: &std::path::Path, text: &str) -> Value {
+        let store = Store::open_corpus(dir, "demo").unwrap();
+        let results = store
+            .search(&SearchQuery {
+                text,
+                language: None,
+                path: None,
+                limit: 10,
+                allow_stale: false,
             })
             .unwrap();
-        store.set_indexed_at(30).unwrap();
-        dir
+        serde_json::to_value(SearchResults { results }).unwrap()
     }
 
     #[tokio::test]
     async fn a_client_lists_the_tools_and_calls_corpora() {
         let dir = fixture();
-        // An in-memory pipe stands in for stdio: the server reads one end,
-        // the client the other.
-        let (server_end, client_end) = tokio::io::duplex(64 * 1024);
-        let server = tokio::spawn(Server::new(dir.path().to_path_buf()).serve(server_end));
-        let client = Client.serve(client_end).await.unwrap();
-        let server = server.await.unwrap().unwrap();
-
-        let info = client.peer_info().unwrap();
+        let session = Session::start(dir.path()).await;
+
+        let info = session.client.peer_info().unwrap();
         assert_eq!(info.server_info.as_ref().unwrap().name, "codecite");
 
-        let tools = client.list_all_tools().await.unwrap();
-        let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
-        assert_eq!(names, ["corpora"]);
-        let corpora = &tools[0];
+        let tools = session.client.list_all_tools().await.unwrap();
+        let mut names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
+        names.sort_unstable();
+        assert_eq!(names, ["corpora", "read", "search"]);
+        let corpora = tools.iter().find(|t| t.name == "corpora").unwrap();
         assert_eq!(corpora.input_schema.get("type").unwrap(), "object");
         // The output schema comes from `Corpora`'s Rust type.
-        let output = serde_json::Value::Object((*corpora.output_schema.clone().unwrap()).clone());
+        let output = Value::Object((*corpora.output_schema.clone().unwrap()).clone());
         assert_eq!(output["properties"]["corpora"]["type"], "array");
 
-        let result = client
-            .call_tool(CallToolRequestParams::new("corpora"))
-            .await
-            .unwrap();
-        assert_eq!(result.is_error, Some(false));
-        let structured = result.structured_content.unwrap();
+        let structured = session.ok("corpora", json!({})).await;
         assert_eq!(structured["corpora"][0]["corpus"], "demo");
-        assert_eq!(structured["corpora"][0]["sources"], 1);
+        assert_eq!(structured["corpora"][0]["sources"], 3);
         assert_eq!(structured["corpora"][0]["stale"], false);
-        // The text is the same JSON, for clients that read only text.
-        let text = &result.content[0].as_text().unwrap().text;
-        assert_eq!(
-            serde_json::from_str::<serde_json::Value>(text).unwrap(),
-            structured
-        );
         assert_eq!(
             structured,
             serde_json::to_value(codecite_core::corpora(dir.path()).unwrap()).unwrap()
         );
+        session.stop().await;
+    }
+
+    #[tokio::test]
+    async fn the_search_and_read_tools_describe_their_inputs() {
+        let dir = fixture();
+        let session = Session::start(dir.path()).await;
+        let tools = session.client.list_all_tools().await.unwrap();
+        let schema = |name: &str| {
+            let tool = tools.iter().find(|t| t.name == name).unwrap();
+            let input = Value::Object((*tool.input_schema).clone());
+            let output = Value::Object((*tool.output_schema.clone().unwrap()).clone());
+            (input, output)
+        };
+        let (input, output) = schema("search");
+        assert_eq!(input["required"], json!(["query"]));
+        for field in ["corpus", "language", "path", "limit", "allow_stale"] {
+            assert!(input["properties"].get(field).is_some(), "{field}: {input}");
+        }
+        assert_eq!(input["properties"]["limit"]["default"], 10);
+        assert_eq!(input["properties"]["limit"]["minimum"], 1);
+        assert_eq!(output["properties"]["results"]["type"], "array");
+        let (input, output) = schema("read");
+        assert_eq!(input["required"], json!(["citation"]));
+        assert_eq!(input["properties"]["max_lines"]["default"], 80);
+        assert!(
+            output["properties"].get("instruction").is_some(),
+            "{output}"
+        );
+        session.stop().await;
+    }
+
+    #[tokio::test]
+    async fn search_returns_what_the_store_does() {
+        let dir = fixture();
+        let session = Session::start(dir.path()).await;
+        // The only corpus is the default.
+        let found = session.ok("search", json!({"query": "validate"})).await;
+        assert_eq!(found, store_search(dir.path(), "validate"));
+        let top = &found["results"][0];
+        assert_eq!(top["citation"], format!("app@{HEAD}:src/lib.rs#L1-L3"));
+        assert_eq!(top["license_class"], "adapt");
+
+        let named = json!({"query": "proof", "corpus": "demo", "limit": 1});
+        let found = session.ok("search", named).await;
+        assert_eq!(found["results"].as_array().unwrap().len(), 1);
+        // Nothing matching is an empty list, not an error.
+        let none = session.ok("search", json!({"query": "zzzz"})).await;
+        assert_eq!(none, json!({"results": []}));
+        session.stop().await;
+    }
+
+    #[tokio::test]
+    async fn read_of_a_spec_says_to_quote_it_exactly() {
+        let dir = fixture();
+        let session = Session::start(dir.path()).await;
+        let read = session
+            .ok(
+                "read",
+                json!({"citation": "rfc9449@sha256:1a2b00000000#section-4.3"}),
+            )
+            .await;
+        assert_eq!(read["citation"], "rfc9449@sha256:1a2b00000000#section-4.3");
+        assert_eq!(read["kind"], "section");
+        assert_eq!(read["title"], "Checking Proofs");
+        assert_eq!(read["strength"], json!(["MUST"]));
+        assert_eq!(read["license_class"], "quote");
+        assert_eq!(read["instruction"], codecite_core::QUOTE_INSTRUCTION);
+        assert_eq!(
+            read["text"],
+            "4.3.  Checking Proofs\n\nThe server MUST check the rfc9449 proof."
+        );
+        let store = Store::open_corpus(dir.path(), "demo").unwrap();
+        let direct = store
+            .read(
+                "rfc9449@sha256:1a2b00000000#section-4.3",
+                &ReadOptions::default(),
+            )
+            .unwrap();
+        assert_eq!(read, serde_json::to_value(direct).unwrap());
 
-        client.cancel().await.unwrap();
-        server.waiting().await.unwrap();
+        // Code has no instruction, and the line cap applies.
+        let citation = format!("app@{HEAD}:src/lib.rs#L1-L3");
+        let read = session
+            .ok("read", json!({"citation": citation, "max_lines": 1}))
+            .await;
+        assert_eq!(read["text"], "pub fn validate(token: &str) -> bool {");
+        assert_eq!(read["truncated"], true);
+        assert!(read.get("instruction").is_none(), "{read}");
+        session.stop().await;
+    }
+
+    #[tokio::test]
+    async fn typed_errors_are_tool_errors_with_candidates() {
+        let dir = fixture();
+        let session = Session::start(dir.path()).await;
+
+        let err = session
+            .err("search", json!({"query": "validate", "corpus": "dmeo"}))
+            .await;
+        assert_eq!(
+            err,
+            json!({
+                "kind": "unknown_corpus",
+                "message": "unknown corpus `dmeo`; known corpora: demo",
+                "candidates": ["demo"]
+            })
+        );
+        let err = session.err("search", json!({"query": "  !! "})).await;
+        assert_eq!(err["kind"], "empty_query");
+        assert!(err.get("candidates").is_none(), "{err}");
+
+        // A bare § that two current documents have.
+        let err = session.err("read", json!({"citation": "§4.3"})).await;
+        assert_eq!(err["kind"], "ambiguous_document");
+        assert_eq!(
+            err["candidates"],
+            json!([
+                "rfc9449@sha256:1a2b00000000#section-4.3",
+                "rfc9700@sha256:3c4d00000000#section-4.3"
+            ])
+        );
+        let err = session
+            .err("read", json!({"citation": "hello world"}))
+            .await;
+        assert_eq!(err["kind"], "not_a_citation");
+        let err = session
+            .err("read", json!({"citation": "RFC 9449 §04.3"}))
+            .await;
+        assert_eq!(err["kind"], "bad_citation");
+        // The message carries the parse error, its cause.
+        let message = err["message"].as_str().unwrap();
+        assert!(
+            message.contains("is not dot-separated numbers"),
+            "{message}"
+        );
+        session.stop().await;
+    }
+
+    #[tokio::test]
+    async fn a_stale_index_is_refused_unless_allowed() {
+        let dir = fixture();
+        rusqlite::Connection::open(dir.path().join("demo.db"))
+            .unwrap()
+            .execute(
+                "UPDATE meta SET value = '2026-01-01T00:00:00Z' WHERE key = 'indexed_at'",
+                [],
+            )
+            .unwrap();
+        let session = Session::start(dir.path()).await;
+        let err = session.err("search", json!({"query": "validate"})).await;
+        assert_eq!(err["kind"], "stale_index");
+        let err = session.err("read", json!({"citation": "§4.3"})).await;
+        assert_eq!(err["kind"], "stale_index");
+        let allowed = json!({"query": "validate", "allow_stale": true});
+        let found = session.ok("search", allowed).await;
+        assert_eq!(found["results"].as_array().unwrap().len(), 1);
+        let allowed = json!({"citation": "RFC 9449 §4.3", "allow_stale": true});
+        assert_eq!(session.ok("read", allowed).await["kind"], "section");
+        session.stop().await;
+    }
+
+    #[tokio::test]
+    async fn several_corpora_need_a_name() {
+        let dir = fixture();
+        write_corpus(dir.path(), "specs");
+        let session = Session::start(dir.path()).await;
+        let err = session.err("search", json!({"query": "validate"})).await;
+        assert_eq!(
+            err,
+            json!({
+                "kind": "no_corpus_named",
+                "message": "name a corpus; known corpora: demo, specs",
+                "candidates": ["demo", "specs"]
+            })
+        );
+        let err = session
+            .err("read", json!({"citation": "RFC 9449 §4.3"}))
+            .await;
+        assert_eq!(err["kind"], "no_corpus_named");
+        let named = json!({"citation": "RFC 9449 §4.3", "corpus": "specs"});
+        assert_eq!(session.ok("read", named).await["kind"], "section");
+        session.stop().await;
+    }
+
+    #[tokio::test]
+    async fn bad_arguments_are_refused_before_the_store() {
+        let dir = fixture();
+        let session = Session::start(dir.path()).await;
+        for (tool, arguments) in [
+            ("search", json!({"query": "validate", "limit": 0})),
+            ("search", json!({"query": "validate", "limit": -1})),
+            ("search", json!({})),
+            ("read", json!({"citation": "§4.3", "max_lines": 0})),
+            ("read", json!({"citation": "§4.3", "corpus_name": "demo"})),
+        ] {
+            let result = session.call(tool, arguments.clone()).await;
+            assert_eq!(result.is_error, Some(true), "{tool} {arguments}");
+            let text = &result.content[0].as_text().unwrap().text;
+            assert!(
+                text.starts_with("failed to deserialize parameters: "),
+                "{tool} {arguments}: {text}"
+            );
+        }
+        session.stop().await;
     }
 
     #[tokio::test]
@@ -178,21 +682,12 @@ mod tests {
         // A file where the directory should be.
         let file = dir.path().join("data");
         std::fs::write(&file, "x").unwrap();
-        let (server_end, client_end) = tokio::io::duplex(64 * 1024);
-        let server = tokio::spawn(Server::new(file).serve(server_end));
-        let client = Client.serve(client_end).await.unwrap();
-        let server = server.await.unwrap().unwrap();
-
-        let result = client
-            .call_tool(CallToolRequestParams::new("corpora"))
-            .await
-            .unwrap();
-        assert_eq!(result.is_error, Some(true));
-        let text = &result.content[0].as_text().unwrap().text;
-        assert!(text.starts_with("I/O error: "), "{text}");
-        assert!(text.len() > "I/O error: ".len(), "{text}");
-
-        client.cancel().await.unwrap();
-        server.waiting().await.unwrap();
+        let session = Session::start(&file).await;
+        let err = session.err("corpora", json!({})).await;
+        assert_eq!(err["kind"], "io");
+        let message = err["message"].as_str().unwrap();
+        assert!(message.starts_with("I/O error: "), "{message}");
+        assert!(message.len() > "I/O error: ".len(), "{message}");
+        session.stop().await;
     }
 }
diff --git a/crates/codecite/src/main.rs b/crates/codecite/src/main.rs
index fe8b14c..3889126 100644
--- a/crates/codecite/src/main.rs
+++ b/crates/codecite/src/main.rs
@@ -1,13 +1,13 @@
 //! The `codecite` command-line tool.
 
 use std::fs;
-use std::io::Write as _;
 use std::path::{Path, PathBuf};
 
 use anyhow::{Context as _, bail};
 use clap::{Parser, Subcommand};
 use codecite_core::{
-    DEFAULT_MAX_LINES, ReadOptions, ReadResult, SearchQuery, SearchResult, Store, is_corpus_name,
+    DEFAULT_MAX_LINES, ReadOptions, ReadResult, SearchQuery, SearchResult, SearchResults, Store,
+    is_corpus_name,
 };
 use codecite_index::{Clones, Fetcher, LicenseClass, Manifest, Source, WalkOptions, index_corpus};
 
@@ -49,7 +49,8 @@ enum Command {
         /// The most results to print.
         #[arg(long, default_value_t = 10, value_parser = clap::value_parser!(u32).range(1..))]
         limit: u32,
-        /// Print the results as a JSON array.
+        /// Print the results as JSON: `{"results": [...]}`, as the MCP
+        /// `search` tool returns them.
         #[arg(long)]
         json: bool,
         /// Search an index older than its manifest's `max_age_days`.
@@ -66,7 +67,7 @@ enum Command {
         /// The most lines of text to print.
         #[arg(long, default_value_t = DEFAULT_MAX_LINES, value_parser = clap::value_parser!(u32).range(1..))]
         max_lines: u32,
-        /// Print the result as a JSON object.
+        /// Print the result as JSON, as the MCP `read` tool returns it.
         #[arg(long)]
         json: bool,
         /// Read an index older than its manifest's `max_age_days`.
@@ -184,7 +185,7 @@ fn search(corpus: &str, query: &SearchQuery<'_>, json: bool) -> anyhow::Result<(
     let dirs = project_dirs()?;
     let store = Store::open_corpus(dirs.data_dir(), corpus)?;
     let results = store.search(query)?;
-    to_stdout(|out| write_results(out, &results, json))
+    to_stdout(|out| write_results(out, results, json))
 }
 
 /// `codecite read`: opens the corpus read-only and prints the cited text,
@@ -200,11 +201,8 @@ fn read(corpus: &str, citation: &str, options: &ReadOptions, json: bool) -> anyh
 /// them, byte for byte.
 fn corpora() -> anyhow::Result<()> {
     let dirs = project_dirs()?;
-    let corpora = codecite_core::corpora(dirs.data_dir())?;
-    // rmcp writes a tool's text as `serde_json::Value::to_string`, which
-    // sorts the keys; going through a `Value` here prints the same bytes.
-    let json = serde_json::to_value(&corpora)?.to_string();
-    to_stdout(|out| writeln!(out, "{json}"))
+    let corpora = serde_json::to_value(codecite_core::corpora(dirs.data_dir())?)?;
+    to_stdout(|out| write_json(out, corpora))
 }
 
 /// `codecite mcp`: serves the corpora in the data directory to one MCP
@@ -233,6 +231,13 @@ fn to_stdout(
     }
 }
 
+/// Writes `value` to `out` as the MCP server's tools write their text.
+/// rmcp writes it as `serde_json::Value::to_string`, compact with sorted
+/// keys, so going through a `Value` here prints the same bytes.
+fn write_json(out: &mut impl std::io::Write, value: serde_json::Value) -> std::io::Result<()> {
+    writeln!(out, "{value}")
+}
+
 /// Writes a read result to `out`: as JSON, or its `Display` form with
 /// control characters escaped line by line.
 fn write_read(
@@ -241,8 +246,7 @@ fn write_read(
     json: bool,
 ) -> std::io::Result<()> {
     if json {
-        serde_json::to_writer_pretty(&mut *out, result)?;
-        writeln!(out)?;
+        write_json(out, serde_json::to_value(result)?)?;
     } else {
         for line in result.to_string().lines() {
             writeln!(out, "{}", escape_controls(line))?;
@@ -251,18 +255,18 @@ fn write_read(
     out.flush()
 }
 
-/// Writes the results to `out`. Unlike `println!`, which panics when stdout
+/// Writes the results to `out`, as `{"results": [...]}` JSON or in the
+/// short human form. Unlike `println!`, which panics when stdout
 /// is closed, a failed write comes back as an error.
 fn write_results(
     out: &mut impl std::io::Write,
-    results: &[SearchResult],
+    results: Vec<SearchResult>,
     json: bool,
 ) -> std::io::Result<()> {
     if json {
-        serde_json::to_writer_pretty(&mut *out, results)?;
-        writeln!(out)?;
+        write_json(out, serde_json::to_value(SearchResults { results })?)?;
     } else {
-        out.write_all(human(results).as_bytes())?;
+        out.write_all(human(&results).as_bytes())?;
     }
     out.flush()
 }
@@ -579,15 +583,15 @@ code · adapt · indexed 2026-10-20T06:00:00Z
     #[test]
     fn a_closed_stdout_is_an_error_not_a_panic() {
         for json in [true, false] {
-            let err = write_results(&mut ClosedPipe, &[], json).unwrap_err();
+            let err = write_results(&mut ClosedPipe, Vec::new(), json).unwrap_err();
             assert_eq!(err.kind(), std::io::ErrorKind::BrokenPipe);
         }
         let mut out = Vec::new();
-        write_results(&mut out, &[], false).unwrap();
+        write_results(&mut out, Vec::new(), false).unwrap();
         assert_eq!(out, b"no results\n");
         let mut out = Vec::new();
-        write_results(&mut out, &[], true).unwrap();
-        assert_eq!(out, b"[]\n");
+        write_results(&mut out, Vec::new(), true).unwrap();
+        assert_eq!(out, b"{\"results\":[]}\n");
     }
 
     #[test]
diff --git a/crates/codecite/tests/mcp.rs b/crates/codecite/tests/mcp.rs
index 582a907..01b2e38 100644
--- a/crates/codecite/tests/mcp.rs
+++ b/crates/codecite/tests/mcp.rs
@@ -6,28 +6,82 @@
 use std::io::{BufRead, BufReader, Write};
 use std::process::{Command, Stdio};
 
-use codecite_core::{SourceRecord, Store};
+use codecite_core::{ChunkRecord, DocumentRecord, Keyword, SourceRecord, Store};
 use serde_json::{Value, json};
 
-/// A data home holding one corpus, `demo`, with one source.
+const LIB: &str = "pub fn validate(token: &str) -> bool {\n    !token.is_empty()\n}\n";
+const SPEC: &str = "4.3.  Checking DPoP Proofs\n\nThe server MUST validate the proof.";
+
+/// A data home holding one corpus, `demo`, with a code source `app` and a
+/// spec, `rfc9449`.
 fn data_home() -> tempfile::TempDir {
     let home = tempfile::tempdir().unwrap();
     let dir = home.path().join("codecite");
     std::fs::create_dir_all(&dir).unwrap();
     let mut store = Store::open_read_write(&dir.join("demo.db")).unwrap();
-    store
+    let chunk = ChunkRecord {
+        kind: "function",
+        start_line: 1,
+        end_line: 3,
+        text: LIB.trim_end(),
+        number: None,
+        anchor: None,
+        title: None,
+        parent: None,
+        strength: &[],
+        symbol: None,
+    };
+    let source = SourceRecord {
+        name: "app",
+        kind: "local",
+        url: "/src/app",
+        revision: "0123456789abcdef0123456789abcdef01234567",
+        license_spdx: Some("MIT"),
+        license_class: "adapt",
+        is_current: true,
+        obsoleted_by: None,
+        updated_by: None,
+    };
+    let app = store.upsert_source(&source).unwrap();
+    let code = DocumentRecord {
+        path: "src/lib.rs",
+        language: Some("rust"),
+        content_hash: b"lib",
+        size_bytes: 1,
+        dirty: false,
+        text: Some(LIB),
+        chunks: vec![chunk.clone()],
+    };
+    store.write_document(app, &code).unwrap();
+    let rfc = store
         .upsert_source(&SourceRecord {
-            name: "app",
-            kind: "local",
-            url: "/src/app",
-            revision: "0123456789abcdef0123456789abcdef01234567",
-            license_spdx: Some("MIT"),
-            license_class: "adapt",
-            is_current: true,
-            obsoleted_by: None,
-            updated_by: None,
+            name: "rfc9449",
+            kind: "document",
+            url: "https://www.rfc-editor.org/rfc/rfc9449.html",
+            revision: "1a2b3c4d5e6f00000000000000000000000000000000000000000000000000ff",
+            license_spdx: None,
+            license_class: "quote",
+            ..source
         })
         .unwrap();
+    let section = ChunkRecord {
+        kind: "section",
+        text: SPEC,
+        number: Some("4.3"),
+        anchor: Some("section-4.3"),
+        title: Some("Checking DPoP Proofs"),
+        strength: &[Keyword::Must],
+        ..chunk
+    };
+    let spec = DocumentRecord {
+        path: "rfc9449",
+        language: None,
+        content_hash: b"rfc9449",
+        text: None,
+        chunks: vec![section],
+        ..code
+    };
+    store.write_document(rfc, &spec).unwrap();
     store.set_indexed_at(30).unwrap();
     home
 }
@@ -47,7 +101,7 @@ fn corpora_prints_the_corpora_as_json() {
     assert!(out.status.success(), "{out:?}");
     let listed: Value = serde_json::from_slice(&out.stdout).unwrap();
     assert_eq!(listed["corpora"][0]["corpus"], "demo");
-    assert_eq!(listed["corpora"][0]["sources"], 1);
+    assert_eq!(listed["corpora"][0]["sources"], 2);
     assert_eq!(listed["corpora"][0]["stale"], false);
     assert_eq!(listed["corpora"].as_array().unwrap().len(), 1);
 
@@ -110,25 +164,101 @@ fn mcp_answers_on_stdout_and_logs_on_stderr() {
 
     let list = send(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})).unwrap();
     let tools = list["result"]["tools"].as_array().unwrap();
-    assert_eq!(tools.len(), 1, "{list}");
-    assert_eq!(tools[0]["name"], "corpora");
-    assert_eq!(tools[0]["inputSchema"]["type"], "object");
+    let mut names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
+    names.sort_unstable();
+    assert_eq!(names, ["corpora", "read", "search"], "{list}");
+    for tool in tools {
+        assert_eq!(tool["inputSchema"]["type"], "object", "{tool}");
+    }
 
+    // Each tool's text is what the CLI prints for the same query, byte for
+    // byte: corpora, a search, a read of code and of a spec, and an empty
+    // search.
+    let cases = [
+        ("corpora", json!({}), vec!["corpora"]),
+        (
+            "search",
+            json!({"query": "validate", "corpus": "demo"}),
+            vec!["search", "demo", "validate", "--json"],
+        ),
+        (
+            "search",
+            json!({"query": "proof", "limit": 1}),
+            vec!["search", "demo", "proof", "--limit", "1", "--json"],
+        ),
+        (
+            "search",
+            json!({"query": "zzzz"}),
+            vec!["search", "demo", "zzzz", "--json"],
+        ),
+        (
+            "read",
+            json!({"citation": "RFC 9449 §4.3"}),
+            vec!["read", "demo", "RFC 9449 §4.3", "--json"],
+        ),
+        (
+            "read",
+            json!({"citation": "app@0123456:src/lib.rs#L1-L3", "max_lines": 2}),
+            vec![
+                "read",
+                "demo",
+                "app@0123456:src/lib.rs#L1-L3",
+                "--max-lines",
+                "2",
+                "--json",
+            ],
+        ),
+    ];
+    for (id, (tool, arguments, args)) in (3..).zip(cases) {
+        let call = send(json!({
+            "jsonrpc": "2.0", "id": id, "method": "tools/call",
+            "params": {"name": tool, "arguments": arguments}
+        }))
+        .unwrap();
+        assert_eq!(call["result"]["isError"], false, "{call}");
+        let text = call["result"]["content"][0]["text"].as_str().unwrap();
+        assert_eq!(
+            serde_json::from_str::<Value>(text).unwrap(),
+            call["result"]["structuredContent"],
+            "{call}"
+        );
+        let cli = codecite()
+            .args(&args)
+            .env("XDG_DATA_HOME", home.path())
+            .output()
+            .unwrap();
+        assert!(cli.status.success(), "{args:?}: {cli:?}");
+        assert_eq!(
+            String::from_utf8(cli.stdout).unwrap(),
+            format!("{text}\n"),
+            "{args:?}"
+        );
+    }
+
+    // A typed error is a tool error, with the message the CLI prints.
     let call = send(json!({
-        "jsonrpc": "2.0", "id": 3, "method": "tools/call",
-        "params": {"name": "corpora", "arguments": {}}
+        "jsonrpc": "2.0", "id": 99, "method": "tools/call",
+        "params": {"name": "read", "arguments": {"citation": "RFC 9449 §4.9"}}
     }))
     .unwrap();
-    assert_eq!(call["result"]["isError"], false, "{call}");
-    let text = call["result"]["content"][0]["text"].as_str().unwrap();
-
-    // The CLI prints the same JSON, byte for byte.
+    assert_eq!(call["result"]["isError"], true, "{call}");
+    let error = &call["result"]["structuredContent"]["error"];
+    assert_eq!(error["kind"], "no_such_section", "{call}");
+    assert_eq!(
+        error["candidates"],
+        json!(["rfc9449@sha256:1a2b3c4d5e6f#section-4.3"])
+    );
     let cli = codecite()
-        .arg("corpora")
+        .args(["read", "demo", "RFC 9449 §4.9"])
         .env("XDG_DATA_HOME", home.path())
         .output()
         .unwrap();
-    assert_eq!(String::from_utf8(cli.stdout).unwrap(), format!("{text}\n"));
+    assert!(!cli.status.success());
+    let stderr = String::from_utf8(cli.stderr).unwrap();
+    assert!(
+        stderr.contains(error["message"].as_str().unwrap()),
+        "{stderr}"
+    );
 
     // Closing stdin ends the server cleanly.
     drop(stdin);
diff --git a/crates/codecite/tests/read.rs b/crates/codecite/tests/read.rs
index b0e0dfb..7dad89f 100644
--- a/crates/codecite/tests/read.rs
+++ b/crates/codecite/tests/read.rs
@@ -91,7 +91,7 @@ fn stderr(output: &Output) -> String {
 fn read_returns_what_a_search_result_cites() {
     let demo = Demo::new();
     let results = demo.json(&["search", "demo", "validate", "--json"]);
-    let citation = results[0]["citation"].as_str().unwrap();
+    let citation = results["results"][0]["citation"].as_str().unwrap();
     assert!(citation.ends_with(":src/lib.rs#L1-L3"), "{citation}");
     let read = demo.json(&["read", "demo", citation, "--json"]);
     assert_eq!(read["citation"], citation);
@@ -108,7 +108,7 @@ fn read_returns_what_a_search_result_cites() {
 fn the_human_form_and_the_line_cap() {
     let demo = Demo::new();
     let results = demo.json(&["search", "demo", "validate", "--json"]);
-    let citation = results[0]["citation"].as_str().unwrap();
+    let citation = results["results"][0]["citation"].as_str().unwrap();
     // Lines 1 to 5, as an abbreviated commit cites them.
     let (repo, rest) = citation.split_once('@').unwrap();
     let short = format!("{repo}@{}:src/lib.rs#L1-L5", &rest[..7]);
@@ -137,7 +137,7 @@ fn the_human_form_and_the_line_cap() {
 fn errors_name_the_problem() {
     let demo = Demo::new();
     let results = demo.json(&["search", "demo", "validate", "--json"]);
-    let citation = results[0]["citation"].as_str().unwrap();
+    let citation = results["results"][0]["citation"].as_str().unwrap();
     let past_end = citation.replace("#L1-L3", "#L4-L9");
     let cases = [
         (
diff --git a/crates/codecite/tests/search.rs b/crates/codecite/tests/search.rs
index 5809a6f..a686dc2 100644
--- a/crates/codecite/tests/search.rs
+++ b/crates/codecite/tests/search.rs
@@ -86,14 +86,16 @@ impl Demo {
             .unwrap()
     }
 
-    /// Runs `codecite search demo <args> --json` and parses its output.
+    /// Runs `codecite search demo <args> --json` and returns the results it
+    /// prints, `{"results": [...]}`.
     fn json(&self, args: &[&str]) -> Vec<serde_json::Value> {
         let mut all = vec!["search", "demo"];
         all.extend(args);
         all.push("--json");
         let output = self.run(&all);
         assert!(output.status.success(), "{}", stderr(&output));
-        serde_json::from_slice(&output.stdout).unwrap()
+        let mut printed: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
+        serde_json::from_value(printed["results"].take()).unwrap()
     }
 
     fn db(&self) -> PathBuf {
`````

- [ ] **Step 3: Run the tests and the real binary.** `cargo test -p codecite-mcp` → 11 passed; `cargo test -p codecite --test mcp` → 2 passed; `cargo test -p codecite --test search --test read` → 8 passed; then all the gates (307 passed, 7 ignored). Build `cargo build --release -p codecite`, index the identity corpus into a scratch data directory (`XDG_DATA_HOME=/var/tmp/mcp-sr-proto/data ./target/release/codecite index identity`, from the worktree, with `~/repos/auth/identity-model` checked out; documents come from the cache), and run `/var/tmp/mcp-sr-proto/real-run.sh` (or the same by hand): it pipes searches, reads and error cases into `codecite mcp` and compares the CLI's output for the same queries. Expected, up to timestamps, scores and commits:

`````text
--- id 2 isError=False
{"results":[{"citation":"identity-model@9458d60f4f2f20227bddf2594fd7d10cba66ea0a:rust/src/dpop/proof.rs#L207-L212","dirty":false,"indexed_at":"2026-10-20T21:12:51Z","kind":"function","license_class":"adapt","score":32.67599355736647,"snippet":"fn new_jti() -> Result<String> {\n    let mut bytes = [0u8; JTI_BYTES];\n    getrandom::fill(&mut bytes)\n        .map_err(|e| IdentityError::Configuration(format!(\"generate DPoP proof jti: {e}\")))?;\n    Ok(URL_SAFE_NO_PAD.encode(bytes))\n}","symbol":"new_jti"},{"citation":"identity-model@9458d60f4f2f20227bddf2594fd7d10cba66ea0a:go/internal/conformance/dpop_test.go#L23-L87","dirty":false,"indexed_at":"2026-10-20T21:12:51Z","kind":"function","license
CLI byte-identical: True
--- id 3 isError=False
{"citation":"rfc9449@sha256:2d8657f07291#section-4.3","indexed_at":"2026-10-20T21:12:53Z","instruction":"Quote this text exactly, with its citation; never paraphrase it.","kind":"section","license_class":"quote","strength":["MUST","SHOULD"],"text":"4.3. Checking DPoP Proofs\n\nTo validate a DPoP proof, the receiving server MUST ensure the following:\n\n1. There is not more than one DPoP HTTP request header field.\n2. The DPoP HTTP request header field value is a single and well-formed JWT.","title":"Checking DPoP Proofs","truncated":true}
CLI byte-identical: True
--- id 4 isError=False
{"citation":"openid-connect-core-1_0@sha256:4d016752a645#section-3.1.3.7/step-13","indexed_at":"2026-10-20T21:12:53Z","instruction":"Quote this text exactly, with its citation; never paraphrase it.","kind":"step","license_class":"quote","strength":["SHOULD"],"text":"13. If the auth_time Claim was requested, either through a specific request for this Claim or by using the max_age parameter, the Client SHOULD check the auth_time Claim value and request re-authentication if it determines too much time has elapsed since the last End-User authentication.","title":"ID Token Validation","truncated":false}
CLI byte-identical: True
--- id 5 isError=True
{"error":{"candidates":["openid-connect-backchannel-1_0@sha256:3b5c933d5269#section-2.6","rfc8725@sha256:7f93df8a106c#section-2.6","rfc9700@sha256:c00e2234fcd7#section-2.6"],"kind":"ambiguous_document","message":"§2.6 names no document and is not in exactly one current document; name one: openid-connect-backchannel-1_0@sha256:3b5c933d5269#section-2.6, rfc8725@sha256:7f93df8a106c#section-2.6, rfc9700@sha256:c00e2234fcd7#section-2.6"}}
--- id 6 isError=True
{"error":{"candidates":["identity"],"kind":"unknown_corpus","message":"unknown corpus `identty`; known corpora: identity"}}
--- id 7 isError=True
{"error":{"candidates":["rfc9449@sha256:2d8657f07291#section-4","rfc9449@sha256:2d8657f07291#section-4.3","rfc9449@sha256:2d8657f07291#section-4.2","rfc9449@sha256:2d8657f07291#section-4.1","rfc9449@sha256:2d8657f07291#section-3"],"kind":"no_such_section","message":"rfc9449@sha256:2d8657f07291 has no section 4.99; nearest: rfc9449@sha256:2d8657f07291#section-4, rfc9449@sha256:2d8657f07291#section-4.3, rfc9449@sha256:2d8657f07291#section-4.2, rfc9449@sha256:2d8657f07291#section-4.1, rfc9449@sha256:2d8657f07291#section-3"}}
--- id 8 isError=True
failed to deserialize parameters: invalid value: integer `0`, expected a nonzero u32
`````

- [ ] **Step 4: Commit.** `git add crates/codecite-mcp crates/codecite Cargo.lock` then `git commit -m "feat(mcp): add the search and read tools (#32)"`.

### Task 3: Document the tools, their errors and the shared JSON

**Files:**
- Modify: `docs/design.md`, `README.md`

- [ ] **Step 1: Apply the diff.**

`````diff
diff --git a/README.md b/README.md
index 48011d9..8684a96 100644
--- a/README.md
+++ b/README.md
@@ -25,7 +25,10 @@ citation points at under its pinned citation, for the citations search prints
 and the spec forms people write (`RFC 9449 §4.3`, `OIDC Core §3.1.3.7 step 13`,
 a spec URL). `codecite corpora` lists the indexed corpora as JSON, with each
 one's age, size, source count and staleness. `codecite mcp` runs the MCP server
-over stdio; its one tool so far, `corpora`, returns the same JSON.
+over stdio. Its tools `corpora`, `search` and `read` return the JSON the
+commands of the same name print (with `--json` for `search` and `read`), byte
+for byte, and a citation that is not in the corpus, an unknown corpus or a
+stale index comes back as a tool error with its candidates.
 `corpora/identity.toml` is the first real corpus: identity-model and the OAuth
 and OpenID specs it cites.
 
@@ -94,6 +97,13 @@ $ codecite corpora
 {"corpora":[{"age_days":0.0,"corpus":"self","indexed_at":"2026-10-20T20:49:36Z","max_age_days":30,"size_bytes":7655424,"sources":1,"stale":false}]}
 ```
 
+To let an agent query the corpora, register the server with its MCP client,
+for example Claude Code:
+
+```sh
+claude mcp add codecite -- codecite mcp
+```
+
 ## Planned
 
 - Code-aware full-text search (BM25 over split identifiers), with optional
diff --git a/docs/design.md b/docs/design.md
index fe72d74..9a0c7bc 100644
--- a/docs/design.md
+++ b/docs/design.md
@@ -274,24 +274,24 @@ The CLI and the MCP server expose the same operations and return the same JSON,
 | --- | --- |
 | `codecite add <corpus> <url>...` | Add sources to a manifest, detect licenses, pin the current commit |
 | `codecite index <corpus> [--manifest <path>] [--print-hashes]` | Fetch, index and run providers; incremental by default. Reads `corpora/<corpus>.toml` unless `--manifest` names another file. `--print-hashes` downloads each document source and prints its sha256, without checking or indexing, so a new or changed document is pinned on purpose |
-| `codecite search <corpus> "<query>" [--language <name>] [--path <glob>] [--limit <n>] [--json] [--allow-stale]` | Ranked chunks with citations, at most `--limit` (default 10). `--json` prints the result objects as a JSON array; otherwise each result is its rank and citation, a line naming its kind, symbol, score, license class and `indexed_at`, and its snippet's first 3 lines |
+| `codecite search <corpus> "<query>" [--language <name>] [--path <glob>] [--limit <n>] [--json] [--allow-stale]` | Ranked chunks with citations, at most `--limit` (default 10). `--json` prints `{"results": [...]}`, the `search` tool's JSON; otherwise each result is its rank and citation, a line naming its kind, symbol, score, license class and `indexed_at`, and its snippet's first 3 lines |
 | `codecite def <symbol>` | Where a symbol is defined |
 | `codecite refs <symbol>` | Every reference to a symbol |
 | `codecite graph <callers\|callees\|impact\|path> ...` | Graph queries with `--min-confidence` |
-| `codecite read <corpus> <citation> [--max-lines <n>] [--json] [--allow-stale]` | The exact text a citation points at, at most `--max-lines` lines (default 80), under its pinned citation. `--json` prints the result object; otherwise the citation, a line naming its kind, title, strength, license class and `indexed_at`, a blank line and the text |
+| `codecite read <corpus> <citation> [--max-lines <n>] [--json] [--allow-stale]` | The exact text a citation points at, at most `--max-lines` lines (default 80), under its pinned citation. `--json` prints the result object, the `read` tool's JSON; otherwise the citation, a line naming its kind, title, strength, license class and `indexed_at`, a blank line and the text |
 | `codecite cite <citation> [--claim <text>]` | Check a spec citation and the claim beside it |
 | `codecite corpora` | List the corpora with each one's age, size, source count and staleness, as the `corpora` tool's JSON |
 | `codecite mcp` | Run the stdio MCP server |
 
 | MCP tool | Inputs | Returns |
 | --- | --- | --- |
-| `search` | `query`, `corpus?`, `language?`, `path?`, `limit` (default 10) | Ranked results |
+| `search` | `query`, `corpus?`, `language?`, `path?`, `limit` (default 10), `allow_stale` (default false) | Ranked results |
 | `definition` | `symbol`, `corpus?` | Definition sites |
 | `references` | `symbol`, `min_confidence` | Reference sites |
 | `callers` / `callees` | `symbol`, `depth` (default 1), `min_confidence` | Edges with providers |
 | `impact` | `symbol`, `depth` (default 3), `min_confidence` (default 2) | Affected symbols, grouped by distance |
 | `path` | `from`, `to`, `max_depth` | Shortest connecting chain |
-| `read` | `citation`, `max_lines` (default 80) | Exact source text |
+| `read` | `citation`, `corpus?`, `max_lines` (default 80), `allow_stale` (default false) | Exact source text |
 | `cite` | `citation`, `claim?`, `corpus?` | `resolved`, `candidates`, `text`, `strength`, `superseded_by`, typed `findings` |
 | `corpora` | none | Available corpora and freshness |
 
@@ -314,9 +314,11 @@ Every result has the same shape:
 - Snippets are capped at 40 lines; agents call `read` for more, which keeps token use predictable.
 - Code citations look like `repo@sha:path#L10-L42`; spec citations like `rfc9449@sha256:1a2b3c4d5e6f#section-4.3` and `openid-connect-core-1_0@sha256:…#section-3.1.3.7/step-13`. The canonical forms parse strictly, so that each span has exactly one spelling: a commit is 7 to 64 lowercase hex digits, a spec hash is 12, numbers have no leading zeros, and one line is `#L10`, not `#L10-L10`. `read` and `cite` also accept the forms people write (`RFC 9449 §4.3`, `OIDC Core §3.1.3.7 step 13`, `Core 3.1.3.7(13)`, a spec URL with its anchor). These parse to an unresolved reference (a document or none, a section number or the document's own anchor, and an optional step), with document aliases taken from the manifest. A string that matches no form is no match rather than an error, and one string holds one citation: step ranges such as `steps 4-6` and lists such as `§2 / §3.1.3.7` are errors. A bare `§` resolves only when exactly one current document has that section and step.
 - `read` returns `citation`, `kind` (`code`, `section` or `step`), `text`, `truncated`, `license_class` and `indexed_at`; a spec result adds `title` and `strength` (a list of keywords), a code result `dirty`. Its citation always names the pinned version: a spec citation without one reads the current version, and a code citation's commit, which may be abbreviated, comes back in full. A step has its section's title. A code citation reads lines of the file's stored text, counted as the chunker counts them, and one that runs past the end of the file is an error naming its length. A citation that is not in the corpus is a typed error (`codecite_core::Error`) with candidates, as pinned citations, never the nearest guess: `UnknownSource` (the corpus's sources or documents), `UnknownVersion` (the versions indexed), `AmbiguousDocument` (each current document with the bare §, and when none has its step but one has the section, the missing step is reported in that one), `NoSuchSection` (the sections sharing the most leading numbers with it, then the closest), `NoSuchStep` (the closest step numbers, or the section when it has none) and `NoSuchFile` (indexed files with the same file name). A section found by its anchor whose number has no canonical form, such as an appendix's `A.1`, is `UncitableSection`. A string that parses as a canonical citation is read as one, even when its path holds `§` or the word "section". Text that matches no citation form is `NotACitation`, and a malformed one is `BadCitation` with the parse error. `read` refuses a stale index as `search` does.
+- The MCP tools answer from the corpus files in the data directory, opened read-only on tokio's blocking pool, one per call. A tool's structured content is the JSON its CLI command prints with `--json` (`corpora` always prints JSON), and its text is that JSON as `serde_json::Value::to_string` writes it, compact with sorted keys, which the CLI prints too, so the two are byte for byte the same. A tool that takes `corpus?` reads the data directory's only corpus when it is left out; with none or several, that is `NoCorpusNamed`, listing them. Arguments that don't fit the input schema, such as a `limit` of 0 or an unknown field, fail before any corpus is opened.
+- A `read` of a `quote` source adds `instruction`: "Quote this text exactly, with its citation; never paraphrase it."
 - `corpora` returns `{"corpora": [...]}`, one entry per `<corpus>.db` in the data directory, sorted by name: `corpus`, `size_bytes` (the file's), and, when the file opens, `indexed_at`, `age_days` (to two decimal places), `max_age_days`, `stale` and `sources` (each pinned document version counts as one). A file that can't be read, or whose first index run never finished, has `error` with the message and its causes instead of those fields, so one broken file never hides the others. The CLI prints it as the MCP tool's text does, `serde_json::Value` compact with sorted keys, so the two are byte for byte the same.
 - `cite` findings: `no_such_section`, `no_such_step`, `anchor_label_mismatch`, `strength_mismatch`, `superseded_document`, `topic_mismatch` (only when exactly one sibling step or section holds the claim's protocol term) and `ambiguous_document`. codecite supplies facts; the agent judges meaning.
-- Errors are explicit: an unknown corpus (with the corpora that exist), a stale index, an empty query, a citation that is not in the corpus or an ambiguous symbol name returns a typed error with candidates, never an empty success. A search that matches nothing is an empty list.
+- Errors are explicit: an unknown corpus (with the corpora that exist), a stale index, an empty query, a citation that is not in the corpus or an ambiguous symbol name returns a typed error with candidates, never an empty success. A search that matches nothing is an empty list. An MCP tool returns a typed error as a tool error (`isError`) whose structured content is `{"error": {"kind", "message", "candidates"}}`: `kind` is the `codecite_core::Error` variant in snake_case (`unknown_corpus`, `stale_index`, `ambiguous_document`, …), `message` the text the CLI prints with its causes, and `candidates` the variant's list, left out when it has none.
 
 ## Security and privacy
 
`````

- [ ] **Step 2: Gates, then commit.** `git add docs/design.md README.md` then `git commit -m "docs: describe the search and read tools and their errors (#32)"`.
