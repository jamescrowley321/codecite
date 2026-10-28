//! `codecite mcp` end to end: the real binary serving a fixture corpus under
//! a temporary `XDG_DATA_HOME`, driven by JSON-RPC lines on its stdin.
//! Linux only, since other platforms ignore `XDG_DATA_HOME`.
#![cfg(target_os = "linux")]

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use codecite_core::{
    ChunkRecord, DocumentRecord, Keyword, MAX_READ_BYTES, MAX_SEARCH_LIMIT, MAX_SNIPPET_BYTES,
    SourceRecord, Store, SymbolRecord,
};
use serde_json::{Value, json};

const LIB: &str = "pub fn validate(token: &str) -> bool {\n    !token.is_empty()\n}\n";
const SPEC: &str = "4.3.  Checking DPoP Proofs\n\nThe server MUST validate the proof.";

/// A data home holding one corpus, `demo`, with a code source `app` and a
/// spec, `rfc9449`, at two versions: the older one says SHOULD where the
/// current one says MUST.
fn data_home() -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    let dir = home.path().join("codecite");
    std::fs::create_dir_all(&dir).unwrap();
    let mut store = Store::open_read_write(&dir.join("demo.db")).unwrap();
    let chunk = ChunkRecord {
        kind: "function",
        start_line: 1,
        end_line: 3,
        text: LIB.trim_end(),
        number: None,
        anchor: None,
        title: None,
        parent: None,
        strength: &[],
        symbol: Some(SymbolRecord {
            id: "codecite . app . src/`lib.rs`/validate().".into(),
            display_name: "validate",
            kind: "function",
        }),
    };
    let source = SourceRecord {
        name: "app",
        kind: "local",
        url: "/src/app",
        revision: "0123456789abcdef0123456789abcdef01234567",
        license_spdx: Some("MIT"),
        license_class: "adapt",
        is_current: true,
        obsoleted_by: None,
        updated_by: None,
    };
    let app = store.upsert_source(&source).unwrap();
    let code = DocumentRecord {
        path: "src/lib.rs",
        language: Some("rust"),
        content_hash: b"lib",
        size_bytes: 1,
        dirty: false,
        text: Some(LIB),
        chunks: vec![chunk.clone()],
    };
    store.write_document(app, &code).unwrap();
    let rfc = store
        .upsert_source(&SourceRecord {
            name: "rfc9449",
            kind: "document",
            url: "https://www.rfc-editor.org/rfc/rfc9449.html",
            revision: "1a2b3c4d5e6f00000000000000000000000000000000000000000000000000ff",
            license_spdx: None,
            license_class: "quote",
            ..source
        })
        .unwrap();
    let section = ChunkRecord {
        kind: "section",
        text: SPEC,
        number: Some("4.3"),
        anchor: Some("section-4.3"),
        title: Some("Checking DPoP Proofs"),
        strength: &[Keyword::Must],
        symbol: None,
        ..chunk
    };
    let spec = DocumentRecord {
        path: "rfc9449",
        language: None,
        content_hash: b"rfc9449",
        text: None,
        chunks: vec![section],
        ..code
    };
    store.write_document(rfc, &spec).unwrap();
    let older = store
        .upsert_source(&SourceRecord {
            name: "rfc9449",
            kind: "document",
            url: "https://www.rfc-editor.org/rfc/rfc9449-draft.html",
            revision: OLD_RFC9449,
            license_spdx: None,
            license_class: "quote",
            is_current: false,
            ..source
        })
        .unwrap();
    let old_section = ChunkRecord {
        text: OLD_SPEC,
        strength: &[Keyword::Should],
        ..spec.chunks[0].clone()
    };
    let old_spec = DocumentRecord {
        content_hash: b"rfc9449-old",
        chunks: vec![old_section],
        ..spec
    };
    store.write_document(older, &old_spec).unwrap();
    store.set_indexed_at(30).unwrap();
    home
}

const OLD_RFC9449: &str = "01d0000000000000000000000000000000000000000000000000000000009449";
const OLD_SPEC: &str = "4.3.  Checking DPoP Proofs\n\nThe server SHOULD validate the proof.";

fn codecite() -> Command {
    Command::new(env!("CARGO_BIN_EXE_codecite"))
}

#[test]
fn corpora_prints_the_corpora_as_json() {
    let home = data_home();
    let out = codecite()
        .arg("corpora")
        .env("XDG_DATA_HOME", home.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let listed: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(listed["corpora"][0]["corpus"], "demo");
    assert_eq!(listed["corpora"][0]["sources"], 3);
    assert_eq!(listed["corpora"][0]["stale"], false);
    assert_eq!(listed["corpora"].as_array().unwrap().len(), 1);

    let empty = tempfile::tempdir().unwrap();
    let out = codecite()
        .arg("corpora")
        .env("XDG_DATA_HOME", empty.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    assert_eq!(out.stdout, b"{\"corpora\":[]}\n");
}

/// A running `codecite mcp` over the data home `home`.
struct Session {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Session {
    fn start(home: &Path) -> Self {
        let mut child = codecite()
            .arg("mcp")
            .env("XDG_DATA_HOME", home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Session {
            child,
            stdin,
            stdout,
        }
    }

    /// Sends one message; for a request, reads lines until its response.
    fn send(&mut self, message: Value) -> Option<Value> {
        writeln!(self.stdin, "{message}").unwrap();
        self.stdin.flush().unwrap();
        let id = message.get("id")?;
        loop {
            let mut line = String::new();
            assert_ne!(
                self.stdout.read_line(&mut line).unwrap(),
                0,
                "stdout closed"
            );
            // Every line on stdout is a JSON-RPC message.
            let reply: Value = serde_json::from_str(&line)
                .unwrap_or_else(|e| panic!("not JSON-RPC: {line:?}: {e}"));
            assert_eq!(reply["jsonrpc"], "2.0", "{reply}");
            if reply.get("id") == Some(id) {
                return Some(reply);
            }
        }
    }

    /// Initializes the session, as a client does first.
    fn initialize(&mut self) -> Value {
        let init = self
            .send(json!({
                "jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": {
                    "protocolVersion": "2025-06-18",
                    "capabilities": {},
                    "clientInfo": {"name": "test", "version": "0"}
                }
            }))
            .unwrap();
        self.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        init
    }

    /// Calls `tool`, and asserts that its text is the structured result
    /// and what the CLI prints for `args`, byte for byte.
    fn call_matches_cli(
        &mut self,
        home: &Path,
        id: u64,
        tool: &str,
        arguments: Value,
        args: &[&str],
    ) {
        let call = self
            .send(json!({
                "jsonrpc": "2.0", "id": id, "method": "tools/call",
                "params": {"name": tool, "arguments": arguments}
            }))
            .unwrap();
        assert_eq!(call["result"]["isError"], false, "{call}");
        let text = call["result"]["content"][0]["text"].as_str().unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(text).unwrap(),
            call["result"]["structuredContent"],
            "{call}"
        );
        let cli = codecite()
            .args(args)
            .env("XDG_DATA_HOME", home)
            .output()
            .unwrap();
        assert!(cli.status.success(), "{args:?}: {cli:?}");
        assert_eq!(
            String::from_utf8(cli.stdout).unwrap(),
            format!("{text}\n"),
            "{args:?}"
        );
    }

    /// Closes stdin, which ends the server, and returns its output and
    /// anything left on stdout.
    fn close(self) -> (std::process::Output, String) {
        let Session {
            child,
            stdin,
            mut stdout,
        } = self;
        drop(stdin);
        let out = child.wait_with_output().unwrap();
        let mut rest = String::new();
        stdout.read_line(&mut rest).unwrap();
        (out, rest)
    }
}

#[test]
fn mcp_answers_on_stdout_and_logs_on_stderr() {
    let home = data_home();
    let mut session = Session::start(home.path());
    let init = session.initialize();
    assert_eq!(init["result"]["serverInfo"]["name"], "codecite", "{init}");
    assert!(
        init["result"]["capabilities"]["tools"].is_object(),
        "{init}"
    );

    let list = session
        .send(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}))
        .unwrap();
    let tools = list["result"]["tools"].as_array().unwrap();
    let mut names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    names.sort_unstable();
    assert_eq!(
        names,
        ["corpora", "definition", "diff", "grep", "read", "search"],
        "{list}"
    );
    for tool in tools {
        assert_eq!(tool["inputSchema"]["type"], "object", "{tool}");
    }

    // Each tool's text is what the CLI prints for the same query, byte for
    // byte: corpora, a search, greps, a read of code and of a spec, an
    // empty search and a definition.
    let cases = [
        ("corpora", json!({}), vec!["corpora"]),
        (
            "search",
            json!({"query": "validate", "corpus": "demo"}),
            vec!["search", "demo", "validate", "--json"],
        ),
        (
            "search",
            json!({"query": "proof", "limit": 1}),
            vec!["search", "demo", "proof", "--limit", "1", "--json"],
        ),
        (
            "search",
            json!({"query": "zzzz"}),
            vec!["search", "demo", "zzzz", "--json"],
        ),
        (
            "grep",
            json!({"pattern": "valid", "limit": 2}),
            vec!["grep", "demo", "valid", "--limit", "2", "--json"],
        ),
        (
            "grep",
            json!({"pattern": "MUST", "fixed": true, "only": "spec"}),
            vec![
                "grep", "demo", "MUST", "--fixed", "--only", "spec", "--json",
            ],
        ),
        (
            "read",
            json!({"citation": "RFC 9449 §4.3"}),
            vec!["read", "demo", "RFC 9449 §4.3", "--json"],
        ),
        (
            "read",
            json!({"citation": "app@0123456:src/lib.rs#L1-L3", "max_lines": 2}),
            vec![
                "read",
                "demo",
                "app@0123456:src/lib.rs#L1-L3",
                "--max-lines",
                "2",
                "--json",
            ],
        ),
        (
            "definition",
            json!({"symbol": "validate"}),
            vec!["def", "demo", "validate", "--json"],
        ),
        (
            "diff",
            json!({"document": "rfc9449", "section": "4.3"}),
            vec!["diff", "demo", "rfc9449", "--section", "4.3", "--json"],
        ),
    ];
    for (id, (tool, arguments, args)) in (3..).zip(cases) {
        session.call_matches_cli(home.path(), id, tool, arguments, &args);
    }

    // A typed error is a tool error, with the message the CLI prints.
    let call = session
        .send(json!({
            "jsonrpc": "2.0", "id": 99, "method": "tools/call",
            "params": {"name": "read", "arguments": {"citation": "RFC 9449 §4.9"}}
        }))
        .unwrap();
    assert_eq!(call["result"]["isError"], true, "{call}");
    let error = &call["result"]["structuredContent"]["error"];
    assert_eq!(error["kind"], "no_such_section", "{call}");
    assert_eq!(
        error["candidates"],
        json!(["rfc9449@sha256:1a2b3c4d5e6f#section-4.3"])
    );
    let cli = codecite()
        .args(["read", "demo", "RFC 9449 §4.9"])
        .env("XDG_DATA_HOME", home.path())
        .output()
        .unwrap();
    assert!(!cli.status.success());
    let stderr = String::from_utf8(cli.stderr).unwrap();
    assert!(
        stderr.contains(error["message"].as_str().unwrap()),
        "{stderr}"
    );

    // Closing stdin ends the server cleanly.
    let (out, rest) = session.close();
    assert!(out.status.success(), "{out:?}");
    assert_eq!(rest, "", "stdout after the last response");
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(
        stderr.contains("serving the corpora in"),
        "stderr: {stderr}"
    );
}

#[test]
fn mcp_cuts_a_mebibyte_line_as_the_cli_does() {
    // A data home whose one corpus holds a file whose only line is 1 MiB.
    let mut line = "token ".repeat(1024 * 1024 / 6 + 1);
    line.truncate(1024 * 1024);
    let home = tempfile::tempdir().unwrap();
    let dir = home.path().join("codecite");
    std::fs::create_dir_all(&dir).unwrap();
    let mut store = Store::open_read_write(&dir.join("big.db")).unwrap();
    let app = store
        .upsert_source(&SourceRecord {
            name: "app",
            kind: "local",
            url: "/src/app",
            revision: "0123456789abcdef0123456789abcdef01234567",
            license_spdx: Some("MIT"),
            license_class: "adapt",
            is_current: true,
            obsoleted_by: None,
            updated_by: None,
        })
        .unwrap();
    let chunk = ChunkRecord {
        kind: "window",
        start_line: 1,
        end_line: 1,
        text: &line,
        number: None,
        anchor: None,
        title: None,
        parent: None,
        strength: &[],
        symbol: None,
    };
    let bundle = DocumentRecord {
        path: "dist/bundle.txt",
        language: None,
        content_hash: b"bundle",
        size_bytes: 1024 * 1024,
        dirty: false,
        text: Some(&line),
        chunks: vec![chunk],
    };
    store.write_document(app, &bundle).unwrap();
    store.set_indexed_at(30).unwrap();
    drop(store);

    let mut session = Session::start(home.path());
    session.initialize();
    let limit = MAX_SEARCH_LIMIT.to_string();
    let citation = "app@0123456:dist/bundle.txt#L1";
    session.call_matches_cli(
        home.path(),
        2,
        "search",
        json!({"query": "token", "limit": MAX_SEARCH_LIMIT}),
        &["search", "big", "token", "--limit", &limit, "--json"],
    );
    session.call_matches_cli(
        home.path(),
        3,
        "read",
        json!({"citation": citation, "max_lines": 1}),
        &["read", "big", citation, "--max-lines", "1", "--json"],
    );
    // And what they return is cut to the budgets.
    let search = session
        .send(json!({
            "jsonrpc": "2.0", "id": 4, "method": "tools/call",
            "params": {"name": "search", "arguments": {"query": "token"}}
        }))
        .unwrap();
    let top = &search["result"]["structuredContent"]["results"][0];
    assert_eq!(top["snippet"].as_str().unwrap().len(), MAX_SNIPPET_BYTES);
    assert_eq!(top["snippet_truncated"], true);
    let read = session
        .send(json!({
            "jsonrpc": "2.0", "id": 5, "method": "tools/call",
            "params": {"name": "read", "arguments": {"citation": citation}}
        }))
        .unwrap();
    let read = &read["result"]["structuredContent"];
    assert_eq!(read["text"].as_str().unwrap().len(), MAX_READ_BYTES);
    assert_eq!(read["truncated"], true);
    let (out, _) = session.close();
    assert!(out.status.success(), "{out:?}");
}
