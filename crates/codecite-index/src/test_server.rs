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
