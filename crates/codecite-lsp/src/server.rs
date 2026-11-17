//! A language-server process: spawned in the workspace root, initialized,
//! waited on until ready, and shut down. Every wait has a deadline, and
//! dropping a [`Server`] shuts it down, so an early return or a panic never
//! leaves the process behind.

use std::collections::{HashSet, VecDeque};
use std::ffi::OsStr;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::deadline::Deadline;
use crate::message::truncate;
use crate::{Client, ClientOptions, Error, Result};

/// How often a bounded wait checks the process and the connection.
const POLL: Duration = Duration::from_millis(10);

/// The program and arguments that start a language server on stdio.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerCommand {
    /// The program. One without a path separator is looked up on `PATH`,
    /// skipping relative and empty entries; a relative path is taken from
    /// codecite's own working directory. Never from the workspace root.
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

/// The most progress tokens tracked at once; later `begin`s are ignored.
const MAX_PROGRESS_TOKENS: usize = 1024;
/// The most bytes of a progress token kept.
const MAX_TOKEN_BYTES: usize = 256;

/// Work-done progress the server has begun and not yet ended.
struct Progress {
    active: HashSet<String>,
    last_event: Instant,
}

type Tracker = Arc<(Mutex<Progress>, Condvar)>;

fn track_progress(tracker: &Tracker, params: &Value) {
    let token = match &params["token"] {
        Value::String(s) => truncate(s, MAX_TOKEN_BYTES).to_owned(),
        other => truncate(&other.to_string(), MAX_TOKEN_BYTES).to_owned(),
    };
    let (progress, changed) = &**tracker;
    let mut progress = lock(progress);
    match params["value"]["kind"].as_str() {
        Some("begin") if progress.active.len() < MAX_PROGRESS_TOKENS => {
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
    fn start(stderr: impl Read + Send + 'static, capacity: usize) -> io::Result<Self> {
        let bytes = Arc::new(Mutex::new(VecDeque::with_capacity(capacity)));
        let sink = Arc::clone(&bytes);
        let drain = thread::Builder::new()
            .name("codecite-lsp-stderr".into())
            .spawn(move || drain_into(stderr, &sink, capacity))?;
        Ok(Self {
            bytes,
            drain: Some(drain),
        })
    }

    /// The tail as text. After the process exits, waits briefly for the
    /// drain to reach EOF so the last lines are included.
    fn text(&mut self, settle: Duration) -> String {
        let deadline = Deadline::after(settle);
        if let Some(drain) = &self.drain {
            while !drain.is_finished() && !deadline.passed() {
                thread::sleep(POLL);
            }
        }
        let tail = lock(&self.bytes);
        let (a, b) = tail.as_slices();
        String::from_utf8_lossy(&[a, b].concat()).trim().to_owned()
    }
}

/// Reads `stderr` to its end, keeping the last `capacity` bytes in `sink`.
/// An interrupted read is retried; any other error ends it.
fn drain_into(mut stderr: impl Read, sink: &Mutex<VecDeque<u8>>, capacity: usize) {
    let mut chunk = [0u8; 4096];
    loop {
        let n = match stderr.read(&mut chunk) {
            Ok(0) => return,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => return,
        };
        let mut tail = lock(sink);
        tail.extend(&chunk[..n]);
        let excess = tail.len().saturating_sub(capacity);
        tail.drain(..excess);
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
        let deadline = Deadline::at(started, options.startup_timeout);
        let root = root.canonicalize()?;
        if !root.is_dir() {
            return Err(Error::Io(io::Error::new(
                io::ErrorKind::NotADirectory,
                format!("workspace root {} is not a directory", root.display()),
            )));
        }
        // Resolved here: the child's own lookup would run after it moves into
        // the root, where a relative path or PATH entry could find a planted
        // program.
        let not_found = || Error::ServerNotFound {
            command: command.program.clone(),
        };
        let cwd = std::env::current_dir()?;
        let program = resolve_program(&command.program, std::env::var_os("PATH").as_deref(), &cwd)
            .ok_or_else(not_found)?;
        // The root exists, so a NotFound from spawn is the program.
        let mut child = match Command::new(&program)
            .args(&command.args)
            .current_dir(&root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(child) => child,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(not_found()),
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
        deadline: Deadline,
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
            .request("initialize", params, deadline.remaining())?;
        self.client
            .notify_within("initialized", json!({}), deadline.remaining())?;
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
        deadline: Deadline,
    ) -> Result<()> {
        let (progress, changed) = &**tracker;
        let mut state = lock(progress);
        loop {
            let idle = state.last_event.elapsed();
            if state.active.is_empty() && idle >= quiet {
                return Ok(());
            }
            if deadline.passed() {
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
                deadline.remaining()
            };
            state = changed
                .wait_timeout(state, wait.min(deadline.remaining()).min(POLL * 10))
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    fn probe(&self, method: &str, params: &Value, deadline: Deadline) -> Result<()> {
        loop {
            match self
                .client
                .request(method, params.clone(), deadline.remaining())
            {
                Ok(_) => return Ok(()),
                Err(Error::ContentModified { .. } | Error::RequestCancelled { .. }) => {
                    if deadline.passed() {
                        return Err(Error::Timeout {
                            method: method.to_owned(),
                            elapsed: Duration::ZERO,
                        });
                    }
                    thread::sleep((POLL * 10).min(deadline.remaining()));
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
        let deadline = Deadline::after(within);
        loop {
            if let Some(status) = self.try_exit_status() {
                return Some(status);
            }
            if deadline.passed() {
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

/// Where `program` runs from, as an absolute path: a path is taken from
/// `cwd` (codecite's working directory), and a bare name is the first
/// executable of that name in an absolute `PATH` entry. Relative and empty
/// entries are skipped, since they would resolve inside the workspace root.
fn resolve_program(program: &str, path: Option<&OsStr>, cwd: &Path) -> Option<PathBuf> {
    let candidate = Path::new(program);
    if candidate.components().count() != 1 || program.contains('/') {
        return Some(cwd.join(candidate));
    }
    std::env::split_paths(path?)
        .filter(|dir| dir.is_absolute())
        .flat_map(|dir| executable_names(program).map(move |name| dir.join(name)))
        .find(|candidate| is_executable(candidate))
}

/// The file names a program may have: on Windows, with each usual suffix.
fn executable_names(program: &str) -> impl Iterator<Item = String> + '_ {
    let suffixes: &[&str] = if cfg!(windows) {
        &["", ".exe", ".cmd", ".bat"]
    } else {
        &[""]
    };
    suffixes
        .iter()
        .map(move |suffix| format!("{program}{suffix}"))
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
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

    #[cfg(unix)]
    #[test]
    fn bare_programs_resolve_only_against_absolute_path_entries() {
        use std::os::unix::fs::PermissionsExt;
        let bin = tempfile::tempdir().unwrap();
        let program = bin.path().join("lsp-under-test");
        std::fs::write(&program, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(bin.path().join("not-executable"), "").unwrap();
        let dir = bin.path().to_str().unwrap();
        let path = |entries: &str| Some(std::ffi::OsString::from(entries));
        assert_eq!(
            resolve_program(
                "lsp-under-test",
                path(&format!("/nonexistent:{dir}")).as_deref(),
                Path::new("/")
            ),
            Some(program.clone())
        );
        assert_eq!(
            resolve_program("not-executable", path(dir).as_deref(), Path::new("/")),
            None
        );
        // Relative and empty entries are never searched, even one that
        // names the program's directory from here.
        let ups = "../".repeat(std::env::current_dir().unwrap().components().count() - 1);
        let relative = format!("{ups}{}", dir.trim_start_matches('/'));
        assert!(Path::new(&relative).join("lsp-under-test").exists());
        assert_eq!(
            resolve_program(
                "lsp-under-test",
                path(&format!(":.:{relative}:")).as_deref(),
                Path::new("/")
            ),
            None
        );
        assert_eq!(
            resolve_program("lsp-under-test", None, Path::new("/")),
            None
        );
    }

    #[test]
    fn relative_program_paths_resolve_against_codecite_s_directory() {
        let cwd = Path::new("/codecite/cwd");
        assert_eq!(
            resolve_program("./bin/x", None, cwd),
            Some(cwd.join("./bin/x"))
        );
        assert_eq!(
            resolve_program("node_modules/.bin/lsp", None, cwd),
            Some(cwd.join("node_modules/.bin/lsp"))
        );
        assert_eq!(
            resolve_program("/usr/bin/lsp", None, cwd),
            Some(PathBuf::from("/usr/bin/lsp"))
        );
    }

    /// Fails with `Interrupted` before every chunk, then ends.
    struct Interrupting {
        chunks: Vec<&'static [u8]>,
        interrupt: bool,
    }

    impl Read for Interrupting {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.interrupt = !self.interrupt;
            if self.interrupt {
                return Err(io::ErrorKind::Interrupted.into());
            }
            match self.chunks.pop() {
                Some(chunk) => {
                    buf[..chunk.len()].copy_from_slice(chunk);
                    Ok(chunk.len())
                }
                None => Ok(0),
            }
        }
    }

    #[test]
    fn the_stderr_drain_retries_interrupted_reads() {
        let sink = Mutex::new(VecDeque::new());
        let stderr = Interrupting {
            chunks: vec![b"world", b"hello "],
            interrupt: false,
        };
        drain_into(stderr, &sink, 8);
        assert_eq!(lock(&sink).iter().copied().collect::<Vec<_>>(), b"lo world");
    }

    #[test]
    fn progress_tracking_is_bounded() {
        let tracker: Tracker = Arc::new((
            Mutex::new(Progress {
                active: HashSet::new(),
                last_event: Instant::now(),
            }),
            Condvar::new(),
        ));
        for n in 0..MAX_PROGRESS_TOKENS * 2 {
            let token = format!("{n}{}", "t".repeat(1000));
            track_progress(
                &tracker,
                &json!({"token": token, "value": {"kind": "begin"}}),
            );
        }
        let progress = lock(&tracker.0);
        assert_eq!(progress.active.len(), MAX_PROGRESS_TOKENS);
        assert!(progress.active.iter().all(|t| t.len() <= MAX_TOKEN_BYTES));
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
