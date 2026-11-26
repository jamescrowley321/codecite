//! The Claude Code plugin under `plugins/codecite` and the marketplace
//! manifest that lists it.

use std::path::PathBuf;
use std::process::{Command, Stdio};

use serde_json::Value;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn text(path: &str) -> String {
    std::fs::read_to_string(root().join(path)).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn json(path: &str) -> Value {
    serde_json::from_str(&text(path)).unwrap_or_else(|e| panic!("{path}: {e}"))
}

#[test]
fn plugin_version_matches_the_binary() {
    assert_eq!(
        json("plugins/codecite/.claude-plugin/plugin.json")["version"],
        env!("CARGO_PKG_VERSION")
    );
}

#[test]
fn marketplace_points_at_the_plugin() {
    let m = json(".claude-plugin/marketplace.json");
    assert_eq!(m["plugins"][0]["name"], "codecite");
    assert!(
        root()
            .join(m["plugins"][0]["source"].as_str().unwrap())
            .join(".claude-plugin/plugin.json")
            .is_file()
    );
}

#[test]
fn mcp_runs_codecite_mcp() {
    let s = &json("plugins/codecite/.mcp.json")["mcpServers"]["codecite"];
    assert_eq!(
        (s["command"].as_str(), s["args"][0].as_str()),
        (Some("codecite"), Some("mcp"))
    );
}

#[test]
fn session_start_hook_is_the_subcommand_and_parses() {
    let cmd = json("plugins/codecite/hooks/hooks.json")["hooks"]["SessionStart"][0]["hooks"][0]
        ["command"]
        .clone();
    assert_eq!(cmd, "codecite hook claude session-start");
    // The binary accepts it: empty stdin, empty data directory, exit 0.
    let data = tempfile::tempdir().unwrap();
    let status = Command::new(env!("CARGO_BIN_EXE_codecite"))
        .args(["hook", "claude", "session-start"])
        .env("XDG_DATA_HOME", data.path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "{status}");
}

#[test]
fn tool_hooks_are_the_subcommands_and_parse() {
    let hooks = &json("plugins/codecite/hooks/hooks.json")["hooks"];
    for (event, matcher, command) in [
        (
            "PostToolUse",
            "Grep|Glob|Bash",
            "codecite hook claude post-tool-use",
        ),
        (
            "PreToolUse",
            "Edit|Write|MultiEdit",
            "codecite hook claude pre-tool-use",
        ),
    ] {
        let entry = &hooks[event][0];
        assert_eq!(entry["matcher"], matcher, "{event}");
        assert_eq!(entry["hooks"][0]["command"], command, "{event}");
        let args: Vec<&str> = command.split(' ').skip(1).collect();
        let data = tempfile::tempdir().unwrap();
        let status = Command::new(env!("CARGO_BIN_EXE_codecite"))
            .args(&args)
            .env("XDG_DATA_HOME", data.path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "{event}: {status}");
    }
}

#[test]
fn skill_is_short_and_named() {
    let s = text("plugins/codecite/skills/codecite/SKILL.md");
    assert!(s.starts_with("---\nname: codecite\ndescription: "));
    assert!(s.lines().count() <= 70);
}
