//! `codecite mcp` end to end: the real binary serving a fixture corpus under
//! a temporary `XDG_DATA_HOME`, driven by JSON-RPC lines on its stdin.
//! Linux only, since other platforms ignore `XDG_DATA_HOME`.
#![cfg(target_os = "linux")]

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

use codecite_core::{SourceRecord, Store};
use serde_json::{Value, json};

/// A data home holding one corpus, `demo`, with one source.
fn data_home() -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    let dir = home.path().join("codecite");
    std::fs::create_dir_all(&dir).unwrap();
    let mut store = Store::open_read_write(&dir.join("demo.db")).unwrap();
    store
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
    store.set_indexed_at(30).unwrap();
    home
}

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
    assert_eq!(listed["corpora"][0]["sources"], 1);
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

#[test]
fn mcp_answers_on_stdout_and_logs_on_stderr() {
    let home = data_home();
    let mut child = codecite()
        .arg("mcp")
        .env("XDG_DATA_HOME", home.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    // Sends one message; for a request, reads lines until its response.
    let mut send = |message: Value| -> Option<Value> {
        writeln!(stdin, "{message}").unwrap();
        stdin.flush().unwrap();
        let id = message.get("id")?;
        loop {
            let mut line = String::new();
            assert_ne!(stdout.read_line(&mut line).unwrap(), 0, "stdout closed");
            // Every line on stdout is a JSON-RPC message.
            let reply: Value = serde_json::from_str(&line)
                .unwrap_or_else(|e| panic!("not JSON-RPC: {line:?}: {e}"));
            assert_eq!(reply["jsonrpc"], "2.0", "{reply}");
            if reply.get("id") == Some(id) {
                return Some(reply);
            }
        }
    };

    let init = send(json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "test", "version": "0"}
        }
    }))
    .unwrap();
    assert_eq!(init["result"]["serverInfo"]["name"], "codecite", "{init}");
    assert!(
        init["result"]["capabilities"]["tools"].is_object(),
        "{init}"
    );
    send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));

    let list = send(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})).unwrap();
    let tools = list["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 1, "{list}");
    assert_eq!(tools[0]["name"], "corpora");
    assert_eq!(tools[0]["inputSchema"]["type"], "object");

    let call = send(json!({
        "jsonrpc": "2.0", "id": 3, "method": "tools/call",
        "params": {"name": "corpora", "arguments": {}}
    }))
    .unwrap();
    assert_eq!(call["result"]["isError"], false, "{call}");
    let text = call["result"]["content"][0]["text"].as_str().unwrap();

    // The CLI prints the same JSON, byte for byte.
    let cli = codecite()
        .arg("corpora")
        .env("XDG_DATA_HOME", home.path())
        .output()
        .unwrap();
    assert_eq!(String::from_utf8(cli.stdout).unwrap(), format!("{text}\n"));

    // Closing stdin ends the server cleanly.
    drop(stdin);
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "{out:?}");
    let mut rest = String::new();
    stdout.read_line(&mut rest).unwrap();
    assert_eq!(rest, "", "stdout after the last response");
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(
        stderr.contains("serving the corpora in"),
        "stderr: {stderr}"
    );
}
