# Stdio MCP Server With the `corpora` Tool Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement #31: `codecite mcp` runs a stdio MCP server in `codecite-mcp` (`rmcp` and `tokio`) with one tool, `corpora`, and `codecite corpora` prints the same JSON: each corpus's age, size, source count and whether it is stale.

**Architecture:** `codecite-core` gains `corpora(dir) -> Result<Corpora>` (new `store/corpora.rs`), which lists every `<corpus>.db` in the data directory and opens each read-only for its freshness (the existing `Store::freshness`) and source count. `Corpora` and `CorpusInfo` derive `Serialize` and `schemars::JsonSchema`, so one Rust type is both the CLI's JSON and the MCP tool's output schema. `codecite-mcp` gains `server.rs`: a `Server` holding the data directory and an `rmcp` `ToolRouter`, a `#[tool] async fn corpora` that runs `codecite_core::corpora` on `tokio`'s blocking pool and returns `Json<Corpora>` (structured content plus the same JSON as text), and `serve_stdio`, which builds a single-threaded runtime and serves stdin/stdout until the client closes stdin. The binary stays synchronous: `codecite mcp` calls `serve_stdio`, and `codecite corpora` prints `serde_json::Value::to_string` of the same value, which is exactly the text `rmcp` sends.

**Tech Stack:** Rust 1.99.1 (edition 2024). New dependencies: `schemars` 1 (MIT) in `codecite-core`; `rmcp` 3.6 (Apache-2.0, `default-features = false`, features `server`, `macros`, `transport-io`) and `tokio` 1 (MIT, feature `rt`) in `codecite-mcp`; dev-only `rmcp` feature `client` and `tokio` feature `macros`. `cargo deny check` passes with no new exceptions or skips. Prototyped: all gates pass (297 passed, 7 ignored), and the real run below indexes this repository and drives `codecite mcp` over a pipe.

**Spec:** Issue #31; `docs/design.md#interfaces` (the `corpora` command and tool), `docs/design.md#architecture`.

## Global Constraints

- `corpora` JSON: `{"corpora": [...]}`, one entry per `<corpus>.db` (a valid corpus name) in the data directory, sorted by name. Each has `corpus` and `size_bytes` (the `.db` file's length); a readable one adds `indexed_at`, `age_days` (rounded to two decimal places), `max_age_days`, `stale` and `sources` (rows in `source`, so each pinned document version counts once). A file that can't be opened, or whose first index run never finished, has `error` (the message and its causes) instead. Fields with no value are left out. A missing data directory lists none.
- One definition per operation: `Corpora` is the result type of both the CLI and the tool, and the tool's output schema is generated from it. The tool takes no arguments, so its input schema is `rmcp`'s empty object.
- The CLI and the MCP text are byte for byte the same: both are `serde_json::Value::to_string` (keys sorted), and the CLI adds a newline.
- stdout carries only MCP messages while `codecite mcp` runs; its one note ("serving the corpora in …") goes to stderr.
- A failed `corpora` (an unreadable data directory) is a tool result with `isError: true` and the error chain as text, not a JSON-RPC error.
- The workspace forbids `unsafe_code` and warns on `missing_docs`; every public item gets a doc comment. Clippy's `result_large_err` rejects `rmcp`'s 368-byte `ServerInitializeError` in `Error`, so that variant is boxed.
- Gates: `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo deny check`.

## Review Focus

- The in-process client test (`codecite-mcp` `server::tests::a_client_lists_the_tools_and_calls_corpora`): a `tokio::io::duplex` pipe joins an `rmcp` client to the server; it lists the tools (`corpora` only, object input schema, output schema from `Corpora`) and calls `corpora` on a fixture corpus.
- The binary test (`crates/codecite/tests/mcp.rs`): JSON-RPC lines on the real binary's stdin, every stdout line parsed as JSON-RPC, the `corpora` text compared byte for byte with `codecite corpora`, a clean exit when stdin closes, and the stderr note.
- Broken corpus files: `junk.db` (not SQLite) and a never-indexed file each get `error` and no freshness, and the good corpus is still listed.
- Blocking work in async: `spawn_blocking` keeps SQLite off the runtime's only thread.

---

### Task 1: List the corpora with their freshness (`codecite-core`)

**Files:**
- Modify: `Cargo.toml` (workspace `schemars = "1"`), `crates/codecite-core/Cargo.toml`, `Cargo.lock`
- Create: `crates/codecite-core/src/store/corpora.rs`
- Modify: `crates/codecite-core/src/store.rs`, `crates/codecite-core/src/lib.rs`

> **Learn: JSON Schema.** `#[derive(JsonSchema)]` (from `schemars`) generates a JSON Schema from the struct the way `#[derive(Serialize)]` generates its JSON: the field names, their types, `Option` fields as not required, and each doc comment as the field's `description`. MCP clients read that schema to learn a tool's result shape, so the doc comments here are what an agent sees.

- [ ] **Step 1: Write the failing tests.** Create `store/corpora.rs` with only its `#[cfg(test)] mod tests` from the diff below, and add `mod corpora;` to `store.rs`. Run `cargo test -p codecite-core corpora`: it fails to compile (`corpora`, `CorpusInfo` and `Store::source_count` don't exist).
- [ ] **Step 2: Implement.** Apply the rest of the diff: the workspace and crate `schemars` dependency, the types, `corpora`, `with_causes`, `Store::source_count`, and the re-exports.

`````diff
diff --git a/Cargo.toml b/Cargo.toml
index dc9670a..0fda747 100644
--- a/Cargo.toml
+++ b/Cargo.toml
@@ -49,3 +49,4 @@ tempfile = "3"
 scraper = { version = "0.27", default-features = false }
 ego-tree = "0.11"
 proptest = { version = "1", default-features = false, features = ["std"] }
+schemars = "1"
diff --git a/crates/codecite-core/Cargo.toml b/crates/codecite-core/Cargo.toml
index 6f503e2..014f318 100644
--- a/crates/codecite-core/Cargo.toml
+++ b/crates/codecite-core/Cargo.toml
@@ -15,6 +15,7 @@ workspace = true
 
 [dependencies]
 rusqlite.workspace = true
+schemars.workspace = true
 serde.workspace = true
 thiserror.workspace = true
 
diff --git a/crates/codecite-core/src/lib.rs b/crates/codecite-core/src/lib.rs
index 538a1d6..5744efc 100644
--- a/crates/codecite-core/src/lib.rs
+++ b/crates/codecite-core/src/lib.rs
@@ -19,9 +19,9 @@ pub use keyword::{Keyword, UnknownKeyword, requirement_keywords};
 
 mod store;
 pub use store::{
-    ChunkRecord, DEFAULT_MAX_AGE_DAYS, DEFAULT_MAX_LINES, DocumentRecord, Freshness,
-    MAX_SNIPPET_LINES, ReadOptions, ReadResult, SCHEMA_VERSION, SearchQuery, SearchResult,
-    SourceRecord, Store, SymbolRecord, is_corpus_name,
+    ChunkRecord, Corpora, CorpusInfo, DEFAULT_MAX_AGE_DAYS, DEFAULT_MAX_LINES, DocumentRecord,
+    Freshness, MAX_SNIPPET_LINES, ReadOptions, ReadResult, SCHEMA_VERSION, SearchQuery,
+    SearchResult, SourceRecord, Store, SymbolRecord, corpora, is_corpus_name,
 };
 
 mod terms;
diff --git a/crates/codecite-core/src/store.rs b/crates/codecite-core/src/store.rs
index b08d6a4..8c36933 100644
--- a/crates/codecite-core/src/store.rs
+++ b/crates/codecite-core/src/store.rs
@@ -9,6 +9,9 @@ use rusqlite::{Connection, ErrorCode, OpenFlags, OptionalExtension, TransactionB
 
 use crate::{Error, Result};
 
+mod corpora;
+pub use corpora::{Corpora, CorpusInfo, corpora};
+
 mod read;
 pub use read::{DEFAULT_MAX_LINES, ReadOptions, ReadResult};
 
diff --git a/crates/codecite-core/src/store/corpora.rs b/crates/codecite-core/src/store/corpora.rs
new file mode 100644
index 0000000..dedcc8c
--- /dev/null
+++ b/crates/codecite-core/src/store/corpora.rs
@@ -0,0 +1,254 @@
+//! Listing the corpora in a data directory, with how fresh each one is.
+
+use std::path::Path;
+
+use schemars::JsonSchema;
+use serde::Serialize;
+
+use super::{Store, known_corpora};
+use crate::Result;
+
+/// The corpora in a data directory: what `codecite corpora` prints and the
+/// MCP `corpora` tool returns, as the same JSON.
+#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
+pub struct Corpora {
+    /// One entry per corpus file, sorted by name.
+    pub corpora: Vec<CorpusInfo>,
+}
+
+/// One corpus: its size and, when its file can be read, its freshness and
+/// how many sources it holds. One that can't be read has an `error`
+/// instead, so a broken file never hides the others.
+#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
+pub struct CorpusInfo {
+    /// The corpus's name, which `search` and `read` take.
+    pub corpus: String,
+    /// The corpus file's size in bytes.
+    pub size_bytes: u64,
+    /// When the last index run finished.
+    #[serde(skip_serializing_if = "Option::is_none")]
+    pub indexed_at: Option<String>,
+    /// Days since that run, to two decimal places.
+    #[serde(skip_serializing_if = "Option::is_none")]
+    pub age_days: Option<f64>,
+    /// The manifest's `max_age_days` at that run.
+    #[serde(skip_serializing_if = "Option::is_none")]
+    pub max_age_days: Option<u32>,
+    /// Whether the index is older than its maximum age, so that queries
+    /// refuse it without `allow_stale`.
+    #[serde(skip_serializing_if = "Option::is_none")]
+    pub stale: Option<bool>,
+    /// How many sources it holds; each pinned version of a document counts
+    /// as one.
+    #[serde(skip_serializing_if = "Option::is_none")]
+    pub sources: Option<u64>,
+    /// Why the corpus file can't be read.
+    #[serde(skip_serializing_if = "Option::is_none")]
+    pub error: Option<String>,
+}
+
+/// Lists every corpus in `dir`, sorted by name. A missing directory holds
+/// none.
+pub fn corpora(dir: &Path) -> Result<Corpora> {
+    let mut corpora = Vec::new();
+    for corpus in known_corpora(dir)? {
+        let path = dir.join(format!("{corpus}.db"));
+        let size_bytes = std::fs::metadata(&path)?.len();
+        let mut info = CorpusInfo {
+            corpus,
+            size_bytes,
+            indexed_at: None,
+            age_days: None,
+            max_age_days: None,
+            stale: None,
+            sources: None,
+            error: None,
+        };
+        let described = Store::open_read_only(&path)
+            .and_then(|store| Ok((store.freshness()?, store.source_count()?)));
+        match described {
+            Ok((freshness, sources)) => {
+                info.stale = Some(freshness.is_stale());
+                info.age_days = Some((freshness.age_days * 100.0).round() / 100.0);
+                info.max_age_days = Some(freshness.max_age_days);
+                info.indexed_at = Some(freshness.indexed_at);
+                info.sources = Some(sources);
+            }
+            Err(err) => info.error = Some(with_causes(&err)),
+        }
+        corpora.push(info);
+    }
+    Ok(Corpora { corpora })
+}
+
+/// `err`'s message followed by each of its causes', so that a SQLite or
+/// I/O error says what went wrong and not only where.
+fn with_causes(err: &dyn std::error::Error) -> String {
+    let mut text = err.to_string();
+    let mut cause = err.source();
+    while let Some(err) = cause {
+        text.push_str(": ");
+        text.push_str(&err.to_string());
+        cause = err.source();
+    }
+    text
+}
+
+impl Store {
+    /// How many sources the corpus holds; each pinned version of a
+    /// document counts as one.
+    pub fn source_count(&self) -> Result<u64> {
+        let count: i64 = self
+            .conn
+            .query_row("SELECT count(*) FROM source", [], |row| row.get(0))?;
+        Ok(u64::try_from(count).unwrap_or_default())
+    }
+}
+
+#[cfg(test)]
+mod tests {
+    use super::*;
+    use crate::SourceRecord;
+    use std::fs;
+
+    fn source(name: &str) -> SourceRecord<'_> {
+        SourceRecord {
+            name,
+            kind: "local",
+            url: "/src/app",
+            revision: "0123456789abcdef0123456789abcdef01234567",
+            license_spdx: Some("MIT"),
+            license_class: "adapt",
+            is_current: true,
+            obsoleted_by: None,
+            updated_by: None,
+        }
+    }
+
+    /// A corpus `name` in `dir` holding `sources` sources, indexed now.
+    fn indexed(dir: &Path, name: &str, sources: &[&str]) {
+        let mut store = Store::open_read_write(&dir.join(format!("{name}.db"))).unwrap();
+        for s in sources {
+            store.upsert_source(&source(s)).unwrap();
+        }
+        store.set_indexed_at(30).unwrap();
+    }
+
+    #[test]
+    fn a_missing_directory_holds_no_corpora() {
+        let dir = tempfile::tempdir().unwrap();
+        let listed = corpora(&dir.path().join("nowhere")).unwrap();
+        assert!(listed.corpora.is_empty());
+        let json = serde_json::to_string(&listed).unwrap();
+        assert_eq!(json, r#"{"corpora":[]}"#);
+    }
+
+    #[test]
+    fn each_corpus_has_its_age_size_sources_and_staleness() {
+        let dir = tempfile::tempdir().unwrap();
+        indexed(dir.path(), "specs", &["rfc9449"]);
+        indexed(dir.path(), "app", &["app", "lib"]);
+        // Indexed 40 days ago, past its maximum age of 30.
+        indexed(dir.path(), "old", &["app"]);
+        rusqlite::Connection::open(dir.path().join("old.db"))
+            .unwrap()
+            .execute(
+                "UPDATE meta SET value = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-40 days')
+                 WHERE key = 'indexed_at'",
+                [],
+            )
+            .unwrap();
+        // Not corpora: no `.db` extension, or a name that isn't a corpus's.
+        fs::write(dir.path().join("notes.txt"), "x").unwrap();
+        fs::write(dir.path().join(".hidden.db"), "x").unwrap();
+
+        let listed = corpora(dir.path()).unwrap().corpora;
+        let names: Vec<&str> = listed.iter().map(|c| c.corpus.as_str()).collect();
+        assert_eq!(names, ["app", "old", "specs"]);
+
+        let app = &listed[0];
+        assert_eq!(app.sources, Some(2));
+        assert_eq!(app.stale, Some(false));
+        assert_eq!(app.age_days, Some(0.0));
+        assert_eq!(app.max_age_days, Some(30));
+        assert!(app.indexed_at.as_deref().unwrap().ends_with('Z'));
+        assert_eq!(
+            app.size_bytes,
+            fs::metadata(dir.path().join("app.db")).unwrap().len()
+        );
+        assert!(app.size_bytes > 0);
+        assert_eq!(app.error, None);
+
+        let old = &listed[1];
+        assert_eq!(old.stale, Some(true));
+        assert_eq!(old.age_days, Some(40.0));
+        assert_eq!(listed[2].sources, Some(1));
+    }
+
+    #[test]
+    fn a_corpus_that_cant_be_read_has_an_error_and_no_freshness() {
+        let dir = tempfile::tempdir().unwrap();
+        indexed(dir.path(), "good", &["app"]);
+        fs::write(dir.path().join("junk.db"), "not a database at all").unwrap();
+        // Created, but its first index run never finished.
+        drop(Store::open_read_write(&dir.path().join("partial.db")).unwrap());
+
+        let listed = corpora(dir.path()).unwrap().corpora;
+        let names: Vec<&str> = listed.iter().map(|c| c.corpus.as_str()).collect();
+        assert_eq!(names, ["good", "junk", "partial"]);
+        assert_eq!(listed[0].error, None);
+        let junk = listed[1].error.as_deref().unwrap();
+        assert!(
+            junk.starts_with("SQLite error: file is not a database"),
+            "{junk}"
+        );
+        for broken in &listed[1..] {
+            assert!(broken.error.is_some(), "{broken:?}");
+            assert_eq!(
+                (broken.indexed_at.as_ref(), broken.stale, broken.sources),
+                (None, None, None)
+            );
+        }
+        assert!(
+            listed[2]
+                .error
+                .as_deref()
+                .unwrap()
+                .contains("never fully indexed"),
+            "{:?}",
+            listed[2].error
+        );
+    }
+
+    #[test]
+    fn the_json_leaves_out_fields_with_no_value() {
+        let info = CorpusInfo {
+            corpus: "junk".into(),
+            size_bytes: 21,
+            indexed_at: None,
+            age_days: None,
+            max_age_days: None,
+            stale: None,
+            sources: None,
+            error: Some("invalid corpus".into()),
+        };
+        assert_eq!(
+            serde_json::to_string(&info).unwrap(),
+            r#"{"corpus":"junk","size_bytes":21,"error":"invalid corpus"}"#
+        );
+        let info = CorpusInfo {
+            corpus: "app".into(),
+            size_bytes: 4096,
+            indexed_at: Some("2026-10-20T06:00:00Z".into()),
+            age_days: Some(1.25),
+            max_age_days: Some(30),
+            stale: Some(false),
+            sources: Some(2),
+            error: None,
+        };
+        assert_eq!(
+            serde_json::to_string(&info).unwrap(),
+            r#"{"corpus":"app","size_bytes":4096,"indexed_at":"2026-10-20T06:00:00Z","age_days":1.25,"max_age_days":30,"stale":false,"sources":2}"#
+        );
+    }
+}
`````

- [ ] **Step 3: Run the tests.** `cargo test -p codecite-core corpora` → 5 passed (4 new, plus `open_corpus_names_the_known_corpora`). Then `cargo clippy --workspace --all-targets -- -D warnings`.
- [ ] **Step 4: Commit.** `git add Cargo.toml Cargo.lock crates/codecite-core` then `git commit -m "feat(core): list the corpora with their age, size, sources and staleness (#31)"`.

### Task 2: The stdio MCP server, its `corpora` tool, and `codecite corpora` / `codecite mcp`

**Files:**
- Modify: `Cargo.toml` (workspace `rmcp`, `tokio`), `crates/codecite-mcp/Cargo.toml`, `Cargo.lock`
- Create: `crates/codecite-mcp/src/server.rs`, `crates/codecite/tests/mcp.rs`
- Modify: `crates/codecite-mcp/src/lib.rs`, `crates/codecite-mcp/src/error.rs`, `crates/codecite/src/main.rs`

> **Learn: `async`/`await` and `tokio`.** An `async fn` returns a future: a value that does nothing until a runtime polls it, and `.await` hands control back to the runtime while it waits. `tokio` is that runtime. The binary stays synchronous and `serve_stdio` builds a single-threaded runtime (`new_current_thread`) and `block_on`s the server: one client on stdio needs no thread pool. SQLite calls block the thread they run on, so the tool moves them to `tokio::task::spawn_blocking`, whose `JoinHandle` is itself a future to `.await`.

> **Learn: `rmcp` tool macros.** `#[tool_router]` on an `impl` collects its `#[tool]` methods into a `ToolRouter`, and `#[tool_handler]` implements `ServerHandler` (`tools/list`, `tools/call`, `initialize`'s server info) from it. A tool's parameters, through `Parameters<T>` where `T: JsonSchema`, become its input schema; returning `Json<T>` makes `T`'s schema its output schema and its value the structured content. `Result<_, String>` makes an `Err` a tool result with `isError: true`.

- [ ] **Step 1: Write the failing tests.** Create `server.rs` with only its `#[cfg(test)] mod tests` and `crates/codecite/tests/mcp.rs`, add `mod server;` to `lib.rs`, the `corpora_and_mcp_take_no_arguments` test to `main.rs`, and the dependencies. Run `cargo test -p codecite-mcp -p codecite`: it fails to compile (`Server`, `Command::Corpora`, `Command::Mcp` don't exist).
- [ ] **Step 2: Implement.** Apply the rest of the diff below (the Cargo.toml hunk first, then the crates).

`````diff
diff --git a/Cargo.toml b/Cargo.toml
--- a/Cargo.toml
+++ b/Cargo.toml
@@ -50,3 +50,5 @@
 ego-tree = "0.11"
 proptest = { version = "1", default-features = false, features = ["std"] }
 schemars = "1"
+rmcp = { version = "3.6", default-features = false, features = ["server", "macros", "transport-io"] }
+tokio = { version = "1", features = ["rt"] }
diff --git a/crates/codecite-mcp/Cargo.toml b/crates/codecite-mcp/Cargo.toml
index c317336..922479b 100644
--- a/crates/codecite-mcp/Cargo.toml
+++ b/crates/codecite-mcp/Cargo.toml
@@ -15,4 +15,13 @@ workspace = true
 
 [dependencies]
 codecite-core.workspace = true
+rmcp.workspace = true
 thiserror.workspace = true
+tokio.workspace = true
+
+[dev-dependencies]
+rmcp = { workspace = true, features = ["client"] }
+rusqlite.workspace = true
+serde_json.workspace = true
+tempfile.workspace = true
+tokio = { workspace = true, features = ["macros"] }
diff --git a/crates/codecite-mcp/src/error.rs b/crates/codecite-mcp/src/error.rs
index c2dc9e8..c755ac4 100644
--- a/crates/codecite-mcp/src/error.rs
+++ b/crates/codecite-mcp/src/error.rs
@@ -11,6 +11,15 @@ pub enum Error {
     /// Reading or writing stdio failed.
     #[error("I/O error")]
     Io(#[from] std::io::Error),
+
+    /// The client's opening handshake failed. Boxed, since the error is
+    /// large and every `Result` here would otherwise carry its size.
+    #[error("MCP handshake failed")]
+    Initialize(#[from] Box<rmcp::service::ServerInitializeError>),
+
+    /// The task serving the client panicked or was cancelled.
+    #[error("the MCP server stopped unexpectedly")]
+    Stopped(#[from] tokio::task::JoinError),
 }
 
 /// `Result` with [`Error`] as the default error type.
diff --git a/crates/codecite-mcp/src/lib.rs b/crates/codecite-mcp/src/lib.rs
index 265bb6b..e3f02b1 100644
--- a/crates/codecite-mcp/src/lib.rs
+++ b/crates/codecite-mcp/src/lib.rs
@@ -2,3 +2,6 @@
 
 mod error;
 pub use error::{Error, Result};
+
+mod server;
+pub use server::{Server, serve_stdio};
diff --git a/crates/codecite-mcp/src/server.rs b/crates/codecite-mcp/src/server.rs
new file mode 100644
index 0000000..9a87873
--- /dev/null
+++ b/crates/codecite-mcp/src/server.rs
@@ -0,0 +1,198 @@
+//! The MCP server: one tool per operation, each returning the JSON the
+//! matching CLI command prints.
+
+use std::path::PathBuf;
+
+use codecite_core::Corpora;
+use rmcp::handler::server::router::tool::ToolRouter;
+use rmcp::handler::server::wrapper::Json;
+use rmcp::{ServerHandler, ServiceExt as _, tool, tool_handler, tool_router};
+
+use crate::Result;
+
+/// codecite's MCP server. It answers from the corpus files in one data
+/// directory and holds no state between calls.
+#[derive(Debug, Clone)]
+pub struct Server {
+    /// Where the corpus files are: `<data dir>/<corpus>.db`.
+    data_dir: PathBuf,
+    /// The tools, built from the `#[tool]` methods below.
+    tool_router: ToolRouter<Self>,
+}
+
+impl Server {
+    /// A server answering from the corpora in `data_dir`.
+    pub fn new(data_dir: PathBuf) -> Self {
+        Server {
+            data_dir,
+            tool_router: Self::tool_router(),
+        }
+    }
+}
+
+#[tool_router]
+impl Server {
+    /// The `corpora` tool: the same JSON as `codecite corpora`.
+    #[tool(
+        description = "List the corpora this server can search: each one's age, size, source count and whether it is stale. A stale corpus is refused unless a query allows it."
+    )]
+    async fn corpora(&self) -> Result<Json<Corpora>, String> {
+        // SQLite blocks. Running it on tokio's blocking pool keeps this
+        // thread free to read and answer other messages meanwhile.
+        let dir = self.data_dir.clone();
+        match tokio::task::spawn_blocking(move || codecite_core::corpora(&dir)).await {
+            Ok(Ok(corpora)) => Ok(Json(corpora)),
+            Ok(Err(err)) => Err(with_causes(&err)),
+            Err(join) => Err(format!("listing corpora failed: {join}")),
+        }
+    }
+}
+
+/// `err`'s message followed by each of its causes'. An agent sees only
+/// this text, so it must say what went wrong and not only where.
+fn with_causes(err: &dyn std::error::Error) -> String {
+    let mut text = err.to_string();
+    let mut cause = err.source();
+    while let Some(err) = cause {
+        text.push_str(": ");
+        text.push_str(&err.to_string());
+        cause = err.source();
+    }
+    text
+}
+
+#[tool_handler(
+    router = self.tool_router,
+    name = "codecite",
+    instructions = "codecite answers from indexed code and specs, with a citation on every result. Start with `corpora` to see what is indexed and how fresh it is."
+)]
+impl ServerHandler for Server {}
+
+/// Runs the server on stdin and stdout until the client closes stdin.
+///
+/// stdout carries only MCP messages, so anything else codecite prints goes
+/// to stderr.
+pub fn serve_stdio(data_dir: PathBuf) -> Result<()> {
+    // One thread is plenty for one client, and the blocking pool still
+    // runs the SQLite work beside it.
+    let runtime = tokio::runtime::Builder::new_current_thread()
+        .enable_all()
+        .build()?;
+    runtime.block_on(async {
+        let running = Server::new(data_dir)
+            .serve(rmcp::transport::stdio())
+            .await
+            .map_err(Box::new)?;
+        running.waiting().await?;
+        Ok(())
+    })
+}
+
+#[cfg(test)]
+mod tests {
+    use super::*;
+    use codecite_core::{SourceRecord, Store};
+    use rmcp::ClientHandler;
+    use rmcp::model::{CallToolRequestParams, ClientConfig};
+
+    /// A client that only calls tools.
+    #[derive(Default)]
+    struct Client;
+
+    impl ClientHandler for Client {
+        fn get_info(&self) -> ClientConfig {
+            ClientConfig::default()
+        }
+    }
+
+    /// A data directory holding one corpus, `demo`, with one source.
+    fn fixture() -> tempfile::TempDir {
+        let dir = tempfile::tempdir().unwrap();
+        let mut store = Store::open_read_write(&dir.path().join("demo.db")).unwrap();
+        store
+            .upsert_source(&SourceRecord {
+                name: "app",
+                kind: "local",
+                url: "/src/app",
+                revision: "0123456789abcdef0123456789abcdef01234567",
+                license_spdx: Some("MIT"),
+                license_class: "adapt",
+                is_current: true,
+                obsoleted_by: None,
+                updated_by: None,
+            })
+            .unwrap();
+        store.set_indexed_at(30).unwrap();
+        dir
+    }
+
+    #[tokio::test]
+    async fn a_client_lists_the_tools_and_calls_corpora() {
+        let dir = fixture();
+        // An in-memory pipe stands in for stdio: the server reads one end,
+        // the client the other.
+        let (server_end, client_end) = tokio::io::duplex(64 * 1024);
+        let server = tokio::spawn(Server::new(dir.path().to_path_buf()).serve(server_end));
+        let client = Client.serve(client_end).await.unwrap();
+        let server = server.await.unwrap().unwrap();
+
+        let info = client.peer_info().unwrap();
+        assert_eq!(info.server_info.as_ref().unwrap().name, "codecite");
+
+        let tools = client.list_all_tools().await.unwrap();
+        let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
+        assert_eq!(names, ["corpora"]);
+        let corpora = &tools[0];
+        assert_eq!(corpora.input_schema.get("type").unwrap(), "object");
+        // The output schema comes from `Corpora`'s Rust type.
+        let output = serde_json::Value::Object((*corpora.output_schema.clone().unwrap()).clone());
+        assert_eq!(output["properties"]["corpora"]["type"], "array");
+
+        let result = client
+            .call_tool(CallToolRequestParams::new("corpora"))
+            .await
+            .unwrap();
+        assert_eq!(result.is_error, Some(false));
+        let structured = result.structured_content.unwrap();
+        assert_eq!(structured["corpora"][0]["corpus"], "demo");
+        assert_eq!(structured["corpora"][0]["sources"], 1);
+        assert_eq!(structured["corpora"][0]["stale"], false);
+        // The text is the same JSON, for clients that read only text.
+        let text = &result.content[0].as_text().unwrap().text;
+        assert_eq!(
+            serde_json::from_str::<serde_json::Value>(text).unwrap(),
+            structured
+        );
+        assert_eq!(
+            structured,
+            serde_json::to_value(codecite_core::corpora(dir.path()).unwrap()).unwrap()
+        );
+
+        client.cancel().await.unwrap();
+        server.waiting().await.unwrap();
+    }
+
+    #[tokio::test]
+    async fn an_unreadable_data_directory_is_a_tool_error() {
+        let dir = tempfile::tempdir().unwrap();
+        // A file where the directory should be.
+        let file = dir.path().join("data");
+        std::fs::write(&file, "x").unwrap();
+        let (server_end, client_end) = tokio::io::duplex(64 * 1024);
+        let server = tokio::spawn(Server::new(file).serve(server_end));
+        let client = Client.serve(client_end).await.unwrap();
+        let server = server.await.unwrap().unwrap();
+
+        let result = client
+            .call_tool(CallToolRequestParams::new("corpora"))
+            .await
+            .unwrap();
+        assert_eq!(result.is_error, Some(true));
+        let text = &result.content[0].as_text().unwrap().text;
+        assert!(text.starts_with("I/O error: "), "{text}");
+        assert!(text.len() > "I/O error: ".len(), "{text}");
+
+        client.cancel().await.unwrap();
+        server.waiting().await.unwrap();
+    }
+}
diff --git a/crates/codecite/src/main.rs b/crates/codecite/src/main.rs
index 45ecff4..260c81e 100644
--- a/crates/codecite/src/main.rs
+++ b/crates/codecite/src/main.rs
@@ -1,6 +1,7 @@
 //! The `codecite` command-line tool.
 
 use std::fs;
+use std::io::Write as _;
 use std::path::{Path, PathBuf};
 
 use anyhow::{Context as _, bail};
@@ -72,6 +73,11 @@ enum Command {
         #[arg(long)]
         allow_stale: bool,
     },
+    /// List the corpora with their age, size, source count and staleness,
+    /// as JSON.
+    Corpora,
+    /// Run the MCP server on stdin and stdout.
+    Mcp,
 }
 
 fn main() -> anyhow::Result<()> {
@@ -112,6 +118,8 @@ fn main() -> anyhow::Result<()> {
             };
             read(&corpus, &citation, &options, json)
         }
+        Command::Corpora => corpora(),
+        Command::Mcp => mcp(),
     }
 }
 
@@ -188,6 +196,32 @@ fn read(corpus: &str, citation: &str, options: &ReadOptions, json: bool) -> anyh
     to_stdout(|out| write_read(out, &result, json))
 }
 
+/// `codecite corpora`: prints the corpora as the MCP `corpora` tool returns
+/// them, byte for byte.
+fn corpora() -> anyhow::Result<()> {
+    let dirs = project_dirs()?;
+    let corpora = codecite_core::corpora(dirs.data_dir())?;
+    // rmcp writes a tool's text as `serde_json::Value::to_string`, which
+    // sorts the keys; going through a `Value` here prints the same bytes.
+    let json = serde_json::to_value(&corpora)?.to_string();
+    to_stdout(|out| writeln!(out, "{json}"))
+}
+
+/// `codecite mcp`: serves the corpora in the data directory to one MCP
+/// client over stdio until it closes stdin.
+fn mcp() -> anyhow::Result<()> {
+    let dirs = project_dirs()?;
+    let data_dir = dirs.data_dir().to_path_buf();
+    // stdout carries the protocol, so this note goes to stderr.
+    eprintln!(
+        "codecite {}: serving the corpora in {} over stdio",
+        env!("CARGO_PKG_VERSION"),
+        data_dir.display()
+    );
+    codecite_mcp::serve_stdio(data_dir)?;
+    Ok(())
+}
+
 /// Runs `write` on stdout. A reader that stops early, like `| head`, is
 /// not a failure.
 fn to_stdout(
@@ -466,6 +500,16 @@ mod tests {
         assert!(Cli::try_parse_from(zero).is_err());
     }
 
+    #[test]
+    fn corpora_and_mcp_take_no_arguments() {
+        let cli = Cli::try_parse_from(["codecite", "corpora"]).unwrap();
+        assert!(matches!(cli.command, Command::Corpora));
+        let cli = Cli::try_parse_from(["codecite", "mcp"]).unwrap();
+        assert!(matches!(cli.command, Command::Mcp));
+        assert!(Cli::try_parse_from(["codecite", "corpora", "identity"]).is_err());
+        assert!(Cli::try_parse_from(["codecite", "mcp", "--stdio"]).is_err());
+    }
+
     #[test]
     fn the_read_form_escapes_control_characters() {
         let result = ReadResult {
diff --git a/crates/codecite/tests/mcp.rs b/crates/codecite/tests/mcp.rs
new file mode 100644
index 0000000..582a907
--- /dev/null
+++ b/crates/codecite/tests/mcp.rs
@@ -0,0 +1,145 @@
+//! `codecite mcp` end to end: the real binary serving a fixture corpus under
+//! a temporary `XDG_DATA_HOME`, driven by JSON-RPC lines on its stdin.
+//! Linux only, since other platforms ignore `XDG_DATA_HOME`.
+#![cfg(target_os = "linux")]
+
+use std::io::{BufRead, BufReader, Write};
+use std::process::{Command, Stdio};
+
+use codecite_core::{SourceRecord, Store};
+use serde_json::{Value, json};
+
+/// A data home holding one corpus, `demo`, with one source.
+fn data_home() -> tempfile::TempDir {
+    let home = tempfile::tempdir().unwrap();
+    let dir = home.path().join("codecite");
+    std::fs::create_dir_all(&dir).unwrap();
+    let mut store = Store::open_read_write(&dir.join("demo.db")).unwrap();
+    store
+        .upsert_source(&SourceRecord {
+            name: "app",
+            kind: "local",
+            url: "/src/app",
+            revision: "0123456789abcdef0123456789abcdef01234567",
+            license_spdx: Some("MIT"),
+            license_class: "adapt",
+            is_current: true,
+            obsoleted_by: None,
+            updated_by: None,
+        })
+        .unwrap();
+    store.set_indexed_at(30).unwrap();
+    home
+}
+
+fn codecite() -> Command {
+    Command::new(env!("CARGO_BIN_EXE_codecite"))
+}
+
+#[test]
+fn corpora_prints_the_corpora_as_json() {
+    let home = data_home();
+    let out = codecite()
+        .arg("corpora")
+        .env("XDG_DATA_HOME", home.path())
+        .output()
+        .unwrap();
+    assert!(out.status.success(), "{out:?}");
+    let listed: Value = serde_json::from_slice(&out.stdout).unwrap();
+    assert_eq!(listed["corpora"][0]["corpus"], "demo");
+    assert_eq!(listed["corpora"][0]["sources"], 1);
+    assert_eq!(listed["corpora"][0]["stale"], false);
+    assert_eq!(listed["corpora"].as_array().unwrap().len(), 1);
+
+    let empty = tempfile::tempdir().unwrap();
+    let out = codecite()
+        .arg("corpora")
+        .env("XDG_DATA_HOME", empty.path())
+        .output()
+        .unwrap();
+    assert!(out.status.success(), "{out:?}");
+    assert_eq!(out.stdout, b"{\"corpora\":[]}\n");
+}
+
+#[test]
+fn mcp_answers_on_stdout_and_logs_on_stderr() {
+    let home = data_home();
+    let mut child = codecite()
+        .arg("mcp")
+        .env("XDG_DATA_HOME", home.path())
+        .stdin(Stdio::piped())
+        .stdout(Stdio::piped())
+        .stderr(Stdio::piped())
+        .spawn()
+        .unwrap();
+    let mut stdin = child.stdin.take().unwrap();
+    let mut stdout = BufReader::new(child.stdout.take().unwrap());
+    // Sends one message; for a request, reads lines until its response.
+    let mut send = |message: Value| -> Option<Value> {
+        writeln!(stdin, "{message}").unwrap();
+        stdin.flush().unwrap();
+        let id = message.get("id")?;
+        loop {
+            let mut line = String::new();
+            assert_ne!(stdout.read_line(&mut line).unwrap(), 0, "stdout closed");
+            // Every line on stdout is a JSON-RPC message.
+            let reply: Value = serde_json::from_str(&line)
+                .unwrap_or_else(|e| panic!("not JSON-RPC: {line:?}: {e}"));
+            assert_eq!(reply["jsonrpc"], "2.0", "{reply}");
+            if reply.get("id") == Some(id) {
+                return Some(reply);
+            }
+        }
+    };
+
+    let init = send(json!({
+        "jsonrpc": "2.0", "id": 1, "method": "initialize",
+        "params": {
+            "protocolVersion": "2025-06-18",
+            "capabilities": {},
+            "clientInfo": {"name": "test", "version": "0"}
+        }
+    }))
+    .unwrap();
+    assert_eq!(init["result"]["serverInfo"]["name"], "codecite", "{init}");
+    assert!(
+        init["result"]["capabilities"]["tools"].is_object(),
+        "{init}"
+    );
+    send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
+
+    let list = send(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})).unwrap();
+    let tools = list["result"]["tools"].as_array().unwrap();
+    assert_eq!(tools.len(), 1, "{list}");
+    assert_eq!(tools[0]["name"], "corpora");
+    assert_eq!(tools[0]["inputSchema"]["type"], "object");
+
+    let call = send(json!({
+        "jsonrpc": "2.0", "id": 3, "method": "tools/call",
+        "params": {"name": "corpora", "arguments": {}}
+    }))
+    .unwrap();
+    assert_eq!(call["result"]["isError"], false, "{call}");
+    let text = call["result"]["content"][0]["text"].as_str().unwrap();
+
+    // The CLI prints the same JSON, byte for byte.
+    let cli = codecite()
+        .arg("corpora")
+        .env("XDG_DATA_HOME", home.path())
+        .output()
+        .unwrap();
+    assert_eq!(String::from_utf8(cli.stdout).unwrap(), format!("{text}\n"));
+
+    // Closing stdin ends the server cleanly.
+    drop(stdin);
+    let out = child.wait_with_output().unwrap();
+    assert!(out.status.success(), "{out:?}");
+    let mut rest = String::new();
+    stdout.read_line(&mut rest).unwrap();
+    assert_eq!(rest, "", "stdout after the last response");
+    let stderr = String::from_utf8(out.stderr).unwrap();
+    assert!(
+        stderr.contains("serving the corpora in"),
+        "stderr: {stderr}"
+    );
+}
`````

- [ ] **Step 3: Run the tests and the real binary.** `cargo test -p codecite-mcp` → 4 passed; `cargo test -p codecite --test mcp` → 2 passed; then all the gates. Run `/var/tmp/mcp-proto/real-run.sh` (or the same by hand): it indexes this worktree as corpus `self` under a scratch `XDG_DATA_HOME`, adds a junk `broken.db`, runs `codecite corpora`, and pipes JSON-RPC lines (including a malformed one and an unknown tool) into `codecite mcp`. Expected, up to timestamps, sizes and counts:

`````text
$ codecite index self
documents: 117 written, 0 unchanged, 0 removed
chunks: 1733 written
skipped: 1 ignored, 10 symlink
elapsed: 0.86s
$ codecite corpora
{"corpora":[{"corpus":"broken","error":"SQLite error: file is not a database: Error code 26: file is not a database","size_bytes":5},{"age_days":0.0,"corpus":"self","indexed_at":"2026-10-20T20:49:36Z","max_age_days":30,"size_bytes":7655424,"sources":1,"stale":false}]}
$ codecite mcp (initialize, tools/list, a malformed line, tools/call corpora, an unknown tool)
{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-06-18","capabilities":{"tools":{}},"serverInfo":{"name":"codecite","version":"0.0.1"},"instructions":"codecite answers from indexed code and specs, with a citation on every result. Start with `corpora` to see what is indexed and how fresh it is."}}
{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"corpora","description":"List the corpora this server can search: each one's age, size, source count and whether it is stale. A stale corpus is refused unless a query allows it.","inputSchema":{"properties":{},"type":"object"},"outputSchema":{"$defs":{"CorpusInfo":{"description":"One corpus: its size and, when its file can be read, its freshness a
{"jsonrpc":"2.0","id":4,"error":{"code":-32602,"message":"tool not found"}}
{"jsonrpc":"2.0","id":3,"result":{"content":[{"type":"text","text":"{\"corpora\":[{\"corpus\":\"broken\",\"error\":\"SQLite error: file is not a database: Error code 26: file is not a database\",\"size_bytes\":5},{\"age_days\":0.0,\"corpus\":\"self\",\"indexed_at\":\"2026-10-20T20:49:36Z\",\"max_age_days\":30,\"size_bytes\":7655424,\"sources\":1,\"stale\":false}]}"}],"structuredContent":{"corpora"
exit 0
-- stderr:
codecite 0.0.1: serving the corpora in /var/tmp/mcp-proto/xdg/codecite over stdio
`````

- [ ] **Step 4: Commit.** `git add Cargo.toml Cargo.lock crates/codecite-mcp crates/codecite` then `git commit -m "feat(mcp): run a stdio MCP server with the corpora tool (#31)"`.

### Task 3: Document the server and the `corpora` JSON

**Files:**
- Modify: `docs/design.md` (Architecture: how `codecite-mcp` serves; Interfaces: the `corpora` row and its JSON), `README.md` (Status and Quick start)

- [ ] **Step 1: Apply the diff.**

`````diff
diff --git a/README.md b/README.md
index 718d42b..48011d9 100644
--- a/README.md
+++ b/README.md
@@ -23,8 +23,11 @@ and refuses an index older than the manifest's `max_age_days` unless
 `--allow-stale`. `codecite read <corpus> <citation>` prints the exact text a
 citation points at under its pinned citation, for the citations search prints
 and the spec forms people write (`RFC 9449 §4.3`, `OIDC Core §3.1.3.7 step 13`,
-a spec URL). `corpora/identity.toml` is the first real corpus: identity-model and
-the OAuth and OpenID specs it cites. The MCP server comes next.
+a spec URL). `codecite corpora` lists the indexed corpora as JSON, with each
+one's age, size, source count and staleness. `codecite mcp` runs the MCP server
+over stdio; its one tool so far, `corpora`, returns the same JSON.
+`corpora/identity.toml` is the first real corpus: identity-model and the OAuth
+and OpenID specs it cites.
 
 ## Install
 
@@ -84,6 +87,13 @@ step · ID Token Validation · SHOULD · quote · indexed 2026-10-20T20:34:20Z
 13. If the auth_time Claim was requested, either through a specific request for this Claim or by using the max_age parameter, the Client SHOULD check the auth_time Claim value and request re-authentication if it determines too much time has elapsed since the last End-User authentication.
 ```
 
+`codecite corpora` lists what is indexed, as the MCP `corpora` tool returns it:
+
+```console
+$ codecite corpora
+{"corpora":[{"age_days":0.0,"corpus":"self","indexed_at":"2026-10-20T20:49:36Z","max_age_days":30,"size_bytes":7655424,"sources":1,"stale":false}]}
+```
+
 ## Planned
 
 - Code-aware full-text search (BM25 over split identifiers), with optional
diff --git a/docs/design.md b/docs/design.md
index e4ce94d..fe72d74 100644
--- a/docs/design.md
+++ b/docs/design.md
@@ -71,6 +71,8 @@ flowchart LR
 
 The workspace has five crates: `codecite-core` (schema and query engine), `codecite-index` (pipeline and providers), `codecite-lsp`, `codecite-mcp`, and the `codecite` binary that ties them together.
 
+`codecite-mcp` serves MCP over stdio with `rmcp` on a single-threaded `tokio` runtime. Its tools are `rmcp` `#[tool]` methods: an input schema is generated from the tool's parameter type and an output schema from its result type (`schemars`), and the result types are the ones `codecite-core` returns to the CLI, so both print the same JSON. A tool runs its SQLite work on `tokio`'s blocking pool. stdout carries only MCP messages; anything else goes to stderr. A failed operation is a tool result with `isError` and the error's message and causes, so the agent can read it; an unknown tool or malformed request is a JSON-RPC error.
+
 All five publish to crates.io at one shared version, `workspace.package.version`. The internal crates require each other's exact version, since Cargo reads a `0.0.x` requirement as `=0.0.x`, so a release bumps all five together. `cargo publish --workspace` packages, verifies and uploads them in dependency order (`codecite-core`, then `codecite-index`, `codecite-lsp` and `codecite-mcp`, then `codecite`). Each package carries the root `README.md`, `LICENSE` and `NOTICE`; the crates link the last two as symlinks, which Cargo packages as files. Corpus manifests are not packaged: `codecite index <corpus>` reads `corpora/<corpus>.toml` from the current directory, or the file `--manifest` names.
 
 ## Data model
@@ -278,7 +280,7 @@ The CLI and the MCP server expose the same operations and return the same JSON,
 | `codecite graph <callers\|callees\|impact\|path> ...` | Graph queries with `--min-confidence` |
 | `codecite read <corpus> <citation> [--max-lines <n>] [--json] [--allow-stale]` | The exact text a citation points at, at most `--max-lines` lines (default 80), under its pinned citation. `--json` prints the result object; otherwise the citation, a line naming its kind, title, strength, license class and `indexed_at`, a blank line and the text |
 | `codecite cite <citation> [--claim <text>]` | Check a spec citation and the claim beside it |
-| `codecite corpora` | List corpora with age, size and staleness |
+| `codecite corpora` | List the corpora with each one's age, size, source count and staleness, as the `corpora` tool's JSON |
 | `codecite mcp` | Run the stdio MCP server |
 
 | MCP tool | Inputs | Returns |
@@ -312,6 +314,7 @@ Every result has the same shape:
 - Snippets are capped at 40 lines; agents call `read` for more, which keeps token use predictable.
 - Code citations look like `repo@sha:path#L10-L42`; spec citations like `rfc9449@sha256:1a2b3c4d5e6f#section-4.3` and `openid-connect-core-1_0@sha256:…#section-3.1.3.7/step-13`. The canonical forms parse strictly, so that each span has exactly one spelling: a commit is 7 to 64 lowercase hex digits, a spec hash is 12, numbers have no leading zeros, and one line is `#L10`, not `#L10-L10`. `read` and `cite` also accept the forms people write (`RFC 9449 §4.3`, `OIDC Core §3.1.3.7 step 13`, `Core 3.1.3.7(13)`, a spec URL with its anchor). These parse to an unresolved reference (a document or none, a section number or the document's own anchor, and an optional step), with document aliases taken from the manifest. A string that matches no form is no match rather than an error, and one string holds one citation: step ranges such as `steps 4-6` and lists such as `§2 / §3.1.3.7` are errors. A bare `§` resolves only when exactly one current document has that section and step.
 - `read` returns `citation`, `kind` (`code`, `section` or `step`), `text`, `truncated`, `license_class` and `indexed_at`; a spec result adds `title` and `strength` (a list of keywords), a code result `dirty`. Its citation always names the pinned version: a spec citation without one reads the current version, and a code citation's commit, which may be abbreviated, comes back in full. A step has its section's title. A code citation reads lines of the file's stored text, counted as the chunker counts them, and one that runs past the end of the file is an error naming its length. A citation that is not in the corpus is a typed error (`codecite_core::Error`) with candidates, as pinned citations, never the nearest guess: `UnknownSource` (the corpus's sources or documents), `UnknownVersion` (the versions indexed), `AmbiguousDocument` (each current document with the bare §, and when none has its step but one has the section, the missing step is reported in that one), `NoSuchSection` (the sections sharing the most leading numbers with it, then the closest), `NoSuchStep` (the closest step numbers, or the section when it has none) and `NoSuchFile` (indexed files with the same file name). A section found by its anchor whose number has no canonical form, such as an appendix's `A.1`, is `UncitableSection`. A string that parses as a canonical citation is read as one, even when its path holds `§` or the word "section". Text that matches no citation form is `NotACitation`, and a malformed one is `BadCitation` with the parse error. `read` refuses a stale index as `search` does.
+- `corpora` returns `{"corpora": [...]}`, one entry per `<corpus>.db` in the data directory, sorted by name: `corpus`, `size_bytes` (the file's), and, when the file opens, `indexed_at`, `age_days` (to two decimal places), `max_age_days`, `stale` and `sources` (each pinned document version counts as one). A file that can't be read, or whose first index run never finished, has `error` with the message and its causes instead of those fields, so one broken file never hides the others. The CLI prints it as the MCP tool's text does, `serde_json::Value` compact with sorted keys, so the two are byte for byte the same.
 - `cite` findings: `no_such_section`, `no_such_step`, `anchor_label_mismatch`, `strength_mismatch`, `superseded_document`, `topic_mismatch` (only when exactly one sibling step or section holds the claim's protocol term) and `ambiguous_document`. codecite supplies facts; the agent judges meaning.
 - Errors are explicit: an unknown corpus (with the corpora that exist), a stale index, an empty query, a citation that is not in the corpus or an ambiguous symbol name returns a typed error with candidates, never an empty success. A search that matches nothing is an empty list.
 
`````

- [ ] **Step 2: Gates, then commit.** `git add docs/design.md README.md` then `git commit -m "docs: describe the MCP server and the corpora JSON (#31)"`.
