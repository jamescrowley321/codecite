//! A blocking JSON-RPC 2.0 client over any reader and writer pair.
//!
//! One reader thread owns the server's output and routes every message:
//! a response to the channel its request waits on, a notification to the
//! caller's handler, a server-to-client request to a built-in reply. Callers
//! block on their own request with a deadline. Nothing is written to stdout,
//! which belongs to the MCP server; diagnostics go to stderr.

use std::collections::HashMap;
use std::io::{BufReader, Read, Write};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, SyncSender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::framing::{DEFAULT_MAX_BODY_BYTES, read_message, write_message};
use crate::message::{
    CONTENT_MODIFIED, Id, METHOD_NOT_FOUND, Message, REQUEST_CANCELLED, ResponseError,
};
use crate::{Error, Result};

/// Called on the reader thread with each server notification's method and
/// params. It must not block for long: the next message waits on it.
pub type NotificationHandler = Box<dyn FnMut(&str, Value) + Send>;

/// How a [`Client`] reads.
pub struct ClientOptions {
    /// The cap on one message's body, in bytes.
    pub max_body: usize,
    /// Receives server notifications; `None` drops them.
    pub on_notification: Option<NotificationHandler>,
}

impl Default for ClientOptions {
    fn default() -> Self {
        Self {
            max_body: DEFAULT_MAX_BODY_BYTES,
            on_notification: None,
        }
    }
}

impl std::fmt::Debug for ClientOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientOptions")
            .field("max_body", &self.max_body)
            .field("on_notification", &self.on_notification.is_some())
            .finish()
    }
}

enum Failure {
    Rpc(ResponseError),
    Closed(String),
}

type Reply = std::result::Result<Value, Failure>;

#[derive(Default)]
struct State {
    pending: HashMap<i64, SyncSender<Reply>>,
    closed: Option<String>,
}

struct Shared {
    writer: Mutex<Option<Box<dyn Write + Send>>>,
    state: Mutex<State>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Shared {
    fn closed_error(&self) -> Error {
        let reason = lock(&self.state)
            .closed
            .clone()
            .unwrap_or_else(|| "closed".to_owned());
        Error::Closed { reason }
    }

    fn send(&self, message: &Message) -> Result<()> {
        let result = {
            let mut writer = lock(&self.writer);
            writer
                .as_mut()
                .map(|w| write_message(w, &message.to_value()))
        };
        match result {
            None => Err(self.closed_error()),
            Some(Ok(())) => Ok(()),
            Some(Err(e)) => {
                self.close(&format!("writing to the server failed: {e}"));
                Err(self.closed_error())
            }
        }
    }

    /// Marks the connection closed, fails every in-flight request at once
    /// and drops the writer. The first reason sticks.
    fn close(&self, reason: &str) {
        let pending = {
            let mut state = lock(&self.state);
            state.closed.get_or_insert_with(|| reason.to_owned());
            std::mem::take(&mut state.pending)
        };
        for (_, tx) in pending {
            let _ = tx.try_send(Err(Failure::Closed(reason.to_owned())));
        }
        lock(&self.writer).take();
    }
}

/// Closes the connection when the reader thread ends, a panic included.
struct CloseOnExit(Arc<Shared>);

impl Drop for CloseOnExit {
    fn drop(&mut self) {
        self.0.close("the reader thread stopped");
    }
}

/// A JSON-RPC client. Requests may be made from several threads at once.
pub struct Client {
    shared: Arc<Shared>,
    next_id: AtomicI64,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("closed", &lock(&self.shared.state).closed)
            .finish_non_exhaustive()
    }
}

impl Client {
    /// Starts a client reading `reader` (the server's output) and writing
    /// `writer` (its input). The reader thread runs until `reader` ends.
    pub fn new<R, W>(reader: R, writer: W, options: ClientOptions) -> Result<Self>
    where
        R: Read + Send + 'static,
        W: Write + Send + 'static,
    {
        let shared = Arc::new(Shared {
            writer: Mutex::new(Some(Box::new(writer))),
            state: Mutex::new(State::default()),
        });
        let thread_shared = Arc::clone(&shared);
        thread::Builder::new()
            .name("codecite-lsp-reader".into())
            .spawn(move || read_loop(thread_shared, reader, options))?;
        Ok(Self {
            shared,
            next_id: AtomicI64::new(1),
        })
    }

    /// Sends a request and waits up to `timeout` for its response. On
    /// expiry it sends `$/cancelRequest` and the late response is dropped.
    pub fn request(&self, method: &str, params: Value, timeout: Duration) -> Result<Value> {
        let start = Instant::now();
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::sync_channel(1);
        {
            let mut state = lock(&self.shared.state);
            if let Some(reason) = &state.closed {
                return Err(Error::Closed {
                    reason: reason.clone(),
                });
            }
            state.pending.insert(id, tx);
        }
        let message = Message::Request {
            id: Id::Number(id),
            method: method.to_owned(),
            params,
        };
        if let Err(e) = self.shared.send(&message) {
            lock(&self.shared.state).pending.remove(&id);
            return Err(e);
        }
        let reply = match rx.recv_timeout(timeout) {
            Ok(reply) => reply,
            Err(RecvTimeoutError::Disconnected) => return Err(self.shared.closed_error()),
            Err(RecvTimeoutError::Timeout) => {
                lock(&self.shared.state).pending.remove(&id);
                // The reply may have landed between the timeout and the removal.
                match rx.try_recv() {
                    Ok(reply) => reply,
                    Err(_) => {
                        let _ = self.notify("$/cancelRequest", json!({ "id": id }));
                        return Err(Error::Timeout {
                            method: method.to_owned(),
                            elapsed: start.elapsed(),
                        });
                    }
                }
            }
        };
        reply.map_err(|failure| match failure {
            Failure::Closed(reason) => Error::Closed { reason },
            Failure::Rpc(e) if e.code == REQUEST_CANCELLED => Error::RequestCancelled {
                method: method.to_owned(),
            },
            Failure::Rpc(e) if e.code == CONTENT_MODIFIED => Error::ContentModified {
                method: method.to_owned(),
            },
            Failure::Rpc(e) => Error::Rpc {
                method: method.to_owned(),
                code: e.code,
                message: e.message,
                data: e.data,
            },
        })
    }

    /// Sends a notification.
    pub fn notify(&self, method: &str, params: Value) -> Result<()> {
        self.shared.send(&Message::Notification {
            method: method.to_owned(),
            params,
        })
    }

    /// Closes the writer, which the server sees as the end of its input.
    /// In-flight and later requests fail with [`Error::Closed`].
    pub fn close(&self) {
        self.shared.close("the client closed the connection");
    }

    /// Why the connection closed, or `None` while it is open.
    pub fn closed_reason(&self) -> Option<String> {
        lock(&self.shared.state).closed.clone()
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.close();
    }
}

fn read_loop<R: Read>(shared: Arc<Shared>, reader: R, mut options: ClientOptions) {
    let _close = CloseOnExit(Arc::clone(&shared));
    let mut reader = BufReader::new(reader);
    loop {
        let value = match read_message(&mut reader, options.max_body) {
            Ok(Some(value)) => value,
            Ok(None) => {
                shared.close("the server closed its output");
                return;
            }
            Err(e) => {
                eprintln!("codecite-lsp: closing the connection: {e}");
                shared.close(&e.to_string());
                return;
            }
        };
        match Message::from_value(value) {
            // Framing is intact, so one bad message need not end the session.
            Err(e) => eprintln!("codecite-lsp: dropped a message: {e}"),
            Ok(Message::Response { id, outcome }) => {
                let waiting = match &id {
                    Some(Id::Number(n)) => lock(&shared.state).pending.remove(n),
                    _ => None,
                };
                match waiting {
                    Some(tx) => {
                        let _ = tx.try_send(outcome.map_err(Failure::Rpc));
                    }
                    None => eprintln!("codecite-lsp: dropped a response to unknown request {id:?}"),
                }
            }
            Ok(Message::Notification { method, params }) => {
                if let Some(handler) = options.on_notification.as_mut() {
                    handler(&method, params);
                }
            }
            Ok(Message::Request { id, method, params }) => {
                let reply = Message::Response {
                    id: Some(id),
                    outcome: default_reply(&method, &params),
                };
                if shared.send(&reply).is_err() {
                    return;
                }
            }
        }
    }
}

/// Answers a server-to-client request. Some servers stall until these are
/// answered, so each gets an empty result or `MethodNotFound`.
fn default_reply(method: &str, params: &Value) -> std::result::Result<Value, ResponseError> {
    match method {
        "workspace/configuration" => {
            let items = params["items"].as_array().map_or(0, Vec::len);
            Ok(Value::Array(vec![Value::Null; items]))
        }
        "window/workDoneProgress/create"
        | "client/registerCapability"
        | "client/unregisterCapability" => Ok(Value::Null),
        _ => Err(ResponseError {
            code: METHOD_NOT_FOUND,
            message: format!("codecite does not handle {method}"),
            data: None,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{PipeReader, PipeWriter, pipe};

    const LONG: Duration = Duration::from_secs(10);

    /// The server's side of a client connected over two OS pipes.
    struct Peer {
        reader: Option<BufReader<PipeReader>>,
        writer: Option<PipeWriter>,
    }

    impl Peer {
        fn recv(&mut self) -> Message {
            let reader = self.reader.as_mut().unwrap();
            let value = read_message(reader, DEFAULT_MAX_BODY_BYTES)
                .unwrap()
                .unwrap();
            Message::from_value(value).unwrap()
        }

        fn send(&mut self, message: Message) {
            write_message(self.writer.as_mut().unwrap(), &message.to_value()).unwrap();
        }

        fn send_raw(&mut self, bytes: &[u8]) {
            self.writer.as_mut().unwrap().write_all(bytes).unwrap();
        }

        fn respond(&mut self, id: i64, outcome: std::result::Result<Value, ResponseError>) {
            self.send(Message::Response {
                id: Some(Id::Number(id)),
                outcome,
            });
        }

        fn recv_request(&mut self) -> (i64, String, Value) {
            match self.recv() {
                Message::Request {
                    id: Id::Number(id),
                    method,
                    params,
                } => (id, method, params),
                other => panic!("expected a request, got {other:?}"),
            }
        }
    }

    fn connect_with(options: ClientOptions) -> (Client, Peer) {
        let (client_reads, peer_writes) = pipe().unwrap();
        let (peer_reads, client_writes) = pipe().unwrap();
        let client = Client::new(client_reads, client_writes, options).unwrap();
        let peer = Peer {
            reader: Some(BufReader::new(peer_reads)),
            writer: Some(peer_writes),
        };
        (client, peer)
    }

    fn connect() -> (Client, Peer) {
        connect_with(ClientOptions::default())
    }

    fn rpc_error(code: i64) -> ResponseError {
        ResponseError {
            code,
            message: "failed".into(),
            data: Some(json!({"why": "test"})),
        }
    }

    #[test]
    fn round_trip() {
        let (client, mut peer) = connect();
        thread::scope(|s| {
            let call = s.spawn(|| client.request("test/echo", json!({"a": 1}), LONG));
            let (id, method, params) = peer.recv_request();
            assert_eq!(method, "test/echo");
            assert_eq!(params, json!({"a": 1}));
            peer.respond(id, Ok(json!({"echo": 1})));
            assert_eq!(call.join().unwrap().unwrap(), json!({"echo": 1}));
        });
    }

    #[test]
    fn responses_out_of_order_reach_their_requests() {
        let (client, mut peer) = connect();
        thread::scope(|s| {
            let first = s.spawn(|| client.request("first", Value::Null, LONG));
            let (id1, _, _) = peer.recv_request();
            let second = s.spawn(|| client.request("second", Value::Null, LONG));
            let (id2, method2, _) = peer.recv_request();
            assert_eq!(method2, "second");
            assert_ne!(id1, id2);
            peer.respond(id2, Ok(json!("two")));
            assert_eq!(second.join().unwrap().unwrap(), json!("two"));
            peer.respond(id1, Ok(json!("one")));
            assert_eq!(first.join().unwrap().unwrap(), json!("one"));
        });
    }

    #[test]
    fn notifications_flow_both_ways_between_responses() {
        let (tx, rx) = mpsc::channel();
        let options = ClientOptions {
            on_notification: Some(Box::new(move |method, params| {
                tx.send((method.to_owned(), params)).unwrap();
            })),
            ..ClientOptions::default()
        };
        let (client, mut peer) = connect_with(options);
        client.notify("initialized", json!({})).unwrap();
        assert_eq!(
            peer.recv(),
            Message::Notification {
                method: "initialized".into(),
                params: json!({})
            }
        );
        thread::scope(|s| {
            let call = s.spawn(|| client.request("q", Value::Null, LONG));
            let (id, _, _) = peer.recv_request();
            peer.send(Message::Notification {
                method: "window/logMessage".into(),
                params: json!({"type": 3, "message": "indexing"}),
            });
            peer.respond(id, Ok(json!(42)));
            assert_eq!(call.join().unwrap().unwrap(), json!(42));
        });
        let (method, params) = rx.recv_timeout(LONG).unwrap();
        assert_eq!(method, "window/logMessage");
        assert_eq!(params["message"], "indexing");
    }

    #[test]
    fn notifications_without_a_handler_are_dropped() {
        let (client, mut peer) = connect();
        // Far more than a pipe buffer holds: nothing may queue them up.
        let big = "x".repeat(64 * 1024);
        thread::scope(|s| {
            let call = s.spawn(|| client.request("q", Value::Null, LONG));
            let (id, _, _) = peer.recv_request();
            for _ in 0..32 {
                peer.send(Message::Notification {
                    method: "$/progress".into(),
                    params: json!({"token": 1, "value": {"kind": "report", "message": big}}),
                });
            }
            peer.respond(id, Ok(json!(true)));
            assert_eq!(call.join().unwrap().unwrap(), json!(true));
        });
    }

    #[test]
    fn server_to_client_requests_get_a_reply() {
        let (_client, mut peer) = connect();
        let cases = [
            (
                "workspace/configuration",
                json!({"items": [{"section": "a"}, {"section": "b"}]}),
                Ok(json!([null, null])),
            ),
            (
                "window/workDoneProgress/create",
                json!({"token": "t"}),
                Ok(Value::Null),
            ),
            (
                "client/registerCapability",
                json!({"registrations": []}),
                Ok(Value::Null),
            ),
        ];
        for (n, (method, params, expected)) in cases.into_iter().enumerate() {
            let id = Id::String(format!("s{n}"));
            peer.send(Message::Request {
                id: id.clone(),
                method: method.into(),
                params,
            });
            assert_eq!(
                peer.recv(),
                Message::Response {
                    id: Some(id),
                    outcome: expected
                }
            );
        }
        peer.send(Message::Request {
            id: Id::Number(99),
            method: "workspace/applyEdit".into(),
            params: json!({}),
        });
        match peer.recv() {
            Message::Response {
                id: Some(Id::Number(99)),
                outcome: Err(e),
            } => assert_eq!(e.code, METHOD_NOT_FOUND),
            other => panic!("expected MethodNotFound, got {other:?}"),
        }
    }

    #[test]
    fn error_responses_are_typed() {
        let (client, mut peer) = connect();
        thread::scope(|s| {
            let call = s.spawn(|| client.request("textDocument/definition", Value::Null, LONG));
            let (id, _, _) = peer.recv_request();
            peer.respond(id, Err(rpc_error(-32602)));
            match call.join().unwrap() {
                Err(Error::Rpc {
                    method,
                    code,
                    message,
                    data,
                }) => {
                    assert_eq!(method, "textDocument/definition");
                    assert_eq!(code, -32602);
                    assert_eq!(message, "failed");
                    assert_eq!(data, Some(json!({"why": "test"})));
                }
                other => panic!("expected an Rpc error, got {other:?}"),
            }

            let call = s.spawn(|| client.request("a", Value::Null, LONG));
            let (id, _, _) = peer.recv_request();
            peer.respond(id, Err(rpc_error(REQUEST_CANCELLED)));
            assert!(matches!(
                call.join().unwrap(),
                Err(Error::RequestCancelled { method }) if method == "a"
            ));

            let call = s.spawn(|| client.request("b", Value::Null, LONG));
            let (id, _, _) = peer.recv_request();
            peer.respond(id, Err(rpc_error(CONTENT_MODIFIED)));
            assert!(matches!(
                call.join().unwrap(),
                Err(Error::ContentModified { method }) if method == "b"
            ));
        });
    }

    #[test]
    fn timeout_sends_cancel_and_drops_the_late_reply() {
        let (client, mut peer) = connect();
        let timeout = Duration::from_millis(50);
        let err = thread::scope(|s| {
            let call = s.spawn(|| client.request("slow", Value::Null, timeout));
            let (id, _, _) = peer.recv_request();
            let err = call.join().unwrap().unwrap_err();
            assert_eq!(
                peer.recv(),
                Message::Notification {
                    method: "$/cancelRequest".into(),
                    params: json!({"id": id})
                }
            );
            peer.respond(id, Ok(json!("late")));
            err
        });
        match err {
            Error::Timeout { method, elapsed } => {
                assert_eq!(method, "slow");
                assert!(elapsed >= timeout, "{elapsed:?}");
            }
            other => panic!("expected a timeout, got {other:?}"),
        }
        // The late reply went nowhere; the next request gets its own.
        thread::scope(|s| {
            let call = s.spawn(|| client.request("next", Value::Null, LONG));
            let (id, _, _) = peer.recv_request();
            peer.respond(id, Ok(json!("mine")));
            assert_eq!(call.join().unwrap().unwrap(), json!("mine"));
        });
    }

    #[test]
    fn unknown_response_ids_are_dropped() {
        let (client, mut peer) = connect();
        peer.respond(12345, Ok(json!("stray")));
        peer.send(Message::Response {
            id: Some(Id::String("nobody".into())),
            outcome: Ok(Value::Null),
        });
        thread::scope(|s| {
            let call = s.spawn(|| client.request("q", Value::Null, LONG));
            let (id, _, _) = peer.recv_request();
            peer.respond(id, Ok(json!(1)));
            assert_eq!(call.join().unwrap().unwrap(), json!(1));
        });
    }

    #[test]
    fn eof_fails_every_in_flight_request_at_once() {
        let (client, mut peer) = connect();
        let start = Instant::now();
        thread::scope(|s| {
            let calls: Vec<_> = (0..3)
                .map(|_| s.spawn(|| client.request("q", Value::Null, LONG)))
                .collect();
            for _ in 0..3 {
                peer.recv_request();
            }
            peer.writer.take();
            for call in calls {
                assert!(matches!(call.join().unwrap(), Err(Error::Closed { .. })));
            }
        });
        assert!(start.elapsed() < LONG / 2, "{:?}", start.elapsed());
        assert!(matches!(
            client.request("later", Value::Null, LONG),
            Err(Error::Closed { reason }) if reason.contains("closed its output")
        ));
    }

    #[test]
    fn a_write_failure_fails_every_in_flight_request() {
        let (client, mut peer) = connect();
        thread::scope(|s| {
            let first = s.spawn(|| client.request("first", Value::Null, LONG));
            peer.recv_request();
            peer.reader.take();
            let second = client.request("second", Value::Null, LONG);
            assert!(matches!(second, Err(Error::Closed { reason }) if reason.contains("writing")));
            assert!(matches!(first.join().unwrap(), Err(Error::Closed { .. })));
        });
        assert!(client.closed_reason().is_some());
    }

    #[test]
    fn a_malformed_frame_closes_the_connection() {
        let huge_header = format!("X-Pad: {}\r\n\r\n", "a".repeat(10_000));
        let cases: [(&[u8], &str); 5] = [
            (b"Content-Type: x\r\n\r\n{}", "no Content-Length"),
            (b"Content-Length: lots\r\n\r\n{}", "invalid Content-Length"),
            (huge_header.as_bytes(), "header is over"),
            (b"Content-Length: 4096\r\n\r\n", "over the 1024-byte cap"),
            (b"Content-Length: 3\r\n\r\nnot", "not a JSON-RPC message"),
        ];
        for (bytes, expected) in cases {
            let (client, mut peer) = connect_with(ClientOptions {
                max_body: 1024,
                ..ClientOptions::default()
            });
            thread::scope(|s| {
                let call = s.spawn(|| client.request("q", Value::Null, LONG));
                peer.recv_request();
                peer.send_raw(bytes);
                match call.join().unwrap() {
                    Err(Error::Closed { reason }) => {
                        assert!(reason.contains(expected), "{reason:?} lacks {expected:?}");
                    }
                    other => panic!("expected Closed for {expected:?}, got {other:?}"),
                }
            });
        }
    }

    #[test]
    fn a_handler_panic_closes_the_connection_instead_of_hanging() {
        let (client, mut peer) = connect_with(ClientOptions {
            on_notification: Some(Box::new(|_, _| panic!("handler bug"))),
            ..ClientOptions::default()
        });
        thread::scope(|s| {
            let call = s.spawn(|| client.request("q", Value::Null, LONG));
            peer.recv_request();
            peer.send(Message::Notification {
                method: "boom".into(),
                params: Value::Null,
            });
            assert!(matches!(call.join().unwrap(), Err(Error::Closed { .. })));
        });
    }

    #[test]
    fn nothing_in_the_crate_writes_to_stdout() {
        // Built with concat! so this test's own source doesn't match.
        let banned = [
            concat!("print", "!("),
            concat!("print", "ln!("),
            concat!("std", "out()"),
            concat!("Stdio::", "inherit"),
        ];
        let sources = [
            ("client.rs", include_str!("client.rs")),
            ("framing.rs", include_str!("framing.rs")),
            ("message.rs", include_str!("message.rs")),
            ("server.rs", include_str!("server.rs")),
            ("lib.rs", include_str!("lib.rs")),
        ];
        for (file, source) in sources {
            for b in banned {
                // `eprintln!` is fine: only a match not preceded by an
                // identifier character is a call to the stdout macro.
                let real = source.match_indices(b).any(|(i, _)| {
                    !source[..i]
                        .chars()
                        .next_back()
                        .is_some_and(|c| c.is_alphanumeric() || c == '_')
                });
                assert!(!real, "{file} uses {b}");
            }
        }
    }
}
