//! Command-line behaviour of the `codecite` binary.

use std::process::{Command, Output};

/// Runs the built binary with colour off, so assertions see plain text even
/// when the caller's shell sets `CLICOLOR_FORCE`.
fn codecite(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_codecite"))
        .args(args)
        .env_remove("CLICOLOR_FORCE")
        .env("NO_COLOR", "1")
        .output()
        .expect("failed to run codecite")
}

#[test]
fn version_flag_prints_package_version() {
    let out = codecite(&["--version"]);
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        format!("codecite {}\n", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn short_version_flag_matches_long() {
    assert_eq!(codecite(&["-V"]).stdout, codecite(&["--version"]).stdout);
}

#[test]
fn no_args_prints_usage_and_fails() {
    let out = codecite(&[]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("Usage: codecite"));
}

#[test]
fn unknown_flag_is_rejected() {
    let out = codecite(&["--nope"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("--nope"));
}

#[test]
fn search_help_has_examples_and_output() {
    let out = codecite(&["search", "--help"]);
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("Examples:"), "{text}");
    assert!(text.contains("Output:"), "{text}");
}
