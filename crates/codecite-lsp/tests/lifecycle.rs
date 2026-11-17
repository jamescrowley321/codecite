//! Language-server lifecycle tests, against a scripted fake server.
//!
//! This test has no libtest harness: its `main` is also the fake server.
//! Run as `lifecycle --fake-lsp <scenario>` it speaks scripted LSP on stdio;
//! run any other way it runs the tests, each spawning this same executable
//! as its server. So CI needs no real language server, and later M7 tests
//! can add scenarios here.

use std::io::{self, BufReader, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::process::{self, Command};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use codecite_lsp::framing::{DEFAULT_MAX_BODY_BYTES, read_message, write_message};
use codecite_lsp::message::{CONTENT_MODIFIED, METHOD_NOT_FOUND, Message, ResponseError};
use codecite_lsp::{Error, Readiness, Server, ServerCommand, ServerOptions, file_uri};
use serde_json::{Value, json};

/// A fake server never outlives this, even if a test leaks it.
const FAKE_LIFETIME: Duration = Duration::from_secs(60);
/// A test that runs longer than this fails, so none can hang the suite.
const TEST_LIMIT: Duration = Duration::from_secs(30);

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--fake-lsp") {
        fake::run(args.get(2).map_or("clean", String::as_str));
    }
    let filter = args.iter().skip(1).find(|a| !a.starts_with('-')).cloned();
    run_tests(filter.as_deref());
}

type Test = (&'static str, fn());

const TESTS: &[Test] = &[
    ("clean_start_and_shutdown", clean_start_and_shutdown),
    (
        "initialize_sends_root_and_limited_capabilities",
        initialize_sends_root_and_limited_capabilities,
    ),
    (
        "startup_timeout_is_server_not_ready",
        startup_timeout_is_server_not_ready,
    ),
    (
        "progress_that_never_ends_is_server_not_ready",
        progress_that_never_ends_is_server_not_ready,
    ),
    (
        "probe_retries_content_modified_until_ready",
        probe_retries_content_modified_until_ready,
    ),
    (
        "crash_during_startup_is_server_exited",
        crash_during_startup_is_server_exited,
    ),
    (
        "crash_after_startup_fails_later_requests",
        crash_after_startup_fails_later_requests,
    ),
    (
        "missing_command_is_server_not_found",
        missing_command_is_server_not_found,
    ),
    (
        "ignored_shutdown_is_killed_after_the_grace_period",
        ignored_shutdown_is_killed_after_the_grace_period,
    ),
    (
        "dropping_the_handle_leaves_no_process",
        dropping_the_handle_leaves_no_process,
    ),
    (
        "dropping_a_stuck_server_leaves_no_process",
        dropping_a_stuck_server_leaves_no_process,
    ),
    (
        "slow_reply_times_out_and_the_server_lives_on",
        slow_reply_times_out_and_the_server_lives_on,
    ),
];

/// Runs every test on its own thread, each bounded by `TEST_LIMIT`.
fn run_tests(filter: Option<&str>) {
    let selected: Vec<&Test> = TESTS
        .iter()
        .filter(|(name, _)| filter.is_none_or(|f| name.contains(f)))
        .collect();
    println!("\nrunning {} tests", selected.len());
    let (tx, rx) = mpsc::channel();
    for &&(name, test) in &selected {
        let tx = tx.clone();
        thread::spawn(move || {
            let outcome = catch_unwind(AssertUnwindSafe(test));
            let _ = tx.send((name, outcome.is_ok()));
        });
    }
    drop(tx);
    let deadline = Instant::now() + TEST_LIMIT;
    let mut failed = Vec::new();
    for _ in &selected {
        match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok((name, true)) => println!("test {name} ... ok"),
            Ok((name, false)) => {
                println!("test {name} ... FAILED");
                failed.push(name);
            }
            Err(_) => {
                println!("tests still running after {TEST_LIMIT:?}");
                process::exit(1);
            }
        }
    }
    if failed.is_empty() {
        println!("\ntest result: ok. {} passed", selected.len());
    } else {
        println!("\ntest result: FAILED. {} failed: {failed:?}", failed.len());
        process::exit(1);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

const LONG: Duration = Duration::from_secs(10);

fn fake(scenario: &str) -> ServerCommand {
    let exe = std::env::current_exe().expect("current exe");
    ServerCommand::new(exe.to_string_lossy(), ["--fake-lsp", scenario])
}

fn options() -> ServerOptions {
    ServerOptions {
        startup_timeout: LONG,
        shutdown_grace: Duration::from_millis(500),
        readiness: Readiness::Progress {
            quiet: Duration::from_millis(100),
        },
        ..ServerOptions::default()
    }
}

fn start(scenario: &str, options: &ServerOptions) -> (tempfile::TempDir, Result<Server, Error>) {
    let root = tempfile::tempdir().expect("tempdir");
    let server = Server::start(&fake(scenario), root.path(), options);
    (root, server)
}

/// Whether a process with this pid exists.
fn alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(process::Stdio::null())
        .status()
        .expect("run kill")
        .success()
}

fn clean_start_and_shutdown() {
    let started = Instant::now();
    let (_root, server) = start("clean", &options());
    let server = server.expect("start");
    // The fake reports 300 ms of indexing; readiness waits for its end.
    assert!(
        started.elapsed() >= Duration::from_millis(300),
        "{:?}",
        started.elapsed()
    );
    let pid = server.pid();
    assert!(alive(pid));
    assert_eq!(
        server.request("test/echo", json!({"x": 1}), LONG).unwrap(),
        json!({"x": 1})
    );
    let status = server.shutdown().expect("shutdown");
    assert!(status.success(), "{status:?}");
    assert!(!alive(pid));
}

fn initialize_sends_root_and_limited_capabilities() {
    let (root, server) = start("clean", &options());
    let server = server.expect("start");
    let params = server
        .request("test/initializeParams", Value::Null, LONG)
        .unwrap();
    let canonical = root.path().canonicalize().unwrap();
    let uri = file_uri(&canonical);
    assert_eq!(params["rootUri"], uri);
    assert_eq!(params["workspaceFolders"][0]["uri"], uri);
    assert_eq!(params["processId"], process::id());
    let caps = &params["capabilities"];
    assert!(caps["textDocument"]["definition"].is_object());
    assert!(caps["textDocument"]["references"].is_object());
    assert!(caps["textDocument"]["callHierarchy"].is_object());
    assert_eq!(caps["window"]["workDoneProgress"], true);
    for absent in ["codeAction", "formatting", "rename"] {
        assert!(caps["textDocument"].get(absent).is_none(), "{absent}");
    }
    assert!(caps.get("workspace").is_none());
    // The working directory is the root.
    assert_eq!(
        server.request("test/cwd", Value::Null, LONG).unwrap(),
        json!(canonical.to_string_lossy())
    );
}

fn startup_timeout_is_server_not_ready() {
    let timeout = Duration::from_millis(300);
    let started = Instant::now();
    let (_root, server) = start(
        "never-initializes",
        &ServerOptions {
            startup_timeout: timeout,
            ..options()
        },
    );
    match server {
        Err(Error::ServerNotReady { elapsed, stderr }) => {
            assert!(elapsed >= timeout, "{elapsed:?}");
            assert!(stderr.contains("fake-lsp: never-initializes"), "{stderr:?}");
        }
        other => panic!("expected ServerNotReady, got {other:?}"),
    }
    assert!(started.elapsed() < LONG, "{:?}", started.elapsed());
}

fn progress_that_never_ends_is_server_not_ready() {
    let (_root, server) = start(
        "endless-progress",
        &ServerOptions {
            startup_timeout: Duration::from_millis(400),
            ..options()
        },
    );
    assert!(
        matches!(server, Err(Error::ServerNotReady { .. })),
        "{server:?}"
    );
}

fn probe_retries_content_modified_until_ready() {
    let (_root, server) = start(
        "content-modified",
        &ServerOptions {
            readiness: Readiness::Probe {
                method: "test/probe".into(),
                params: Value::Null,
            },
            ..options()
        },
    );
    let server = server.expect("start");
    assert_eq!(
        server
            .request("test/probeCount", Value::Null, LONG)
            .unwrap(),
        json!(3)
    );
}

fn crash_during_startup_is_server_exited() {
    let (_root, server) = start("crash-on-initialize", &options());
    match server {
        Err(Error::ServerExited { status, stderr }) => {
            assert_eq!(status.code(), Some(3));
            assert!(stderr.contains("crashing during startup"), "{stderr:?}");
        }
        other => panic!("expected ServerExited, got {other:?}"),
    }
}

fn crash_after_startup_fails_later_requests() {
    let (_root, server) = start("crash-on-request", &options());
    let server = server.expect("start");
    for _ in 0..2 {
        match server.request("test/echo", Value::Null, LONG) {
            Err(Error::ServerExited { status, stderr }) => {
                assert_eq!(status.code(), Some(4));
                assert!(stderr.contains("crashing mid-request"), "{stderr:?}");
            }
            other => panic!("expected ServerExited, got {other:?}"),
        }
    }
}

fn missing_command_is_server_not_found() {
    let root = tempfile::tempdir().unwrap();
    let command = ServerCommand::new("codecite-no-such-language-server", ["--stdio"]);
    match Server::start(&command, root.path(), &options()) {
        Err(Error::ServerNotFound { command }) => {
            assert_eq!(command, "codecite-no-such-language-server");
        }
        other => panic!("expected ServerNotFound, got {other:?}"),
    }
}

fn ignored_shutdown_is_killed_after_the_grace_period() {
    let grace = Duration::from_millis(300);
    let (_root, server) = start(
        "ignores-shutdown",
        &ServerOptions {
            shutdown_grace: grace,
            ..options()
        },
    );
    let server = server.expect("start");
    let pid = server.pid();
    let started = Instant::now();
    let status = server.shutdown().expect("shutdown");
    let took = started.elapsed();
    assert!(!status.success(), "{status:?}");
    assert_eq!(status.code(), None, "killed by a signal");
    // `shutdown` gets no answer within the grace, then exit gets the grace.
    assert!(took >= grace * 2, "{took:?}");
    assert!(took < LONG, "{took:?}");
    assert!(!alive(pid));
}

fn dropping_the_handle_leaves_no_process() {
    let (_root, server) = start("clean", &options());
    let server = server.expect("start");
    let pid = server.pid();
    assert!(alive(pid));
    drop(server);
    assert!(!alive(pid));
}

fn dropping_a_stuck_server_leaves_no_process() {
    let (_root, server) = start("ignores-shutdown", &options());
    let server = server.expect("start");
    let pid = server.pid();
    // A panic unwinding past the handle drops it the same way.
    let unwound = catch_unwind(AssertUnwindSafe(move || {
        let _server = server;
        panic!("early exit");
    }));
    assert!(unwound.is_err());
    assert!(!alive(pid));
}

fn slow_reply_times_out_and_the_server_lives_on() {
    let (_root, server) = start("clean", &options());
    let server = server.expect("start");
    let timeout = Duration::from_millis(100);
    match server.request("test/sleep", json!({"ms": 500}), timeout) {
        Err(Error::Timeout { method, elapsed }) => {
            assert_eq!(method, "test/sleep");
            assert!(elapsed >= timeout);
        }
        other => panic!("expected Timeout, got {other:?}"),
    }
    assert_eq!(
        server.request("test/cancelled", Value::Null, LONG).unwrap(),
        json!([2])
    );
}

// ---------------------------------------------------------------------------
// The fake language server
// ---------------------------------------------------------------------------

mod fake {
    use super::*;

    struct Fake {
        input: BufReader<io::StdinLock<'static>>,
        output: io::StdoutLock<'static>,
        scenario: String,
        initialize_params: Value,
        probes: u64,
        cancelled: Vec<Value>,
    }

    /// Speaks scripted LSP on stdio, then exits; never returns.
    pub fn run(scenario: &str) -> ! {
        thread::spawn(|| {
            thread::sleep(FAKE_LIFETIME);
            process::exit(9);
        });
        eprintln!("fake-lsp: {scenario}");
        let mut fake = Fake {
            input: BufReader::new(io::stdin().lock()),
            output: io::stdout().lock(),
            scenario: scenario.to_owned(),
            initialize_params: Value::Null,
            probes: 0,
            cancelled: Vec::new(),
        };
        loop {
            let Some(message) = fake.recv() else {
                // Input closed. A stubborn server keeps running regardless.
                if fake.scenario == "ignores-shutdown" {
                    thread::sleep(FAKE_LIFETIME);
                }
                process::exit(1);
            };
            fake.handle(message);
        }
    }

    impl Fake {
        fn recv(&mut self) -> Option<Message> {
            let value = read_message(&mut self.input, DEFAULT_MAX_BODY_BYTES).ok()??;
            Message::from_value(value).ok()
        }

        fn send(&mut self, message: Message) {
            if write_message(&mut self.output, &message.to_value()).is_err() {
                process::exit(1);
            }
        }

        fn notify(&mut self, method: &str, params: Value) {
            self.send(Message::Notification {
                method: method.into(),
                params,
            });
        }

        fn progress(&mut self, kind: &str) {
            self.notify(
                "$/progress",
                json!({"token": "indexing", "value": {"kind": kind, "title": "Indexing"}}),
            );
        }

        fn handle(&mut self, message: Message) {
            match message {
                Message::Request { id, method, params } => {
                    if let Some(outcome) = self.answer(&method, params) {
                        self.send(Message::Response {
                            id: Some(id),
                            outcome,
                        });
                    }
                }
                Message::Notification { method, params } => self.on_notification(&method, params),
                Message::Response { .. } => {}
            }
        }

        fn on_notification(&mut self, method: &str, params: Value) {
            match (method, self.scenario.as_str()) {
                ("initialized", "clean" | "crash-on-request" | "ignores-shutdown") => {
                    // Ask to create a token, as real servers do, and wait
                    // for the reply before reporting progress on it.
                    self.send(Message::Request {
                        id: codecite_lsp::message::Id::String("create".into()),
                        method: "window/workDoneProgress/create".into(),
                        params: json!({"token": "indexing"}),
                    });
                    match self.recv() {
                        Some(Message::Response { outcome: Ok(_), .. }) => {}
                        other => {
                            eprintln!("fake-lsp: bad reply to create: {other:?}");
                            process::exit(5);
                        }
                    }
                    self.progress("begin");
                    thread::sleep(Duration::from_millis(300));
                    self.progress("end");
                }
                ("initialized", "endless-progress") => self.progress("begin"),
                ("exit", "ignores-shutdown") => {}
                ("exit", _) => process::exit(0),
                ("$/cancelRequest", _) => self.cancelled.push(params["id"].clone()),
                _ => {}
            }
        }

        fn answer(&mut self, method: &str, params: Value) -> Option<Result<Value, ResponseError>> {
            let scenario = self.scenario.clone();
            Some(match (method, scenario.as_str()) {
                ("initialize", "never-initializes") => return None,
                ("initialize", "crash-on-initialize") => {
                    eprintln!("fake-lsp: crashing during startup");
                    let _ = io::stderr().flush();
                    process::exit(3);
                }
                ("initialize", _) => {
                    self.initialize_params = params;
                    Ok(json!({"capabilities": {"definitionProvider": true}}))
                }
                ("shutdown", "ignores-shutdown") => return None,
                ("shutdown", _) => Ok(Value::Null),
                (_, "crash-on-request") => {
                    eprintln!("fake-lsp: crashing mid-request");
                    let _ = io::stderr().flush();
                    process::exit(4);
                }
                ("test/echo", _) => Ok(params),
                ("test/initializeParams", _) => Ok(self.initialize_params.clone()),
                ("test/cwd", _) => Ok(json!(std::env::current_dir().unwrap().to_string_lossy())),
                ("test/sleep", _) => {
                    thread::sleep(Duration::from_millis(params["ms"].as_u64().unwrap_or(0)));
                    Ok(Value::Null)
                }
                ("test/cancelled", _) => Ok(Value::Array(self.cancelled.clone())),
                ("test/probe", _) => {
                    self.probes += 1;
                    if self.probes < 3 {
                        Err(ResponseError {
                            code: CONTENT_MODIFIED,
                            message: "still indexing".into(),
                            data: None,
                        })
                    } else {
                        Ok(Value::Null)
                    }
                }
                ("test/probeCount", _) => Ok(json!(self.probes)),
                _ => Err(ResponseError {
                    code: METHOD_NOT_FOUND,
                    message: format!("fake-lsp does not handle {method}"),
                    data: None,
                }),
            })
        }
    }
}
