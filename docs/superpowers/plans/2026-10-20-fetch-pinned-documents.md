# Fetch Pinned Documents Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement #22: `codecite index` downloads each `document` source by URL, hashes the bytes with sha256 and compares them with the manifest's pin. A mismatch stops the run with an error naming the document and both hashes. Fetched bytes are cached by hash in the user's cache directory, so a pinned document is never fetched twice and indexing works offline after the first run. `codecite index <corpus> --print-hashes` downloads each document and prints its hash without checking, so the owner pins new documents on purpose. Documents are fetched and checked, not yet indexed (#25 splits and indexes them).

**Architecture:** Three tasks.

1. `codecite-index` gets a `fetch` module: `Fetcher::new(cache_dir)` owns a `ureq::Agent` (60 s timeout, `codecite/<version>` user agent). `fetch_pinned(&DocumentSource) -> Result<Fetched, FetchError>` reads `<cache_dir>/<sha256>` when it is there and still hashes to its name, else streams the response into memory through a `Sha256` hasher, stops past `MAX_DOCUMENT_SIZE` (16 MiB), compares, and on a match writes the bytes to a temporary file in the cache directory and renames it into place. `fetch_hash` downloads without comparing and caches under the actual hash, so pinning that hash later needs no second download. A `#[cfg(test)]` HTTP/1.1 server on 127.0.0.1 (`test_server.rs`, std only) serves fixed bytes per path and counts requests.
2. `index_corpus` takes a `&Fetcher` and fetches every document source that is not `excluded` before it touches the store, so a mismatch leaves the corpus file as it was. `IndexSummary` counts documents downloaded and read from the cache, and lists each document as "fetched and checked, not indexed yet".
3. The binary computes the cache directory with `directories` (`<cache dir>/codecite/documents`, `~/.cache/codecite/documents` on Linux), passes the fetcher to `index_corpus`, and adds `--print-hashes`, which prints `<sha256>  <name>  <url>` per document (plus `  (pinned)` when it equals the pin) and opens no corpus file. design.md (Indexing step 1, Interfaces) and README follow.

**Tech Stack:** Rust 1.99.1 (edition 2024). New dependency: `ureq` 3 (MIT OR Apache-2.0) with `default-features = false, features = ["rustls"]`: a blocking client with rustls, `ring` and Mozilla's roots (`webpki-roots`), and no gzip, cookies or JSON. `tempfile` moves from dev-dependency to dependency of `codecite-index`, for the atomic cache write. `cargo deny` needs a license exception for `webpki-roots` (CDLA-Permissive-2.0, a permissive data license) and two duplicate skips, since `ring` 0.17 still pulls `getrandom` 0.2 (tempfile uses 0.4) and, on Windows only, `windows-sys` 0.52. Prototyped: all gates pass, including `cargo deny check`.

**Spec:** Issue #22; `docs/design.md#indexing-and-retrieval` (step 1), `#corpus-manifests-and-licensing`, `#security-and-privacy` (Network), `#interfaces`; `docs/superpowers/specs/2026-10-17-identity-corpus-design.md` (The corpus).

## Global Constraints

- The only network access is a GET of a manifest document's `url` (and the redirects the server answers with, ureq's default of at most 10). Excluded documents are never fetched. Git sources still do nothing (#29).
- A cached document is never fetched again: the cache file's name is the sha256 of its bytes, so it can't go stale. A cache file whose bytes no longer hash to its name (a truncated or edited file) is deleted and fetched again.
- A mismatch is an error naming the document, its URL, the pinned hash and the fetched hash, and nothing is cached for it. `--print-hashes` does cache what it fetched, under the fetched hash.
- Documents are fetched before anything is written: a mismatch, a failed download or an oversized document stops the run with the store untouched.
- The body is read in 64 KiB pieces through the hasher and stops one byte past 16 MiB, so a hostile or broken server can't make the run hold more than that.
- Cache writes go to a temporary file in the cache directory, then a rename, so a crash or a concurrent run never leaves a partial file under a hash name.
- The workspace forbids `unsafe_code` and warns on `missing_docs`; every public item gets a doc comment.
- Gates: `cargo fmt --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked && cargo deny check`.

## Review Focus

- Match, mismatch and offline cache hit, as the issue asks: `a_matching_document_is_fetched_and_cached`, `a_mismatch_names_the_document_and_both_hashes`, `a_cached_document_needs_no_network` (Task 1; the last drops the server, then shows a fresh cache fails to connect, so the hit really used no network). End to end through the binary: `documents_are_pinned_by_hash_and_cached` (Task 3).
- The store is untouched on a mismatch: `a_changed_document_stops_the_run_before_any_write` (Task 2) has a local source before the document in the manifest and checks no source row exists.
- No other network access: `excluded_documents_are_not_fetched` (Task 2) checks the server saw no request.
- Size cap: `a_document_past_the_size_limit_is_refused` (Task 1) serves exactly 16 MiB (accepted) and one byte more (refused).
- Known limits, not bugs: HTTP (not only HTTPS) URLs are allowed, since the hash pins the bytes whatever the transport; the proxy variables (`HTTPS_PROXY`, `NO_PROXY`, ...) are honoured; the cache is never pruned, so old pinned versions stay until the owner deletes them.
- Prototyped against the real network: `--print-hashes` on `https://www.rfc-editor.org/rfc/rfc9449.html` printed its hash, a zero pin failed with both hashes, the re-pinned run read the cache (`fetched: 0 downloaded, 1 from the cache`), and an unknown host failed with a lookup error.

---

### Task 1: Fetch and cache documents by hash (`codecite-index`)

**Files:**
- Modify: `Cargo.toml`, `crates/codecite-index/Cargo.toml`, `deny.toml`, `Cargo.lock`
- Create: `crates/codecite-index/src/fetch.rs`, `crates/codecite-index/src/test_server.rs`
- Modify: `crates/codecite-index/src/lib.rs`, `crates/codecite-index/src/error.rs`

- [ ] **Step 1: Add the dependencies and the cargo-deny entries**

```diff
diff --git a/Cargo.toml b/Cargo.toml
index 125601b..1581df2 100644
--- a/Cargo.toml
+++ b/Cargo.toml
@@ -34,6 +34,7 @@ serde = { version = "1", features = ["derive"] }
 toml = "1"
 globset = "0.4"
 sha2 = "0.11"
+ureq = { version = "3", default-features = false, features = ["rustls"] }
 tree-sitter = "0.27"
 tree-sitter-rust = "0.24"
 tree-sitter-python = "0.25"
```

```diff
diff --git a/crates/codecite-index/Cargo.toml b/crates/codecite-index/Cargo.toml
index 3bd466d..c51780a 100644
--- a/crates/codecite-index/Cargo.toml
+++ b/crates/codecite-index/Cargo.toml
@@ -17,6 +17,8 @@ serde.workspace = true
 toml.workspace = true
 globset.workspace = true
 sha2.workspace = true
+tempfile.workspace = true
+ureq.workspace = true
 tree-sitter.workspace = true
 tree-sitter-rust.workspace = true
 tree-sitter-python.workspace = true
@@ -28,4 +30,3 @@ streaming-iterator.workspace = true
 
 [dev-dependencies]
 rusqlite.workspace = true
-tempfile.workspace = true
```

`tempfile` leaves `[dev-dependencies]`, since a dependency is visible to tests too. Then:

```diff
diff --git a/deny.toml b/deny.toml
index 7f9e540..a4733e0 100644
--- a/deny.toml
+++ b/deny.toml
@@ -26,7 +26,12 @@ allow = [
 ]
 # The BSD and ISC entries are allowed ahead of the crates that will need them.
 unused-allowed-license = "allow"
-exceptions = [{ crate = "option-ext", allow = ["MPL-2.0"] }]
+exceptions = [
+    { crate = "option-ext", allow = ["MPL-2.0"] },
+    # Mozilla's root certificates as data, via `ureq` -> `webpki-roots`. A
+    # permissive data license; it asks only that the license text be kept.
+    { crate = "webpki-roots", allow = ["CDLA-Permissive-2.0"] },
+]
 
 [bans]
 multiple-versions = "deny"
@@ -36,6 +41,8 @@ highlight = "all"
 # version, and the upstream reason, e.g.
 # { crate = "foo@0.1.0", reason = "bar still depends on foo 0.1" }
 skip = [
+    { crate = "getrandom@0.2.17", reason = "ring 0.17 (TLS for ureq, via rustls) still depends on getrandom 0.2; tempfile uses 0.4" },
+    { crate = "windows-sys@0.52.0", reason = "ring 0.17 (TLS for ureq, via rustls) still depends on windows-sys 0.52; Windows only" },
     { crate = "hashbrown@0.16.1", reason = "rsqlite-vfs, via sqlite-wasm-rs (a wasm32-only dependency of rusqlite), still depends on hashbrown 0.16" },
 ]
```

Run `cargo build -p codecite-index` to update `Cargo.lock`, then `cargo deny check`. Expected: `advisories ok, bans ok, licenses ok, sources ok`.

- [ ] **Step 2: Write the test server and the failing tests**

Create `crates/codecite-index/src/test_server.rs`:

```rust
//! A tiny HTTP/1.1 server on 127.0.0.1 for fetch tests: serves fixed bytes
//! per path, counts requests, and stops when dropped.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

/// The routes, the request count and the stop flag, shared with the thread.
#[derive(Default)]
struct State {
    routes: Mutex<HashMap<String, Vec<u8>>>,
    requests: AtomicUsize,
    stop: AtomicBool,
}

/// A running server. Unknown paths get `404 Not Found`.
pub(crate) struct Server {
    addr: SocketAddr,
    state: Arc<State>,
    thread: Option<JoinHandle<()>>,
}

impl Server {
    pub(crate) fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let state = Arc::new(State::default());
        let shared = Arc::clone(&state);
        let thread = std::thread::spawn(move || {
            for stream in listener.incoming() {
                if shared.stop.load(Ordering::SeqCst) {
                    break;
                }
                if let Ok(stream) = stream {
                    serve(stream, &shared);
                }
            }
        });
        Server {
            addr,
            state,
            thread: Some(thread),
        }
    }

    /// Serves `body` at `path` from now on.
    pub(crate) fn set(&self, path: &str, body: impl Into<Vec<u8>>) {
        let mut routes = self.state.routes.lock().unwrap();
        routes.insert(path.to_owned(), body.into());
    }

    /// The URL of `path` on this server.
    pub(crate) fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }

    /// How many requests the server has answered.
    pub(crate) fn requests(&self) -> usize {
        self.state.requests.load(Ordering::SeqCst)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.state.stop.store(true, Ordering::SeqCst);
        // Wake the accept loop so it sees the flag.
        let _ = TcpStream::connect(self.addr);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Answers one request and closes the connection.
fn serve(stream: TcpStream, state: &State) {
    let mut reader = BufReader::new(&stream);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() {
        return;
    }
    // Skip the headers.
    let mut line = String::new();
    while reader.read_line(&mut line).is_ok_and(|n| n > 0) && line != "\r\n" {
        line.clear();
    }
    let path = request_line.split(' ').nth(1).unwrap_or("/");
    state.requests.fetch_add(1, Ordering::SeqCst);
    let body = state.routes.lock().unwrap().get(path).cloned();
    let (status, body) = match body {
        Some(body) => ("200 OK", body),
        None => ("404 Not Found", b"not found".to_vec()),
    };
    let mut stream = &stream;
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(&body);
}
```

Create `crates/codecite-index/src/fetch.rs` holding only the module doc comment, the `use` lines from Step 3 and this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::LicenseClass;
    use crate::test_server::Server;

    const SPEC: &[u8] = b"<html><h1>RFC 9449</h1></html>\n";

    fn sha256(bytes: &[u8]) -> String {
        hex(&Sha256::digest(bytes))
    }

    fn document(url: String, sha256: String) -> DocumentSource {
        DocumentSource {
            name: "rfc9449".into(),
            url,
            sha256,
            license: None,
            license_class: LicenseClass::Quote,
            current: true,
            aliases: Vec::new(),
            obsoleted_by: None,
            updated_by: None,
        }
    }

    #[test]
    fn hex_is_lowercase_and_two_digits_per_byte() {
        assert_eq!(hex(&[0x00, 0x0f, 0xab]), "000fab");
        assert_eq!(
            sha256(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn a_matching_document_is_fetched_and_cached() {
        let server = Server::start();
        server.set("/rfc9449.html", SPEC);
        let cache = tempfile::tempdir().unwrap();
        let fetcher = Fetcher::new(cache.path().join("documents"));
        let doc = document(server.url("/rfc9449.html"), sha256(SPEC));

        let fetched = fetcher.fetch_pinned(&doc).unwrap();
        assert_eq!(fetched.bytes, SPEC);
        assert!(!fetched.from_cache);
        let cached = cache.path().join("documents").join(&doc.sha256);
        assert_eq!(fs::read(cached).unwrap(), SPEC);

        let again = fetcher.fetch_pinned(&doc).unwrap();
        assert_eq!(again.bytes, SPEC);
        assert!(again.from_cache);
        assert_eq!(server.requests(), 1);
    }

    #[test]
    fn a_mismatch_names_the_document_and_both_hashes() {
        let server = Server::start();
        server.set("/rfc9449.html", SPEC);
        let cache = tempfile::tempdir().unwrap();
        let fetcher = Fetcher::new(cache.path());
        let pinned = sha256(b"the version that was pinned");
        let doc = document(server.url("/rfc9449.html"), pinned.clone());

        let err = fetcher.fetch_pinned(&doc).unwrap_err();
        let FetchError::Mismatch {
            name,
            expected,
            actual,
            ..
        } = &err
        else {
            panic!("{err:?}");
        };
        assert_eq!(
            (name.as_str(), expected, actual),
            ("rfc9449", &pinned, &sha256(SPEC))
        );
        let message = err.to_string();
        assert!(message.contains("`rfc9449`"), "{message}");
        assert!(message.contains(&pinned), "{message}");
        assert!(message.contains(&sha256(SPEC)), "{message}");
        // Nothing is cached under either hash.
        assert_eq!(fs::read_dir(cache.path()).unwrap().count(), 0);
    }

    #[test]
    fn a_cached_document_needs_no_network() {
        let cache = tempfile::tempdir().unwrap();
        let server = Server::start();
        server.set("/rfc9449.html", SPEC);
        let doc = document(server.url("/rfc9449.html"), sha256(SPEC));
        Fetcher::new(cache.path()).fetch_pinned(&doc).unwrap();
        drop(server);

        // The server is gone, so a download would fail to connect.
        let fetched = Fetcher::new(cache.path()).fetch_pinned(&doc).unwrap();
        assert_eq!(fetched.bytes, SPEC);
        assert!(fetched.from_cache);

        let empty = tempfile::tempdir().unwrap();
        let err = Fetcher::new(empty.path()).fetch_pinned(&doc).unwrap_err();
        assert!(matches!(err, FetchError::Download { .. }), "{err:?}");
    }

    #[test]
    fn a_corrupt_cache_file_is_fetched_again() {
        let server = Server::start();
        server.set("/rfc9449.html", SPEC);
        let cache = tempfile::tempdir().unwrap();
        let doc = document(server.url("/rfc9449.html"), sha256(SPEC));
        fs::write(cache.path().join(&doc.sha256), b"truncated").unwrap();

        let fetched = Fetcher::new(cache.path()).fetch_pinned(&doc).unwrap();
        assert!(!fetched.from_cache);
        assert_eq!(fs::read(cache.path().join(&doc.sha256)).unwrap(), SPEC);
    }

    #[test]
    fn an_error_status_is_a_download_error() {
        let server = Server::start();
        let cache = tempfile::tempdir().unwrap();
        let doc = document(server.url("/gone.html"), sha256(SPEC));

        let err = Fetcher::new(cache.path()).fetch_pinned(&doc).unwrap_err();
        assert!(matches!(err, FetchError::Download { .. }), "{err:?}");
        let source = std::error::Error::source(&err).unwrap().to_string();
        assert!(source.contains("404"), "{source}");
    }

    #[test]
    fn a_document_past_the_size_limit_is_refused() {
        let server = Server::start();
        let size = usize::try_from(MAX_DOCUMENT_SIZE).unwrap();
        server.set("/exact.html", vec![b'a'; size]);
        server.set("/big.html", vec![b'a'; size + 1]);
        let cache = tempfile::tempdir().unwrap();
        let fetcher = Fetcher::new(cache.path());

        let exact = fetcher.fetch_hash(&document(server.url("/exact.html"), String::new()));
        assert_eq!(exact.unwrap(), sha256(&vec![b'a'; size]));
        let err = fetcher
            .fetch_hash(&document(server.url("/big.html"), String::new()))
            .unwrap_err();
        assert!(matches!(err, FetchError::TooLarge { .. }), "{err:?}");
    }

    #[test]
    fn fetch_hash_reports_what_the_url_serves_now() {
        let server = Server::start();
        server.set("/rfc9449.html", SPEC);
        let cache = tempfile::tempdir().unwrap();
        let fetcher = Fetcher::new(cache.path());
        let doc = document(server.url("/rfc9449.html"), sha256(b"an old pin"));

        assert_eq!(fetcher.fetch_hash(&doc).unwrap(), sha256(SPEC));
        // Pinning the printed hash then needs no second download.
        let repinned = document(doc.url.clone(), sha256(SPEC));
        assert!(fetcher.fetch_pinned(&repinned).unwrap().from_cache);
        assert_eq!(server.requests(), 1);
    }
}
```

Wire the modules into `lib.rs` and the error into `error.rs`:

```diff
diff --git a/crates/codecite-index/src/lib.rs b/crates/codecite-index/src/lib.rs
index 096072f..055750f 100644
--- a/crates/codecite-index/src/lib.rs
+++ b/crates/codecite-index/src/lib.rs
@@ -3,8 +3,10 @@
 //! It also reads corpus manifests, which say what each corpus indexes, walks
 //! local checkouts, listing the files git sees and reporting every file it
 //! skips, and splits files into chunks: code at definitions, Markdown at
-//! headings, anything else into 60-line windows. [`index_corpus`] runs those
-//! steps over a corpus's local sources and writes the result into its store.
+//! headings, anything else into 60-line windows. [`Fetcher`] downloads document
+//! sources and checks them against their pinned sha256, caching them by hash.
+//! [`index_corpus`] fetches a corpus's documents, runs the other steps over
+//! its local sources and writes the result into its store.
 
 mod chunk;
 pub use chunk::{
@@ -14,6 +16,9 @@ pub use chunk::{
 mod error;
 pub use error::{Error, Result};
 
+mod fetch;
+pub use fetch::{FetchError, Fetched, Fetcher, MAX_DOCUMENT_SIZE};
+
 mod index;
 pub use index::{IndexSummary, index_corpus};
 
@@ -25,6 +30,9 @@ pub use manifest::{
 #[cfg(test)]
 mod test_repo;
 
+#[cfg(test)]
+mod test_server;
+
 mod walk;
 pub use walk::{
     DEFAULT_MAX_FILE_SIZE, LocalFile, LocalTree, SkipReason, SkippedFile, WalkError, WalkOptions,
```

```diff
diff --git a/crates/codecite-index/src/error.rs b/crates/codecite-index/src/error.rs
index e2f638c..1fcb0bf 100644
--- a/crates/codecite-index/src/error.rs
+++ b/crates/codecite-index/src/error.rs
@@ -24,6 +24,11 @@ pub enum Error {
     #[error(transparent)]
     Chunker(#[from] crate::ChunkerError),
 
+    /// A document source could not be fetched, or its bytes don't match
+    /// the manifest's pinned `sha256`.
+    #[error(transparent)]
+    Fetch(#[from] crate::FetchError),
+
     /// A walked file could not be read.
     #[error("reading {}", path.display())]
     ReadFile {
```

Run: `cargo test -p codecite-index fetch`
Expected: FAIL to compile (`Fetcher`, `FetchError`, `hex` not found).

> **Learn: `sha2`.** `Sha256` implements the `Digest` trait: `Sha256::digest(bytes)` hashes a slice in one call, and `Sha256::new()`, `update(piece)` and `finalize()` hash a stream piece by piece, with the same result. `finalize` returns a fixed-size array of 32 bytes; the manifest writes it as 64 lowercase hex digits, which `hex` produces with `write!(s, "{b:02x}")` (two digits, zero-padded, per byte).

- [ ] **Step 3: Implement the fetcher**

The whole of `fetch.rs` above the test module:

```rust
//! Document sources: fetch each one by URL, check its bytes against the
//! manifest's pinned sha256, and keep them in a cache named by that hash, so
//! a pinned document is downloaded once and indexing then works offline.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::DocumentSource;

/// The largest document a fetch accepts: 16 MiB. The largest RFC in HTML is
/// a few MiB; anything bigger is not a spec.
pub const MAX_DOCUMENT_SIZE: u64 = 16 * 1024 * 1024;

/// How long one download may take, from connecting to the last byte.
const TIMEOUT: Duration = Duration::from_secs(60);

/// Fetches document sources over HTTP(S) and caches them by content hash.
///
/// The cache holds one file per document, named by the lowercase hex sha256
/// of its bytes. Since the name is the hash, a cached file can't go stale: a
/// pinned document is read from the cache whenever its hash is there.
#[derive(Debug)]
pub struct Fetcher {
    cache_dir: PathBuf,
    agent: ureq::Agent,
}

/// A document's bytes and where they came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fetched {
    /// The document's bytes, exactly as served.
    pub bytes: Vec<u8>,
    /// Whether the bytes came from the cache rather than the network.
    pub from_cache: bool,
}

/// Why a document could not be fetched or did not match its pin.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum FetchError {
    /// The fetched bytes do not hash to the manifest's `sha256`.
    #[error(
        "document `{name}` from {url} has sha256 {actual}, but the manifest pins {expected}; \
         check what changed, then update `sha256` (`codecite index <corpus> --print-hashes` prints it)"
    )]
    Mismatch {
        /// The document's name.
        name: String,
        /// Where it was fetched from.
        url: String,
        /// The hash the manifest pins.
        expected: String,
        /// The hash of the fetched bytes.
        actual: String,
    },

    /// The request failed, the server answered with an error status, or the
    /// connection broke while reading the body.
    #[error("fetching document `{name}` from {url}")]
    Download {
        /// The document's name.
        name: String,
        /// Where it was fetched from.
        url: String,
        /// What went wrong.
        source: io::Error,
    },

    /// The document is larger than [`MAX_DOCUMENT_SIZE`].
    #[error("document `{name}` from {url} is larger than {limit} bytes")]
    TooLarge {
        /// The document's name.
        name: String,
        /// Where it was fetched from.
        url: String,
        /// The size limit in bytes.
        limit: u64,
    },

    /// Reading or writing the document cache failed.
    #[error("document cache {}", path.display())]
    Cache {
        /// The cache file or directory.
        path: PathBuf,
        /// What went wrong.
        source: io::Error,
    },
}

impl Fetcher {
    /// A fetcher that caches documents in `cache_dir`, created on first use.
    pub fn new(cache_dir: impl Into<PathBuf>) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(TIMEOUT))
            .user_agent(concat!("codecite/", env!("CARGO_PKG_VERSION")))
            .build()
            .into();
        Fetcher {
            cache_dir: cache_dir.into(),
            agent,
        }
    }

    /// The document's pinned bytes: from the cache when they are there, else
    /// downloaded and checked against `doc.sha256`. A mismatch is an error
    /// and nothing is cached.
    pub fn fetch_pinned(&self, doc: &DocumentSource) -> Result<Fetched, FetchError> {
        let cached = self.cache_dir.join(&doc.sha256);
        if let Some(bytes) = self.read_cached(&cached)? {
            return Ok(Fetched {
                bytes,
                from_cache: true,
            });
        }
        let download = self.download(doc)?;
        if download.hash != doc.sha256 {
            return Err(FetchError::Mismatch {
                name: doc.name.clone(),
                url: doc.url.clone(),
                expected: doc.sha256.clone(),
                actual: download.hash,
            });
        }
        let bytes = download.keep(&self.cache_dir)?;
        Ok(Fetched {
            bytes,
            from_cache: false,
        })
    }

    /// Downloads the document now and returns the sha256 of what the URL
    /// serves, without comparing it to the pin. The bytes are cached under
    /// that hash, so pinning it later needs no second download.
    pub fn fetch_hash(&self, doc: &DocumentSource) -> Result<String, FetchError> {
        let download = self.download(doc)?;
        let hash = download.hash.clone();
        download.keep(&self.cache_dir)?;
        Ok(hash)
    }

    /// A cached document's bytes, or `None` when it isn't cached. A cached
    /// file whose bytes no longer hash to its name is deleted and treated as
    /// missing.
    fn read_cached(&self, path: &Path) -> Result<Option<Vec<u8>>, FetchError> {
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(source) => {
                return Err(FetchError::Cache {
                    path: path.to_owned(),
                    source,
                });
            }
        };
        let name = path.file_name().and_then(|n| n.to_str());
        if name == Some(hex(&Sha256::digest(&bytes)).as_str()) {
            return Ok(Some(bytes));
        }
        fs::remove_file(path).map_err(|source| FetchError::Cache {
            path: path.to_owned(),
            source,
        })?;
        Ok(None)
    }

    /// Streams the response body into memory, hashing it on the way, and
    /// stops as soon as it passes [`MAX_DOCUMENT_SIZE`].
    fn download(&self, doc: &DocumentSource) -> Result<Download, FetchError> {
        let failed = |source: io::Error| FetchError::Download {
            name: doc.name.clone(),
            url: doc.url.clone(),
            source,
        };
        let response = self
            .agent
            .get(&doc.url)
            .call()
            .map_err(|e| failed(e.into_io()))?;
        // One byte past the limit tells a document of exactly the limit
        // from a larger one.
        let mut body = response
            .into_body()
            .into_reader()
            .take(MAX_DOCUMENT_SIZE + 1);

        let mut hasher = Sha256::new();
        let mut bytes = Vec::new();
        let mut buffer = [0; 64 * 1024];
        loop {
            let n = match body.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(failed(e)),
            };
            hasher.update(&buffer[..n]);
            bytes.extend_from_slice(&buffer[..n]);
        }
        if bytes.len() as u64 > MAX_DOCUMENT_SIZE {
            return Err(FetchError::TooLarge {
                name: doc.name.clone(),
                url: doc.url.clone(),
                limit: MAX_DOCUMENT_SIZE,
            });
        }
        Ok(Download {
            bytes,
            hash: hex(&hasher.finalize()),
        })
    }
}

/// A finished download: the bytes and their hash.
struct Download {
    bytes: Vec<u8>,
    hash: String,
}

impl Download {
    /// Writes the bytes into the cache under their hash and returns them.
    /// The file is written under a temporary name and renamed into place,
    /// so a crash or a concurrent run never leaves a partial cache file.
    fn keep(self, cache_dir: &Path) -> Result<Vec<u8>, FetchError> {
        let cache_error = |path: &Path| {
            let path = path.to_owned();
            move |source| FetchError::Cache { path, source }
        };
        fs::create_dir_all(cache_dir).map_err(cache_error(cache_dir))?;
        let mut file = tempfile::Builder::new()
            .prefix(".download-")
            .tempfile_in(cache_dir)
            .map_err(cache_error(cache_dir))?;
        file.write_all(&self.bytes)
            .map_err(cache_error(file.path()))?;
        let path = cache_dir.join(&self.hash);
        file.persist(&path)
            .map_err(|e| cache_error(&path)(e.error))?;
        Ok(self.bytes)
    }
}

/// Lowercase hex, the form the manifest pins.
fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}
```

> **Learn: streaming reads.** `Read::read` fills part of a buffer and returns how many bytes it wrote; `0` means the end. Reading a fixed 64 KiB buffer in a loop lets the hasher see each piece as it arrives, and `Read::take(limit + 1)` stops the reader one byte past the limit, so an endless or huge body costs at most 16 MiB. `ErrorKind::Interrupted` is retried, as `read_to_end` does. `persist` renames the temporary file into place, which is atomic on one file system, so a reader sees either no file or the whole file.

> **Learn: why a content hash beats an ETag.** An ETag is an opaque tag the server picks: it can change when the bytes don't (a new server, a re-encoding) and stay the same when they do, and it can't be checked offline or by anyone without the server. The sha256 is computed here from the bytes themselves, so the manifest's pin says exactly which bytes every user indexes, and a changed document is caught whatever the server claims.

Run: `cargo test -p codecite-index fetch`
Expected: PASS (8 fetch tests).

- [ ] **Step 4: Gates and commit**

Run: `cargo fmt --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked && cargo deny check`
Expected: all green.

```bash
git add Cargo.toml Cargo.lock deny.toml crates/codecite-index/Cargo.toml crates/codecite-index/src/fetch.rs crates/codecite-index/src/test_server.rs crates/codecite-index/src/lib.rs crates/codecite-index/src/error.rs
git commit -m "feat(index): fetch documents and pin them by sha256 (#22)"
```

`index_corpus` doesn't call the fetcher yet, but nothing warns: `Fetcher` is public.

---

### Task 2: Fetch every document before indexing (`codecite-index`)

**Files:**
- Modify: `crates/codecite-index/src/index.rs`

- [ ] **Step 1: Write the failing tests**

The test-module half of this diff: the `document` helper, `SPEC` and `SPEC_SHA256`, `Corpus::try_index` (which builds a `Fetcher` in the corpus's temporary directory), the three new tests, and the `downloaded`/`cached` lines in `summary_prints_each_count`. The `index_corpus` call in `the_name_defaults_to_the_directory_and_must_be_unique` moves to `try_index`.

```diff
diff --git a/crates/codecite-index/src/index.rs b/crates/codecite-index/src/index.rs
index 03f8ff1..f8118c3 100644
--- a/crates/codecite-index/src/index.rs
+++ b/crates/codecite-index/src/index.rs
@@ -11,8 +11,8 @@ use codecite_core::{ChunkRecord, DocumentRecord, SourceRecord, Store, SymbolReco
 use sha2::{Digest, Sha256};
 
 use crate::{
-    Chunker, Error, Language, LicenseClass, LocalSource, LocalTree, Manifest, Result, SkipReason,
-    Source, WalkOptions, walk_local,
+    Chunker, Error, Fetcher, Language, LicenseClass, LocalSource, LocalTree, Manifest, Result,
+    SkipReason, Source, WalkOptions, walk_local,
 };
 
 /// What an index run did.
@@ -26,6 +26,10 @@ pub struct IndexSummary {
     pub removed: usize,
     /// Chunks written, over every written document.
     pub chunks: usize,
+    /// Document sources downloaded and checked against their pin.
+    pub downloaded: usize,
+    /// Document sources read from the cache.
+    pub cached: usize,
     /// Files not indexed, counted by reason.
     pub skipped: BTreeMap<SkipReason, usize>,
     /// Manifest sources not indexed, with the reason.
@@ -36,20 +40,37 @@ pub struct IndexSummary {
     pub elapsed: Duration,
 }
 
-/// Indexes every local source in `manifest` into `store`.
+/// Fetches every document source in `manifest` and indexes every local
+/// source into `store`.
 ///
-/// A document whose content hash matches the stored one is not rewritten,
-/// and a stored document whose file is gone, or is now skipped, is deleted.
-/// Git and document sources are reported in
-/// [`IndexSummary::skipped_sources`], as are sources marked `excluded`.
-/// Every source the run did not index is then deleted from the store.
+/// Documents are fetched first, from `fetcher`'s cache when they are there,
+/// so a document whose bytes don't match its pinned `sha256` stops the run
+/// before anything is written. Documents are not indexed yet.
+///
+/// A file whose content hash matches the stored one is not rewritten, and a
+/// stored file that is gone, or is now skipped, is deleted. Git and document
+/// sources are reported in [`IndexSummary::skipped_sources`], as are sources
+/// marked `excluded`. Every source the run did not index is then deleted
+/// from the store.
 pub fn index_corpus(
     store: &mut Store,
     manifest: &Manifest,
     options: &WalkOptions,
+    fetcher: &Fetcher,
 ) -> Result<IndexSummary> {
     let started = Instant::now();
     let mut summary = IndexSummary::default();
+    for source in &manifest.sources {
+        if let Source::Document(doc) = source
+            && doc.license_class != LicenseClass::Excluded
+        {
+            if fetcher.fetch_pinned(doc)?.from_cache {
+                summary.cached += 1;
+            } else {
+                summary.downloaded += 1;
+            }
+        }
+    }
     let mut chunker = Chunker::new()?;
     let mut indexed: Vec<String> = Vec::new();
     for source in &manifest.sources {
@@ -71,9 +92,12 @@ pub fn index_corpus(
                 continue;
             }
             Source::Document(doc) => {
-                summary
-                    .skipped_sources
-                    .push((doc.name.clone(), "document sources are not indexed yet"));
+                let reason = if doc.license_class == LicenseClass::Excluded {
+                    "use = \"excluded\""
+                } else {
+                    "document sources are fetched and checked, not indexed yet"
+                };
+                summary.skipped_sources.push((doc.name.clone(), reason));
                 continue;
             }
         };
@@ -211,6 +235,13 @@ impl fmt::Display for IndexSummary {
             self.written, self.unchanged, self.removed
         )?;
         writeln!(f, "chunks: {} written", self.chunks)?;
+        if self.downloaded + self.cached > 0 {
+            writeln!(
+                f,
+                "fetched: {} downloaded, {} from the cache",
+                self.downloaded, self.cached
+            )?;
+        }
         if !self.skipped.is_empty() {
             let parts: Vec<String> = self
                 .skipped
@@ -232,7 +263,9 @@ impl fmt::Display for IndexSummary {
 #[cfg(test)]
 mod tests {
     use super::*;
+    use crate::DocumentSource;
     use crate::test_repo::Repo;
+    use crate::test_server::Server;
     use rusqlite::Connection;
     use std::path::PathBuf;
 
@@ -247,6 +280,24 @@ mod tests {
         })
     }
 
+    fn document(url: String, sha256: &str) -> Source {
+        Source::Document(DocumentSource {
+            name: "rfc9449".into(),
+            url,
+            sha256: sha256.into(),
+            license: None,
+            license_class: LicenseClass::Quote,
+            current: true,
+            aliases: Vec::new(),
+            obsoleted_by: None,
+            updated_by: None,
+        })
+    }
+
+    const SPEC: &str = "<h1>RFC 9449</h1>\n";
+    /// The sha256 of [`SPEC`].
+    const SPEC_SHA256: &str = "adcd06b613eb4e660e403fbec3d11011983dec4501489279e191b728e2fe6b2b";
+
     fn manifest(sources: Vec<Source>) -> Manifest {
         Manifest {
             name: "test".into(),
@@ -285,8 +336,13 @@ mod tests {
         }
 
         fn index(&self, manifest: &Manifest) -> IndexSummary {
+            self.try_index(manifest).unwrap()
+        }
+
+        fn try_index(&self, manifest: &Manifest) -> Result<IndexSummary> {
             let mut store = Store::open_read_write(&self.path()).unwrap();
-            index_corpus(&mut store, manifest, &WalkOptions::default()).unwrap()
+            let fetcher = Fetcher::new(self.dir.path().join("cache"));
+            index_corpus(&mut store, manifest, &WalkOptions::default(), &fetcher)
         }
 
         fn sql(&self) -> Connection {
@@ -483,15 +539,74 @@ mod tests {
             .unwrap();
         assert_eq!(name, dir.file_name().unwrap().to_str().unwrap());
 
-        let mut store = Store::open_read_write(&corpus.path()).unwrap();
         let twice = manifest(vec![local(&repo, Some("app")), local(&repo, Some("app"))]);
-        let err = index_corpus(&mut store, &twice, &WalkOptions::default()).unwrap_err();
+        let err = corpus.try_index(&twice).unwrap_err();
         assert!(
             matches!(&err, Error::DuplicateSource { name } if name == "app"),
             "{err:?}"
         );
     }
 
+    #[test]
+    fn documents_are_fetched_once_and_not_indexed_yet() {
+        let server = Server::start();
+        server.set("/rfc9449.html", SPEC);
+        let corpus = Corpus::new();
+        let manifest = manifest(vec![document(server.url("/rfc9449.html"), SPEC_SHA256)]);
+
+        let first = corpus.index(&manifest);
+        assert_eq!((first.downloaded, first.cached), (1, 0));
+        assert_eq!(
+            first.skipped_sources,
+            [(
+                "rfc9449".to_owned(),
+                "document sources are fetched and checked, not indexed yet"
+            )]
+        );
+        let second = corpus.index(&manifest);
+        assert_eq!((second.downloaded, second.cached), (0, 1));
+        assert_eq!(server.requests(), 1);
+    }
+
+    #[test]
+    fn a_changed_document_stops_the_run_before_any_write() {
+        let repo = fixture();
+        let server = Server::start();
+        server.set("/rfc9449.html", "<h1>RFC 9449, edited</h1>\n");
+        let corpus = Corpus::new();
+        let manifest = manifest(vec![
+            local(&repo, Some("app")),
+            document(server.url("/rfc9449.html"), SPEC_SHA256),
+        ]);
+
+        let err = corpus.try_index(&manifest).unwrap_err();
+        assert!(
+            matches!(&err, Error::Fetch(crate::FetchError::Mismatch { name, .. }) if name == "rfc9449"),
+            "{err:?}"
+        );
+        let sources: i64 = corpus
+            .sql()
+            .query_row("SELECT count(*) FROM source", [], |row| row.get(0))
+            .unwrap();
+        assert_eq!(sources, 0);
+    }
+
+    #[test]
+    fn excluded_documents_are_not_fetched() {
+        let server = Server::start();
+        let corpus = Corpus::new();
+        let mut doc = document(server.url("/rfc9449.html"), SPEC_SHA256);
+        if let Source::Document(d) = &mut doc {
+            d.license_class = LicenseClass::Excluded;
+        }
+        let summary = corpus.index(&manifest(vec![doc]));
+        assert_eq!(server.requests(), 0);
+        assert_eq!(
+            summary.skipped_sources,
+            [("rfc9449".to_owned(), "use = \"excluded\"")]
+        );
+    }
+
     #[test]
     fn summary_prints_each_count() {
         let summary = IndexSummary {
@@ -499,6 +614,8 @@ mod tests {
             unchanged: 5,
             removed: 1,
             chunks: 9,
+            downloaded: 1,
+            cached: 2,
             skipped: BTreeMap::from([(SkipReason::Ignored, 3), (SkipReason::Binary, 1)]),
             skipped_sources: vec![("rfc9449".into(), "document sources are not indexed yet")],
             removed_sources: vec!["old".into()],
@@ -508,6 +625,7 @@ mod tests {
             summary.to_string(),
             "documents: 2 written, 5 unchanged, 1 removed\n\
              chunks: 9 written\n\
+             fetched: 1 downloaded, 2 from the cache\n\
              skipped: 3 ignored, 1 binary\n\
              source rfc9449 not indexed: document sources are not indexed yet\n\
              source old removed: the run no longer indexes it\n\
```

Run: `cargo test -p codecite-index index::`
Expected (tests only applied): FAIL to compile (`index_corpus` takes 3 arguments; no field `downloaded`).

- [ ] **Step 2: Implement**

Apply the rest of the diff above: the `Fetcher` import, the two summary fields, the fetch loop at the top of `index_corpus` (a let chain: `if let Source::Document(doc) = source && doc.license_class != LicenseClass::Excluded`), the document reasons in the source loop, and the `fetched:` summary line, printed only when the run fetched any document.

Run: `cargo test -p codecite-index`
Expected: PASS (84 tests).

- [ ] **Step 3: Commit**

The binary doesn't compile until Task 3 passes the fetcher, so run the gates for this crate only: `cargo fmt --check && cargo clippy -p codecite-index --all-targets --locked -- -D warnings && cargo test -p codecite-index --locked`.

```bash
git add crates/codecite-index/src/index.rs
git commit -m "feat(index): check every document's pin before indexing (#22)"
```

---

### Task 3: `codecite index --print-hashes`, the cache directory and the docs

**Files:**
- Modify: `crates/codecite/src/main.rs`, `crates/codecite/tests/index.rs`, `docs/design.md`, `README.md`

- [ ] **Step 1: Write the failing end-to-end test**

`codecite` now also runs with `XDG_CACHE_HOME` under the test's home. `serve` answers exactly `requests` requests and then drops its listener, so a later download fails to connect: the last two runs prove the cache made them offline.

```diff
diff --git a/crates/codecite/tests/index.rs b/crates/codecite/tests/index.rs
index 398ce1b..85dc91c 100644
--- a/crates/codecite/tests/index.rs
+++ b/crates/codecite/tests/index.rs
@@ -4,8 +4,11 @@
 #![cfg(target_os = "linux")]
 
 use std::fs;
+use std::io::{BufRead, BufReader, Write};
+use std::net::TcpListener;
 use std::path::Path;
 use std::process::{Command, Output};
+use std::thread::JoinHandle;
 
 fn git(dir: &Path, args: &[&str]) {
     let status = Command::new("git")
@@ -33,6 +36,7 @@ fn codecite(cwd: &Path, home: &Path, args: &[&str]) -> Output {
         .current_dir(cwd)
         .env("HOME", home)
         .env("XDG_DATA_HOME", home.join("data"))
+        .env("XDG_CACHE_HOME", home.join("cache"))
         .args(args)
         .output()
         .unwrap()
@@ -109,3 +113,84 @@ fn index_errors_name_the_problem() {
     assert!(!mismatch.status.success());
     assert!(String::from_utf8_lossy(&mismatch.stderr).contains("is for corpus `other`"));
 }
+
+/// Serves `body` on 127.0.0.1 for exactly `requests` requests, then stops,
+/// so any later download fails to connect. Returns the URL and the thread.
+fn serve(body: &'static [u8], requests: usize) -> (String, JoinHandle<()>) {
+    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
+    let url = format!("http://{}/rfc9449.html", listener.local_addr().unwrap());
+    let thread = std::thread::spawn(move || {
+        for stream in listener.incoming().take(requests) {
+            let mut stream = stream.unwrap();
+            let mut reader = BufReader::new(&stream);
+            let mut line = String::new();
+            while reader.read_line(&mut line).unwrap() > 0 && line != "\r\n" {
+                line.clear();
+            }
+            let head = format!(
+                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
+                body.len()
+            );
+            stream.write_all(head.as_bytes()).unwrap();
+            stream.write_all(body).unwrap();
+        }
+    });
+    (url, thread)
+}
+
+fn document_manifest(work: &Path, url: &str, sha256: &str) {
+    fs::create_dir_all(work.join("corpora")).unwrap();
+    fs::write(
+        work.join("corpora/specs.toml"),
+        format!(
+            "name = \"specs\"\n\n[[source]]\nkind = \"document\"\nname = \"rfc9449\"\n\
+             url = {url:?}\nsha256 = {sha256:?}\nuse = \"quote\"\ncurrent = true\n"
+        ),
+    )
+    .unwrap();
+}
+
+#[test]
+fn documents_are_pinned_by_hash_and_cached() {
+    const SPEC: &[u8] = b"<h1>RFC 9449</h1>\n";
+    const SPEC_SHA256: &str = "adcd06b613eb4e660e403fbec3d11011983dec4501489279e191b728e2fe6b2b";
+    let old_pin = "0".repeat(64);
+    let tmp = tempfile::tempdir().unwrap();
+    let work = tmp.path().join("work");
+    // Two downloads: --print-hashes, then the run that finds the mismatch.
+    let (url, server) = serve(SPEC, 2);
+    document_manifest(&work, &url, &old_pin);
+
+    let hashes = stdout(&codecite(
+        &work,
+        tmp.path(),
+        &["index", "specs", "--print-hashes"],
+    ));
+    assert_eq!(hashes, format!("{SPEC_SHA256}  rfc9449  {url}\n"));
+    assert!(!tmp.path().join("data/codecite/specs.db").exists());
+
+    let mismatch = codecite(&work, tmp.path(), &["index", "specs"]);
+    assert!(!mismatch.status.success());
+    let stderr = String::from_utf8_lossy(&mismatch.stderr);
+    assert!(stderr.contains("document `rfc9449`"), "{stderr}");
+    assert!(stderr.contains(&old_pin), "{stderr}");
+    assert!(stderr.contains(SPEC_SHA256), "{stderr}");
+    server.join().unwrap();
+
+    // Re-pinned: the bytes --print-hashes fetched are in the cache, so the
+    // run works with the server gone.
+    document_manifest(&work, &url, SPEC_SHA256);
+    let indexed = stdout(&codecite(&work, tmp.path(), &["index", "specs"]));
+    assert!(
+        indexed.contains("fetched: 0 downloaded, 1 from the cache"),
+        "{indexed}"
+    );
+    let pinned = stdout(&codecite(&work, tmp.path(), &["index", "specs"]));
+    assert!(pinned.contains("1 from the cache"), "{pinned}");
+    assert!(
+        tmp.path()
+            .join("cache/codecite/documents")
+            .join(SPEC_SHA256)
+            .is_file()
+    );
+}
```

Run: `cargo test -p codecite --test index`
Expected: FAIL (the binary does not build against the new `index_corpus`; once it does, `--print-hashes` is an unknown argument).

- [ ] **Step 2: Implement**

```diff
diff --git a/crates/codecite/src/main.rs b/crates/codecite/src/main.rs
index 59be3ba..126cf35 100644
--- a/crates/codecite/src/main.rs
+++ b/crates/codecite/src/main.rs
@@ -6,7 +6,7 @@ use std::path::{Path, PathBuf};
 use anyhow::{Context as _, bail};
 use clap::{Parser, Subcommand};
 use codecite_core::Store;
-use codecite_index::{Manifest, WalkOptions, index_corpus};
+use codecite_index::{Fetcher, LicenseClass, Manifest, Source, WalkOptions, index_corpus};
 
 /// Command-line arguments.
 #[derive(Parser)]
@@ -26,18 +26,27 @@ enum Command {
         /// The manifest to read [default: corpora/<CORPUS>.toml].
         #[arg(long)]
         manifest: Option<PathBuf>,
+        /// Download each document source and print its sha256 without
+        /// checking it against the manifest or indexing anything.
+        #[arg(long)]
+        print_hashes: bool,
     },
 }
 
 fn main() -> anyhow::Result<()> {
     match Cli::parse().command {
-        Command::Index { corpus, manifest } => index(&corpus, manifest),
+        Command::Index {
+            corpus,
+            manifest,
+            print_hashes,
+        } => index(&corpus, manifest, print_hashes),
     }
 }
 
 /// `codecite index`: reads the manifest, indexes it into the corpus file in
-/// the user's data directory, and prints a summary.
-fn index(corpus: &str, manifest: Option<PathBuf>) -> anyhow::Result<()> {
+/// the user's data directory, and prints a summary. With `print_hashes`, it
+/// prints each document's hash instead.
+fn index(corpus: &str, manifest: Option<PathBuf>, print_hashes: bool) -> anyhow::Result<()> {
     check_corpus_name(corpus)?;
     let manifest_path =
         manifest.unwrap_or_else(|| Path::new("corpora").join(format!("{corpus}.toml")));
@@ -51,13 +60,19 @@ fn index(corpus: &str, manifest: Option<PathBuf>) -> anyhow::Result<()> {
         );
     }
 
-    let path = corpus_path(corpus)?;
+    let dirs = project_dirs()?;
+    let fetcher = Fetcher::new(dirs.cache_dir().join("documents"));
+    if print_hashes {
+        return print_document_hashes(&manifest, &fetcher);
+    }
+
+    let path = dirs.data_dir().join(format!("{corpus}.db"));
     if let Some(dir) = path.parent() {
         fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
     }
     let mut store = Store::open_read_write(&path)
         .with_context(|| format!("opening corpus file {}", path.display()))?;
-    let summary = index_corpus(&mut store, &manifest, &WalkOptions::default())
+    let summary = index_corpus(&mut store, &manifest, &WalkOptions::default(), &fetcher)
         .with_context(|| format!("indexing corpus `{corpus}`"))?;
     println!("indexed corpus `{corpus}` into {}", path.display());
     println!("{summary}");
@@ -76,12 +91,31 @@ fn check_corpus_name(corpus: &str) -> anyhow::Result<()> {
     Ok(())
 }
 
-/// Where the corpus file lives: `<data dir>/codecite/<corpus>.db`, e.g.
-/// `~/.local/share/codecite/identity.db` on Linux. Never in the repository.
-fn corpus_path(corpus: &str) -> anyhow::Result<PathBuf> {
-    let dirs = directories::ProjectDirs::from("", "", "codecite")
-        .context("no home directory to keep corpus files in")?;
-    Ok(dirs.data_dir().join(format!("{corpus}.db")))
+/// `codecite index --print-hashes`: downloads each document source that is
+/// not `excluded` and prints `<sha256>  <name>  <url>`, plus `(pinned)` when
+/// the hash is the one the manifest pins, so the owner can pin a new or
+/// changed document on purpose.
+fn print_document_hashes(manifest: &Manifest, fetcher: &Fetcher) -> anyhow::Result<()> {
+    for source in &manifest.sources {
+        let Source::Document(doc) = source else {
+            continue;
+        };
+        if doc.license_class == LicenseClass::Excluded {
+            continue;
+        }
+        let hash = fetcher.fetch_hash(doc)?;
+        let pinned = if hash == doc.sha256 { "  (pinned)" } else { "" };
+        println!("{hash}  {}  {}{pinned}", doc.name, doc.url);
+    }
+    Ok(())
+}
+
+/// Where codecite keeps its files, never in a repository: corpus files in
+/// the data directory (`~/.local/share/codecite/<corpus>.db` on Linux) and
+/// fetched documents in the cache directory (`~/.cache/codecite/documents`).
+fn project_dirs() -> anyhow::Result<directories::ProjectDirs> {
+    directories::ProjectDirs::from("", "", "codecite")
+        .context("no home directory to keep corpus files in")
 }
 
 #[cfg(test)]
@@ -118,8 +152,16 @@ mod tests {
         assert!(Cli::try_parse_from(["codecite", "index"]).is_err());
         let cli =
             Cli::try_parse_from(["codecite", "index", "identity", "--manifest", "m.toml"]).unwrap();
-        let Command::Index { corpus, manifest } = cli.command;
+        let Command::Index {
+            corpus,
+            manifest,
+            print_hashes,
+        } = cli.command;
         assert_eq!(corpus, "identity");
         assert_eq!(manifest, Some(PathBuf::from("m.toml")));
+        assert!(!print_hashes);
+        let cli = Cli::try_parse_from(["codecite", "index", "identity", "--print-hashes"]).unwrap();
+        let Command::Index { print_hashes, .. } = cli.command;
+        assert!(print_hashes);
     }
 }
```

> **Learn: `let ... else`.** `let Source::Document(doc) = source else { continue; };` binds `doc` when the pattern matches and runs the `else` block (which must leave the loop or the function) when it doesn't, so the happy path stays unindented.

Run: `cargo test -p codecite`
Expected: PASS (3 unit, 3 end-to-end tests).

- [ ] **Step 3: Docs**

```diff
diff --git a/docs/design.md b/docs/design.md
index d13b90b..0307df2 100644
--- a/docs/design.md
+++ b/docs/design.md
@@ -178,7 +178,7 @@ BM25 over code-aware terms does most of the work; vectors and graph signals refi
 
 **Indexing pipeline**
 
-1. **Fetch:** `git clone --filter=blob:none` at the manifest's pinned commit, read a local working tree, or fetch a document by URL and check it against its pinned `sha256`. Fetched documents are cached outside the repo; changed bytes stop the run until the pin is updated on purpose.
+1. **Fetch:** `git clone --filter=blob:none` at the manifest's pinned commit, read a local working tree, or fetch a document by URL and check it against its pinned `sha256`. Every document is fetched before anything is written, so changed bytes stop the run with an error naming the document and both hashes, and the store is untouched until the pin is updated on purpose. Fetched documents are cached outside the repo, in `<cache dir>/codecite/documents/<sha256>` (`~/.cache` on Linux): a document whose pinned hash is in the cache is never fetched again, so indexing works offline after the first run. A cache file whose bytes no longer match its name is deleted and fetched again. A download is plain HTTP(S) GET (`ureq`, rustls with Mozilla's roots, the standard proxy variables), capped at 16 MiB and 60 seconds; the hash, not the transport or an ETag, is what pins the bytes. A document marked `excluded` is never fetched.
 2. **Walk:** list the files git sees (`git ls-files`, so `.gitignore`, `.git/info/exclude` and the global excludes apply and tracked files are never ignored), apply the manifest's `include`/`exclude` globs, and skip deny-listed secrets, symlinks, files over 1 MiB and binaries (a NUL byte in the first 8000 bytes, git's rule; extensions are never trusted). Every skipped path is reported with its reason; an ignored directory counts once. A path under a symlinked directory is skipped as a symlink. The repository's config is not trusted, since it can name programs for git to run: the walk turns `core.fsmonitor` off, allows git no transport (an empty `GIT_ALLOW_PROTOCOL`, so a partial clone never fetches a missing object from its promisor remote), and never runs `git status`; `dirty` compares HEAD's tree, the index and `git hash-object --no-filters` of the kept files only.
 3. **Chunk:** the language comes from the file extension, then a `python`, `node` or `ts-node` shebang. tree-sitter splits Rust, Python, TypeScript, JavaScript, Go and C# at definitions, one query per language: functions and methods become `function` chunks; classes, structs, enums, traits, interfaces and Rust `impl` blocks become `class` chunks, nested ones included, and each definition but an `impl` block yields a symbol. A definition nested more than 32 deep stays inside its parent's chunk. A symbol's descriptor repeats its parents', so one longer than 512 bytes gives no symbol, to its definition or anything inside it, and a file holds at most 8 MiB of descriptors; past that, definitions keep their chunks without symbols. Together these bound a file's chunking memory by its size. Markdown splits into ATX heading sections, ignoring headings in fenced code. Every other line, and every file in another language, falls into windows of at most 60 lines that never start or end on a line without a letter or digit. Spec documents split into numbered sections and the numbered steps inside them, each with its anchor and its uppercase requirement keywords (BCP 14: RFC 2119 and RFC 8174).
 4. **Terms:** split camelCase, snake_case and kebab-case identifiers into lowercase parts and keep the original, so `validateToken` matches "validate token" and the exact name. Any character that is not a letter or digit separates parts, an acronym ends before the capital that starts the next word (`HTTPServer` → `http server`), and digits stay with the part before them (`sha256`, `rfc9449`).
@@ -253,7 +253,7 @@ The CLI and the MCP server expose the same operations and return the same JSON,
 | CLI command | Purpose |
 | --- | --- |
 | `codecite add <corpus> <url>...` | Add sources to a manifest, detect licenses, pin the current commit |
-| `codecite index <corpus> [--manifest <path>]` | Fetch, index and run providers; incremental by default. Reads `corpora/<corpus>.toml` unless `--manifest` names another file |
+| `codecite index <corpus> [--manifest <path>] [--print-hashes]` | Fetch, index and run providers; incremental by default. Reads `corpora/<corpus>.toml` unless `--manifest` names another file. `--print-hashes` downloads each document source and prints its sha256, without checking or indexing, so a new or changed document is pinned on purpose |
 | `codecite search <corpus> "<query>"` | Ranked chunks with citations |
 | `codecite def <symbol>` | Where a symbol is defined |
 | `codecite refs <symbol>` | Every reference to a symbol |
```

```diff
diff --git a/README.md b/README.md
index cde35fd..b006fc5 100644
--- a/README.md
+++ b/README.md
@@ -12,7 +12,10 @@ from memory.
 
 Pre-alpha. `codecite index <corpus>` indexes the local git checkouts a corpus
 manifest (`corpora/<corpus>.toml`) lists into `~/.local/share/codecite/<corpus>.db`
-on Linux; searching it comes next.
+on Linux, and downloads the documents it lists, checking each against its
+pinned sha256 and caching it in `~/.cache/codecite/documents`, so later runs
+work offline. `codecite index <corpus> --print-hashes` prints each document's
+current hash, for pinning. Indexing documents and searching come next.
 
 ## Planned
```

- [ ] **Step 4: Run it for real**

```bash
cargo build --release
d=$(mktemp -d); mkdir -p $d/corpora
printf 'name = "specs"\n\n[[source]]\nkind = "document"\nname = "rfc9449"\nurl = "https://www.rfc-editor.org/rfc/rfc9449.html"\nsha256 = "%064d"\nuse = "quote"\ncurrent = true\n' 0 > $d/corpora/specs.toml
cd $d && export XDG_CACHE_HOME=$d/cache XDG_DATA_HOME=$d/data
<worktree>/target/release/codecite index specs --print-hashes   # prints the hash
<worktree>/target/release/codecite index specs                  # fails: both hashes
# paste the printed hash into sha256, then:
<worktree>/target/release/codecite index specs                  # fetched: 0 downloaded, 1 from the cache
```

Also try a host that does not resolve: the error names the document and the URL.

- [ ] **Step 5: Gates and commit**

Run: `cargo fmt --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked && cargo deny check`
Expected: all green.

```bash
git add crates/codecite/src/main.rs crates/codecite/tests/index.rs docs/design.md README.md
git commit -m "feat: print document hashes with codecite index --print-hashes (#22)"
```
