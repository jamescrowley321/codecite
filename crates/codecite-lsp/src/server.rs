//! A language-server process: spawned in the workspace root, initialized,
//! waited on until ready, and shut down. Every wait has a deadline, and
//! dropping a [`Server`] shuts it down, so an early return or a panic never
//! leaves the process behind.

use std::collections::{HashSet, VecDeque};
use std::io::{self, Read};
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::{Client, ClientOptions, Error, Result};

/// How often a bounded wait checks the process and the connection.
const POLL: Duration = Duration::from_millis(10);

/// The program and arguments that start a language server on stdio.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerCommand {
    /// The program, looked up on `PATH` when it has no path separator.
    pub program: String,
    /// Its arguments, for example `["--stdio"]`.
    pub args: Vec<String>,
}

impl ServerCommand {
    /// A command with arguments.
    pub fn new(
        program: impl Into<String>,
        args: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            program: program.into(),
            args: args.into_iter().map(Into::into).collect(),
        }
    }
}

/// When a started server counts as ready.
#[derive(Debug, Clone, PartialEq)]
pub enum Readiness {
    /// As soon as `initialized` is sent.
    Initialized,
    /// When every `$/progress` the server began has ended and no progress
    /// event has arrived for `quiet`. A server that reports no progress is
    /// ready after `quiet`.
    Progress {
        /// How long the server must stay quiet.
        quiet: Duration,
    },
    /// When `method` first answers without `ContentModified` or
    /// `RequestCancelled`; it is retried until then.
    Probe {
        /// The probe request's method.
        method: String,
        /// Its params.
        params: Value,
    },
}

/// Timeouts and limits for a [`Server`].
#[derive(Debug, Clone)]
pub struct ServerOptions {
    /// The bound on spawning, `initialize` and readiness together.
    pub startup_timeout: Duration,
    /// How long `shutdown` may take to answer, and then how long the process
    /// may take to exit after `exit`, before it is killed.
    pub shutdown_grace: Duration,
    /// When the server counts as ready.
    pub readiness: Readiness,
    /// How many bytes of the server's stderr to keep for error messages.
    pub stderr_capacity: usize,
    /// The cap on one message's body, in bytes.
    pub max_body: usize,
}

impl Default for ServerOptions {
    fn default() -> Self {
        Self {
            startup_timeout: Duration::from_secs(60),
            shutdown_grace: Duration::from_secs(2),
            readiness: Readiness::Progress {
                quiet: Duration::from_secs(1),
            },
            stderr_capacity: 8 * 1024,
            max_body: crate::framing::DEFAULT_MAX_BODY_BYTES,
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Work-done progress the server has begun and not yet ended.
struct Progress {
    active: HashSet<String>,
    last_event: Instant,
}

type Tracker = Arc<(Mutex<Progress>, Condvar)>;

fn track_progress(tracker: &Tracker, params: &Value) {
    let token = match &params["token"] {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    let (progress, changed) = &**tracker;
    let mut progress = lock(progress);
    match params["value"]["kind"].as_str() {
        Some("begin") => {
            progress.active.insert(token);
        }
        Some("end") => {
            progress.active.remove(&token);
        }
        _ => {}
    }
    progress.last_event = Instant::now();
    changed.notify_all();
}

/// The last `capacity` bytes the server wrote to stderr.
struct StderrTail {
    bytes: Arc<Mutex<VecDeque<u8>>>,
    drain: Option<JoinHandle<()>>,
}

impl StderrTail {
    fn start(mut stderr: impl Read + Send + 'static, capacity: usize) -> io::Result<Self> {
        let bytes = Arc::new(Mutex::new(VecDeque::with_capacity(capacity)));
        let sink = Arc::clone(&bytes);
        let drain = thread::Builder::new()
            .name("codecite-lsp-stderr".into())
            .spawn(move || {
                let mut chunk = [0u8; 4096];
                while let Ok(n @ 1..) = stderr.read(&mut chunk) {
                    let mut tail = lock(&sink);
                    tail.extend(&chunk[..n]);
                    let excess = tail.len().saturating_sub(capacity);
                    tail.drain(..excess);
                }
            })?;
        Ok(Self {
            bytes,
            drain: Some(drain),
        })
    }

    /// The tail as text. After the process exits, waits briefly for the
    /// drain to reach EOF so the last lines are included.
    fn text(&mut self, settle: Duration) -> String {
        let deadline = Instant::now() + settle;
        if let Some(drain) = &self.drain {
            while !drain.is_finished() && Instant::now() < deadline {
                thread::sleep(POLL);
            }
        }
        let tail = lock(&self.bytes);
        let (a, b) = tail.as_slices();
        String::from_utf8_lossy(&[a, b].concat()).trim().to_owned()
    }
}

/// A running language server.
pub struct Server {
    client: Client,
    child: Mutex<Child>,
    stderr: Mutex<StderrTail>,
    shutdown_grace: Duration,
    /// Set once a closed connection has waited the grace period for the
    /// process to exit, so later requests don't each wait again.
    waited_for_exit: AtomicBool,
    exit_status: Option<ExitStatus>,
}

impl std::fmt::Debug for Server {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Server")
            .field("pid", &self.pid())
            .field("client", &self.client)
            .finish_non_exhaustive()
    }
}

impl Server {
    /// Starts `command` with `root` as its working directory and workspace,
    /// initializes it and waits until it is ready, all within
    /// `options.startup_timeout`.
    ///
    /// A program that isn't found is [`Error::ServerNotFound`]; a server
    /// that isn't ready in time is [`Error::ServerNotReady`]; one that dies
    /// on the way is [`Error::ServerExited`]. On any error the process is
    /// shut down before this returns.
    pub fn start(command: &ServerCommand, root: &Path, options: &ServerOptions) -> Result<Self> {
        let started = Instant::now();
        let deadline = started + options.startup_timeout;
        let root = root.canonicalize()?;
        if !root.is_dir() {
            return Err(Error::Io(io::Error::new(
                io::ErrorKind::NotADirectory,
                format!("workspace root {} is not a directory", root.display()),
            )));
        }
        // The root exists, so a NotFound from spawn is the program.
        let mut child = match Command::new(&command.program)
            .args(&command.args)
            .current_dir(&root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(child) => child,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Err(Error::ServerNotFound {
                    command: command.program.clone(),
                });
            }
            Err(e) => return Err(Error::Io(e)),
        };
        let pipes = (child.stdin.take(), child.stdout.take(), child.stderr.take());
        let (Some(stdin), Some(stdout), Some(stderr)) = pipes else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error::Io(io::Error::other("language server pipes missing")));
        };
        let stderr = match StderrTail::start(stderr, options.stderr_capacity) {
            Ok(tail) => tail,
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Error::Io(e));
            }
        };
        let tracker: Tracker = Arc::new((
            Mutex::new(Progress {
                active: HashSet::new(),
                last_event: Instant::now(),
            }),
            Condvar::new(),
        ));
        let handler_tracker = Arc::clone(&tracker);
        let client_options = ClientOptions {
            max_body: options.max_body,
            on_notification: Some(Box::new(move |method, params| {
                if method == "$/progress" {
                    track_progress(&handler_tracker, &params);
                }
            })),
            ..ClientOptions::default()
        };
        let client = match Client::new(stdout, stdin, client_options) {
            Ok(client) => client,
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e);
            }
        };
        // From here, dropping `server` on an error shuts the process down.
        let server = Server {
            client,
            child: Mutex::new(child),
            stderr: Mutex::new(stderr),
            shutdown_grace: options.shutdown_grace,
            waited_for_exit: AtomicBool::new(false),
            exit_status: None,
        };
        server
            .initialize(&root, deadline, &options.readiness, &tracker)
            .map_err(|e| server.startup_error(e, started))?;
        Ok(server)
    }

    fn initialize(
        &self,
        root: &Path,
        deadline: Instant,
        readiness: &Readiness,
        tracker: &Tracker,
    ) -> Result<()> {
        let uri = file_uri(root);
        let name = root
            .file_name()
            .map_or_else(|| uri.clone(), |n| n.to_string_lossy().into_owned());
        let params = json!({
            "processId": std::process::id(),
            "clientInfo": {"name": "codecite", "version": env!("CARGO_PKG_VERSION")},
            "rootUri": uri,
            "workspaceFolders": [{"uri": uri, "name": name}],
            "capabilities": client_capabilities(),
        });
        self.client
            .request("initialize", params, remaining(deadline))?;
        self.client
            .notify_within("initialized", json!({}), remaining(deadline))?;
        // The quiet period counts from here, however long startup took.
        lock(&tracker.0).last_event = Instant::now();
        match readiness {
            Readiness::Initialized => Ok(()),
            Readiness::Progress { quiet } => self.wait_for_progress(tracker, *quiet, deadline),
            Readiness::Probe { method, params } => self.probe(method, params, deadline),
        }
    }

    fn wait_for_progress(
        &self,
        tracker: &Tracker,
        quiet: Duration,
        deadline: Instant,
    ) -> Result<()> {
        let (progress, changed) = &**tracker;
        let mut state = lock(progress);
        loop {
            let now = Instant::now();
            let idle = now.saturating_duration_since(state.last_event);
            if state.active.is_empty() && idle >= quiet {
                return Ok(());
            }
            if now >= deadline {
                return Err(Error::Timeout {
                    method: "$/progress".into(),
                    elapsed: Duration::ZERO,
                });
            }
            if let Some(reason) = self.client.closed_reason() {
                return Err(Error::Closed { reason });
            }
            let wait = if state.active.is_empty() {
                quiet - idle
            } else {
                deadline - now
            };
            state = changed
                .wait_timeout(state, wait.min(deadline - now).min(POLL * 10))
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    fn probe(&self, method: &str, params: &Value, deadline: Instant) -> Result<()> {
        loop {
            match self
                .client
                .request(method, params.clone(), remaining(deadline))
            {
                Ok(_) => return Ok(()),
                Err(Error::ContentModified { .. } | Error::RequestCancelled { .. }) => {
                    if Instant::now() >= deadline {
                        return Err(Error::Timeout {
                            method: method.to_owned(),
                            elapsed: Duration::ZERO,
                        });
                    }
                    thread::sleep((POLL * 10).min(remaining(deadline)));
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// Turns a failure during startup into the error callers act on: any
    /// deadline passing is `ServerNotReady`, a lost connection is usually
    /// `ServerExited`.
    fn startup_error(&self, e: Error, started: Instant) -> Error {
        match e {
            Error::Timeout { .. } => Error::ServerNotReady {
                elapsed: started.elapsed(),
                stderr: self.stderr_tail(),
            },
            e => self.exited_or(e),
        }
    }

    /// The server's process id.
    pub fn pid(&self) -> u32 {
        lock(&self.child).id()
    }

    /// The tail of what the server wrote to stderr.
    pub fn stderr_tail(&self) -> String {
        lock(&self.stderr).text(Duration::ZERO)
    }

    /// The JSON-RPC client, for notifications and requests that need no
    /// exit check.
    pub fn client(&self) -> &Client {
        &self.client
    }

    /// Sends a request and waits up to `timeout`. If the server has exited,
    /// this is [`Error::ServerExited`] with its status and stderr tail.
    pub fn request(&self, method: &str, params: Value, timeout: Duration) -> Result<Value> {
        if let Some(status) = self.try_exit_status() {
            return Err(self.exited(status));
        }
        self.client
            .request(method, params, timeout)
            .map_err(|e| self.exited_or(e))
    }

    fn try_exit_status(&self) -> Option<ExitStatus> {
        lock(&self.child).try_wait().ok().flatten()
    }

    fn wait_for_exit(&self, within: Duration) -> Option<ExitStatus> {
        let deadline = Instant::now() + within;
        loop {
            if let Some(status) = self.try_exit_status() {
                return Some(status);
            }
            if Instant::now() >= deadline {
                return None;
            }
            thread::sleep(POLL);
        }
    }

    fn exited(&self, status: ExitStatus) -> Error {
        Error::ServerExited {
            status,
            stderr: lock(&self.stderr).text(Duration::from_millis(500)),
        }
    }

    /// A closed connection usually means the process died: report that,
    /// with its status, if it exits within the grace period. Only the first
    /// closed error waits; later ones just check.
    fn exited_or(&self, e: Error) -> Error {
        match e {
            Error::Closed { .. } => {
                let status = if self.waited_for_exit.swap(true, Ordering::SeqCst) {
                    self.try_exit_status()
                } else {
                    self.wait_for_exit(self.shutdown_grace)
                };
                match status {
                    Some(status) => self.exited(status),
                    None => e,
                }
            }
            e => e,
        }
    }

    /// Sends `shutdown` and `exit`, closes the server's input, waits the
    /// grace period for it to exit, then kills it. Returns its exit status.
    /// Writing `shutdown` and its answer share one grace period, and so does
    /// writing `exit`, so a server that stops reading is still killed.
    pub fn shutdown(mut self) -> Result<ExitStatus> {
        self.stop()
    }

    fn stop(&mut self) -> Result<ExitStatus> {
        if let Some(status) = self.exit_status {
            return Ok(status);
        }
        let status = match self.try_exit_status() {
            Some(status) => status,
            None => {
                let grace = self.shutdown_grace;
                if self.client.request("shutdown", Value::Null, grace).is_ok() {
                    let _ = self.client.notify_within("exit", Value::Null, grace);
                }
                self.client.close();
                match self.wait_for_exit(grace) {
                    Some(status) => status,
                    None => {
                        let mut child = lock(&self.child);
                        // It may exit between the last check and the kill.
                        let _ = child.kill();
                        child.wait()?
                    }
                }
            }
        };
        self.exit_status = Some(status);
        Ok(status)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if let Err(e) = self.stop() {
            eprintln!("codecite-lsp: stopping the language server failed: {e}");
        }
    }
}

fn remaining(deadline: Instant) -> Duration {
    deadline.saturating_duration_since(Instant::now())
}

/// What codecite asks a server for: definitions, references, call hierarchy
/// and work-done progress. No edits, code actions or formatting.
fn client_capabilities() -> Value {
    json!({
        "textDocument": {
            "definition": {"dynamicRegistration": false, "linkSupport": false},
            "references": {"dynamicRegistration": false},
            "callHierarchy": {"dynamicRegistration": false},
        },
        "window": {"workDoneProgress": true},
    })
}

/// A `file://` URI for an absolute path, percent-encoding every byte
/// outside RFC 3986's unreserved set, `/` and `:`. On Windows, backslashes
/// become slashes and the verbatim prefix `canonicalize` adds is dropped.
pub fn file_uri(path: &Path) -> String {
    uri_from_path_bytes(path.as_os_str().as_encoded_bytes(), cfg!(windows))
}

fn uri_from_path_bytes(bytes: &[u8], windows: bool) -> String {
    let mut path = bytes.to_vec();
    let mut uri = String::from("file:");
    if windows {
        for byte in &mut path {
            if *byte == b'\\' {
                *byte = b'/';
            }
        }
        if let Some(rest) = path.strip_prefix(b"//?/UNC/") {
            path = [b"//".as_slice(), rest].concat();
        } else if let Some(rest) = path.strip_prefix(b"//?/") {
            path = rest.to_vec();
        }
    }
    if windows && path.starts_with(b"//") {
        // A UNC path's server is the URI's authority: file://server/share.
    } else {
        uri.push_str("//");
        if path.first() != Some(&b'/') {
            uri.push('/');
        }
    }
    for byte in path {
        if byte.is_ascii_alphanumeric() || b"-._~/:".contains(&byte) {
            uri.push(char::from(byte));
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }
    uri
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_uris_percent_encode_reserved_bytes() {
        assert_eq!(file_uri(Path::new("/w/src")), "file:///w/src");
        assert_eq!(
            file_uri(Path::new("/my work/a#b%c")),
            "file:///my%20work/a%23b%25c"
        );
        assert_eq!(file_uri(Path::new("/caf\u{e9}")), "file:///caf%C3%A9");
    }

    #[test]
    fn windows_file_uris_drop_the_verbatim_prefix() {
        let windows = |path: &str| uri_from_path_bytes(path.as_bytes(), true);
        assert_eq!(windows(r"C:\w\src"), "file:///C:/w/src");
        assert_eq!(windows(r"\\?\C:\my work"), "file:///C:/my%20work");
        assert_eq!(windows(r"\\server\share\x"), "file://server/share/x");
        assert_eq!(windows(r"\\?\UNC\server\share\x"), "file://server/share/x");
        // Off Windows a backslash is an ordinary byte.
        assert_eq!(uri_from_path_bytes(br"/a\b", false), "file:///a%5Cb");
    }

    #[test]
    fn capabilities_ask_only_for_navigation_and_progress() {
        let caps = client_capabilities();
        let text_document: Vec<_> = caps["textDocument"].as_object().unwrap().keys().collect();
        assert_eq!(text_document, ["callHierarchy", "definition", "references"]);
        assert_eq!(caps["window"], json!({"workDoneProgress": true}));
        assert!(caps.get("workspace").is_none());
    }

    #[test]
    fn progress_tracks_begun_and_ended_tokens() {
        let tracker: Tracker = Arc::new((
            Mutex::new(Progress {
                active: HashSet::new(),
                last_event: Instant::now(),
            }),
            Condvar::new(),
        ));
        track_progress(&tracker, &json!({"token": "a", "value": {"kind": "begin"}}));
        track_progress(&tracker, &json!({"token": 7, "value": {"kind": "begin"}}));
        track_progress(
            &tracker,
            &json!({"token": "a", "value": {"kind": "report"}}),
        );
        track_progress(&tracker, &json!({"token": "a", "value": {"kind": "end"}}));
        assert_eq!(lock(&tracker.0).active, HashSet::from(["7".to_owned()]));
        track_progress(&tracker, &json!({"token": 7, "value": {"kind": "end"}}));
        assert!(lock(&tracker.0).active.is_empty());
    }
}
