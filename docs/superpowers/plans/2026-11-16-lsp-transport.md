# LSP transport and server lifecycle (#224, #225)

Goal: give `codecite-lsp` a JSON-RPC 2.0 client over stdio (#224) and a
language-server process it starts, initializes and shuts down (#225), tested
against a scripted fake server so CI needs no real one.

Two of #233's questions shape this and are taken at its recommendations, open
to veto on the PR: a blocking client with one reader thread and channels (no
`tokio`), and hand-written message types with `serde_json` (no `lsp-types`).
No new dependency: `serde`, `serde_json` and `proptest` are already in the
workspace.

## Steps

1. **Framing, test-first** (`src/framing.rs`). Tests: a round trip; a
   `Content-Type` header accepted and ignored; header names in any case; a
   missing length, a non-numeric length, a header block over its cap (8 KiB),
   a body over its cap (16 MiB by default) and a body that isn't JSON each give
   their own `Error` variant; EOF before a header is a clean end, EOF inside a
   frame is an error. A `proptest` writes any sequence of JSON messages and
   reads them back through a reader that returns at most a few bytes per read.
2. **Messages** (`src/message.rs`). Classify a decoded value as a request,
   notification or response; ids are numbers or strings; error responses carry
   `code`, `message` and `data`.
3. **Client, test-first** (`src/client.rs`), over `std::io::pipe` pairs. A
   reader thread owns the server's output and routes each message: a response
   to the waiting request's channel by id, an unknown id to stderr, a
   notification to the caller's callback, a server-to-client request to a
   built-in reply (`workspace/configuration` gets one `null` per item,
   `window/workDoneProgress/create` and `client/(un)registerCapability` get
   `null`, anything else `MethodNotFound`). `request` takes a timeout; on
   expiry it sends `$/cancelRequest`, forgets the id so the late reply is
   dropped, and returns `Timeout` naming the method and elapsed time.
   `RequestCancelled` and `ContentModified` are their own variants. EOF, a bad
   frame or a failed write fails every in-flight request at once and every
   later one immediately. Tests: round trip, out-of-order responses, an
   interleaved notification, each server-to-client request, an error
   response, the two retryable codes, a timeout with cancel and a late reply,
   EOF with several requests in flight, a write failure, a malformed frame.
4. **Server, test-first** (`src/server.rs`, `tests/lifecycle.rs`). `Server::
   start` spawns the command in the workspace root with piped stdio, drains
   stderr into a bounded ring buffer, sends `initialize` (root URI, one
   workspace folder, capabilities for definition, references, call hierarchy
   and work-done progress only) and `initialized`, then waits for readiness:
   every `$/progress` token it began has ended, or a probe request answers
   without `ContentModified`, or nothing. All of it shares one startup
   deadline; past it, `ServerNotReady` with the elapsed time and the stderr
   tail. A missing program is `ServerNotFound`. A dead server's requests fail
   with `ServerExited` carrying its status and stderr tail. `shutdown` and
   `Drop` both send `shutdown` and `exit`, close stdin, wait a grace period
   and kill.
5. **Fake server.** `tests/lifecycle.rs` is a `harness = false` test whose
   `main` runs the fake server when called as `--fake-lsp <scenario>`, and
   otherwise runs the lifecycle tests in parallel under one watchdog,
   spawning its own executable as the server. Scenarios: clean, slow reply,
   never initializes, slow `initialize`, crash during startup, crash after
   it, progress that never ends, `ContentModified` until ready, ignores
   `shutdown`, a malformed frame, stops reading its input. The fake exits by
   itself after 60 s, so even a failed test leaves nothing running. Tests:
   clean start and shutdown; startup timeout; crash during and after
   startup; missing command; ignored `shutdown` killed after the grace
   period; the pid gone after dropping the handle, cleanly and while a panic
   unwinds past a stuck server; a server that stops reading still killed.
6. **Docs.** `docs/design.md`'s provider model describes the transport and
   the lifecycle with its timeouts. The decisions table is #233's.
7. **Checks.** `cargo fmt --check`, `cargo clippy --workspace --all-targets
   --locked -- -D warnings`, `cargo test --workspace --locked`, `cargo deny
   check`.
