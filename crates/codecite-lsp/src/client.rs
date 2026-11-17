//! A blocking JSON-RPC 2.0 client over any reader and writer pair.
//!
//! One reader thread owns the server's output and routes every message:
//! a response to the channel its request waits on, a notification to the
//! caller's handler, a server-to-client request to a built-in reply. One
//! writer thread owns the server's input and writes what the others queue.
//! Callers block on their own write and request with a deadline; the reader
//! queues its replies and never waits on the writer, so a server that stops
//! reading can't stall it. Nothing is written to stdout, which belongs to
//! the MCP server; diagnostics go to stderr.

use std::collections::HashMap;
use std::io::{BufReader, Read, Write};
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::framing::{DEFAULT_MAX_BODY_BYTES, read_message, write_message};
use crate::message::{
    CONTENT_MODIFIED, Id, METHOD_NOT_FOUND, Message, REQUEST_CANCELLED, ResponseError,
};
use crate::{Error, Result};

/// The most messages that may wait for the writer. A server that stops
/// reading while it keeps sending requests fills this, and the connection
/// closes instead of queueing replies without bound.
const MAX_QUEUED: usize = 1024;

/// Called on the reader thread with each server notification's method and
/// params. It must not block for long: the next message waits on it.
pub type NotificationHandler = Box<dyn FnMut(&str, Value) + Send>;

/// How a [`Client`] reads and writes.
pub struct ClientOptions {
    /// The cap on one message's body, in bytes.
    pub max_body: usize,
    /// How long [`Client::notify`] waits for its message to be written.
    pub write_timeout: Duration,
    /// Receives server notifications; `None` drops them.
    pub on_notification: Option<NotificationHandler>,
}

impl Default for ClientOptions {
    fn default() -> Self {
        Self {
            max_body: DEFAULT_MAX_BODY_BYTES,
            write_timeout: Duration::from_secs(10),
            on_notification: None,
        }
    }
}

impl std::fmt::Debug for ClientOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientOptions")
            .field("max_body", &self.max_body)
            .field("write_timeout", &self.write_timeout)
            .field("on_notification", &self.on_notification.is_some())
            .finish()
    }
}

enum Failure {
    Rpc(ResponseError),
    Closed(String),
}

type Reply = std::result::Result<Value, Failure>;

/// Why a message wasn't written.
enum SendFailure {
    Closed,
    TimedOut,
}

/// A message for the writer thread, and where to say it was written.
struct Outgoing {
    message: Value,
    written: Option<SyncSender<()>>,
}

#[derive(Default)]
struct State {
    pending: HashMap<i64, SyncSender<Reply>>,
    closed: Option<String>,
}

struct Shared {
    /// Feeds the writer thread; `None` once closed, which ends that thread
    /// after it writes what is already queued.
    outbox: Mutex<Option<Sender<Outgoing>>>,
    /// How many messages are queued and not yet written.
    queued: AtomicUsize,
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

    /// Queues a message for the writer and never blocks on it. With `wait`,
    /// waits that long for it to be written; past that the connection is
    /// closed, since part of a frame may already be on the wire.
    fn send(
        &self,
        message: &Message,
        wait: Option<Duration>,
    ) -> std::result::Result<(), SendFailure> {
        let (written, done) = match wait {
            Some(_) => {
                let (tx, rx) = mpsc::sync_channel(1);
                (Some(tx), Some(rx))
            }
            None => (None, None),
        };
        let queued = {
            let outbox = lock(&self.outbox);
            let Some(outbox) = outbox.as_ref() else {
                return Err(SendFailure::Closed);
            };
            let queued = self.queued.fetch_add(1, Ordering::SeqCst);
            if queued < MAX_QUEUED {
                outbox
                    .send(Outgoing {
                        message: message.to_value(),
                        written,
                    })
                    .is_ok()
            } else {
                false
            }
        };
        if !queued {
            self.queued.fetch_sub(1, Ordering::SeqCst);
            self.close(&format!(
                "the server is not reading its input: {MAX_QUEUED} messages are waiting"
            ));
            return Err(SendFailure::Closed);
        }
        let (Some(wait), Some(done)) = (wait, done) else {
            return Ok(());
        };
        match done.recv_timeout(wait) {
            Ok(()) => Ok(()),
            Err(RecvTimeoutError::Disconnected) => Err(SendFailure::Closed),
            Err(RecvTimeoutError::Timeout) => {
                self.close(&format!("writing to the server timed out after {wait:?}"));
                Err(SendFailure::TimedOut)
            }
        }
    }

    /// Marks the connection closed, fails every in-flight request at once
    /// and stops queueing writes. The first reason sticks.
    fn close(&self, reason: &str) {
        let pending = {
            let mut state = lock(&self.state);
            state.closed.get_or_insert_with(|| reason.to_owned());
            std::mem::take(&mut state.pending)
        };
        for (_, tx) in pending {
            let _ = tx.try_send(Err(Failure::Closed(reason.to_owned())));
        }
        lock(&self.outbox).take();
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
    write_timeout: Duration,
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
    /// `writer` (its input). The reader thread runs until `reader` ends; the
    /// writer thread until the connection closes or a write fails. A writer
    /// stuck on a server that never reads again ends when the server dies.
    pub fn new<R, W>(reader: R, writer: W, options: ClientOptions) -> Result<Self>
    where
        R: Read + Send + 'static,
        W: Write + Send + 'static,
    {
        let (outbox, inbox) = mpsc::channel();
        let shared = Arc::new(Shared {
            outbox: Mutex::new(Some(outbox)),
            queued: AtomicUsize::new(0),
            state: Mutex::new(State::default()),
        });
        let write_timeout = options.write_timeout;
        let writer_shared = Arc::clone(&shared);
        thread::Builder::new()
            .name("codecite-lsp-writer".into())
            .spawn(move || write_loop(&writer_shared, writer, &inbox))?;
        let reader_shared = Arc::clone(&shared);
        let spawned = thread::Builder::new()
            .name("codecite-lsp-reader".into())
            .spawn(move || read_loop(reader_shared, reader, options));
        if let Err(e) = spawned {
            shared.close("the reader thread did not start");
            return Err(e.into());
        }
        Ok(Self {
            shared,
            next_id: AtomicI64::new(1),
            write_timeout,
        })
    }

    /// Sends a request and waits up to `timeout`, writing it included, for
    /// its response. If the response is late it sends `$/cancelRequest` and
    /// the late response is dropped; if the write itself is late the
    /// connection is closed.
    pub fn request(&self, method: &str, params: Value, timeout: Duration) -> Result<Value> {
        let start = Instant::now();
        let deadline = start + timeout;
        let timed_out = || Error::Timeout {
            method: method.to_owned(),
            elapsed: start.elapsed(),
        };
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
        if let Err(failure) = self.shared.send(&message, Some(timeout)) {
            lock(&self.shared.state).pending.remove(&id);
            return Err(match failure {
                SendFailure::Closed => self.shared.closed_error(),
                SendFailure::TimedOut => timed_out(),
            });
        }
        let wait = deadline.saturating_duration_since(Instant::now());
        let reply = match rx.recv_timeout(wait) {
            Ok(reply) => reply,
            Err(RecvTimeoutError::Disconnected) => return Err(self.shared.closed_error()),
            Err(RecvTimeoutError::Timeout) => {
                lock(&self.shared.state).pending.remove(&id);
                // The reply may have landed between the timeout and the removal.
                match rx.try_recv() {
                    Ok(reply) => reply,
                    Err(_) => {
                        let cancel = Message::Notification {
                            method: "$/cancelRequest".into(),
                            params: json!({ "id": id }),
                        };
                        let _ = self.shared.send(&cancel, None);
                        return Err(timed_out());
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

    /// Sends a notification, waiting up to the client's `write_timeout` for
    /// it to be written.
    pub fn notify(&self, method: &str, params: Value) -> Result<()> {
        self.notify_within(method, params, self.write_timeout)
    }

    /// Sends a notification, waiting up to `timeout` for it to be written.
    /// Past that the connection is closed and this is [`Error::Timeout`].
    pub fn notify_within(&self, method: &str, params: Value, timeout: Duration) -> Result<()> {
        let start = Instant::now();
        let message = Message::Notification {
            method: method.to_owned(),
            params,
        };
        self.shared
            .send(&message, Some(timeout))
            .map_err(|failure| match failure {
                SendFailure::Closed => self.shared.closed_error(),
                SendFailure::TimedOut => Error::Timeout {
                    method: method.to_owned(),
                    elapsed: start.elapsed(),
                },
            })
    }

    /// Closes the connection: in-flight and later requests fail with
    /// [`Error::Closed`]. What is already queued is still written, then the
    /// server's input is closed, which it sees as the end of its input.
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

/// Writes queued messages in order until the queue closes or a write fails.
/// Dropping `writer` on return closes the server's input.
fn write_loop<W: Write>(shared: &Shared, mut writer: W, inbox: &Receiver<Outgoing>) {
    while let Ok(outgoing) = inbox.recv() {
        let result = write_message(&mut writer, &outgoing.message);
        shared.queued.fetch_sub(1, Ordering::SeqCst);
        match result {
            Ok(()) => {
                if let Some(written) = outgoing.written {
                    let _ = written.try_send(());
                }
            }
            Err(e) => {
                // Closed before `outgoing.written` drops, so its waiter
                // sees the reason.
                shared.close(&format!("writing to the server failed: {e}"));
                return;
            }
        }
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
                // Queued, never waited on: the writer may be stuck.
                if shared.send(&reply, None).is_err() {
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

    /// More than any OS pipe buffers, so writing it blocks until the peer reads.
    fn bigger_than_a_pipe() -> Value {
        json!({ "pad": "x".repeat(1024 * 1024) })
    }

    #[test]
    fn a_peer_that_stops_reading_times_out_the_request() {
        let (client, peer) = connect();
        let timeout = Duration::from_millis(200);
        let start = Instant::now();
        match client.request("big", bigger_than_a_pipe(), timeout) {
            Err(Error::Timeout { method, elapsed }) => {
                assert_eq!(method, "big");
                assert!(elapsed >= timeout, "{elapsed:?}");
            }
            other => panic!("expected a timeout, got {other:?}"),
        }
        assert!(start.elapsed() < LONG / 2, "{:?}", start.elapsed());
        // Half a frame may be on the wire, so the connection is closed.
        assert!(client.closed_reason().unwrap().contains("timed out"));
        assert!(matches!(
            client.request("later", Value::Null, LONG),
            Err(Error::Closed { .. })
        ));
        assert!(matches!(
            client.notify("later", Value::Null),
            Err(Error::Closed { .. })
        ));
        drop(peer);
    }

    #[test]
    fn the_reader_never_waits_on_a_stuck_write() {
        let (tx, rx) = mpsc::channel();
        let (client, mut peer) = connect_with(ClientOptions {
            on_notification: Some(Box::new(move |method, _| {
                tx.send(method.to_owned()).unwrap();
            })),
            ..ClientOptions::default()
        });
        thread::scope(|s| {
            // Blocks writing: the peer isn't reading.
            let stuck = s.spawn(|| client.request("big", bigger_than_a_pipe(), LONG));
            thread::sleep(Duration::from_millis(50));
            for n in 0..3 {
                peer.send(Message::Request {
                    id: Id::Number(n),
                    method: "window/workDoneProgress/create".into(),
                    params: json!({"token": n}),
                });
            }
            peer.send(Message::Notification {
                method: "after".into(),
                params: Value::Null,
            });
            // The replies wait their turn; the reader moved on regardless.
            assert_eq!(rx.recv_timeout(LONG / 2).unwrap(), "after");
            peer.reader.take();
            assert!(matches!(stuck.join().unwrap(), Err(Error::Closed { .. })));
        });
    }

    #[test]
    fn a_peer_flooding_requests_without_reading_closes_the_connection() {
        let (client, mut peer) = connect();
        for n in 0..(MAX_QUEUED as i64 * 8) {
            let request = Message::Request {
                id: Id::Number(n),
                method: "window/workDoneProgress/create".into(),
                params: json!({"token": n}),
            };
            // Once the client closes, its reader is gone and writes fail.
            let sent = write_message(peer.writer.as_mut().unwrap(), &request.to_value());
            if sent.is_err() || client.closed_reason().is_some() {
                break;
            }
        }
        let deadline = Instant::now() + LONG / 2;
        while client.closed_reason().is_none() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        let reason = client.closed_reason().expect("closed");
        assert!(reason.contains("not reading"), "{reason:?}");
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
        // Every source file, read at run time so a new one can't be missed.
        let mut sources = Vec::new();
        let mut dirs = vec![std::path::PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src"
        ))];
        while let Some(dir) = dirs.pop() {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    dirs.push(path);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    let source = std::fs::read_to_string(&path).unwrap();
                    sources.push((path.display().to_string(), source));
                }
            }
        }
        let names: Vec<_> = sources.iter().map(|(file, _)| file.as_str()).collect();
        for expected in ["client.rs", "error.rs", "framing.rs", "lib.rs", "server.rs"] {
            assert!(
                names.iter().any(|n| n.ends_with(expected)),
                "{expected} not scanned: {names:?}"
            );
        }
        for (file, source) in &sources {
            let source = source.as_str();
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

    #[test]
    fn notify_within_bounds_the_write() {
        let (client, _peer) = connect();
        let timeout = Duration::from_millis(100);
        let start = Instant::now();
        let err = client
            .notify_within("big", bigger_than_a_pipe(), timeout)
            .unwrap_err();
        assert!(
            matches!(&err, Error::Timeout { method, .. } if method == "big"),
            "{err:?}"
        );
        assert!(start.elapsed() < LONG / 2, "{:?}", start.elapsed());
    }

    #[test]
    fn closing_flushes_what_was_already_sent() {
        let (client, mut peer) = connect();
        client.notify("exit", Value::Null).unwrap();
        client.close();
        assert_eq!(
            peer.recv(),
            Message::Notification {
                method: "exit".into(),
                params: Value::Null
            }
        );
        let reader = peer.reader.as_mut().unwrap();
        assert!(
            read_message(reader, DEFAULT_MAX_BODY_BYTES)
                .unwrap()
                .is_none()
        );
    }
}
