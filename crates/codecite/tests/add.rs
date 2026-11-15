//! `codecite add` end to end: a bare remote served over `file://`, the
//! manifest it writes, and an index run over that manifest. Linux only, since
//! other platforms ignore `XDG_DATA_HOME`.
#![cfg(target_os = "linux")]

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

const MIT: &str = include_str!("../../codecite-index/licenses/MIT.txt");

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=codecite",
            "-c",
            "user.email=codecite@example.com",
        ])
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?}");
    String::from_utf8(output.stdout)
        .unwrap()
        .trim_end()
        .to_owned()
}

/// Runs the binary in `cwd` with its data directory under `home`.
fn codecite(cwd: &Path, home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_codecite"))
        .current_dir(cwd)
        .env("HOME", home)
        .env("XDG_DATA_HOME", home.join("data"))
        .env("XDG_CACHE_HOME", home.join("cache"))
        .args(args)
        .output()
        .unwrap()
}

fn stdout(output: &Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout.clone()).unwrap()
}

/// A bare remote named `app` whose `main` holds an MIT `LICENSE` and one
/// source file; returns its URL and head commit.
fn remote(tmp: &Path) -> (String, String) {
    let repo = tmp.join("app");
    fs::create_dir_all(repo.join("src")).unwrap();
    git(&repo, &["init", "-q"]);
    fs::write(repo.join("LICENSE"), MIT).unwrap();
    fs::write(repo.join("src/lib.rs"), "pub fn validate_token() {}\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    let remote = tmp.join("remote/app.git");
    fs::create_dir_all(&remote).unwrap();
    git(&remote, &["init", "-q", "--bare"]);
    git(&remote, &["config", "uploadpack.allowFilter", "true"]);
    let url = format!("file://{}", remote.display());
    git(&repo, &["push", "-q", &url, "HEAD:refs/heads/main"]);
    git(&remote, &["symbolic-ref", "HEAD", "refs/heads/main"]);
    (url, git(&repo, &["rev-parse", "HEAD"]))
}

#[test]
fn add_pins_and_licenses_a_repository_that_index_then_cites() {
    let tmp = tempfile::tempdir().unwrap();
    let (url, commit) = remote(tmp.path());
    let work = tmp.path().join("work");
    fs::create_dir_all(&work).unwrap();

    let out = stdout(&codecite(&work, tmp.path(), &["add", "mywork", &url]));
    assert_eq!(
        out,
        format!(
            "added {url} @ {} · MIT · adapt\n\
             check each license and use, then run: codecite index mywork\n",
            &commit[..12]
        )
    );
    let manifest = fs::read_to_string(work.join("corpora/mywork.toml")).unwrap();
    assert_eq!(
        manifest,
        format!(
            "name = \"mywork\"\n\n[[source]]\nkind = \"git\"\nurl = \"{url}\"\n\
             commit = \"{commit}\"\nlicense = \"MIT\"\nuse = \"adapt\"\n"
        )
    );

    // Added again, the URL is kept at its pin and the file is untouched.
    let again = stdout(&codecite(&work, tmp.path(), &["add", "mywork", &url]));
    assert!(
        again.starts_with(&format!(
            "kept {url} @ {} · already in corpora/mywork.toml\n",
            &commit[..12]
        )),
        "{again}"
    );
    assert_eq!(
        fs::read_to_string(work.join("corpora/mywork.toml")).unwrap(),
        manifest
    );

    stdout(&codecite(&work, tmp.path(), &["index", "mywork"]));
    let hits = stdout(&codecite(
        &work,
        tmp.path(),
        &["search", "mywork", "validate_token"],
    ));
    assert!(
        hits.contains(&format!("app@{}:src/lib.rs", &commit[..12])),
        "{hits}"
    );
}

#[test]
fn add_errors_write_no_manifest() {
    let tmp = tempfile::tempdir().unwrap();

    let no_url = codecite(tmp.path(), tmp.path(), &["add", "mywork"]);
    assert!(!no_url.status.success());
    let stderr = String::from_utf8_lossy(&no_url.stderr);
    assert!(stderr.contains("Usage:"), "{stderr}");

    let gone = format!("file://{}", tmp.path().join("missing.git").display());
    let unreachable = codecite(tmp.path(), tmp.path(), &["add", "mywork", &gone]);
    assert!(!unreachable.status.success());
    let stderr = String::from_utf8_lossy(&unreachable.stderr);
    assert!(stderr.contains(&gone), "{stderr}");
    assert!(!tmp.path().join("corpora/mywork.toml").exists());

    let bad_name = codecite(tmp.path(), tmp.path(), &["add", "../x", &gone]);
    assert!(!bad_name.status.success());
}
