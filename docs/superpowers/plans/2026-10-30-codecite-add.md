# codecite add Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `codecite add <corpus> <url>...` adds git repositories to a corpus manifest, each pinned at its remote's current commit with its license detected, so a corpus of many repositories takes a few commands (#184).

**Architecture:** Three pieces in `codecite-index`, and a thin CLI command over them.

1. **Pin.** `remote_head(url)` in `clone.rs` runs `git ls-remote -- <url> HEAD` and returns the 40-hex commit the remote's default branch points at.
2. **Detect.** `license.rs` builds an `askalono::Store` at first use from license texts embedded with `include_str!` (SPDX's texts of the common licenses), scans the checkout's top-level `LICENSE*`, `LICENCE*`, `COPYING*` and `UNLICENSE` files, and returns the SPDX id of each file that scores at least 0.9, plus the license class the design's table gives it (permissive → `adapt`; copyleft or nothing found → `approach_only`).
3. **Write.** `add.rs` ties them together: for each URL not already in the manifest, pin it, check it out through the shared clone cache (`Clones::checkout`, so the next `codecite index` finds the clone and fetches nothing), detect its license, and append one `[[source]]` table to the manifest's text. The new text is parsed with `Manifest::parse` before it replaces the file through a temporary file and a rename, so a failure on any URL, or a manifest that would not parse, writes nothing. Appending text, rather than re-serializing, keeps every comment and the author's layout.

**Tech Stack:** Rust, `askalono` 0.5 (default features off), `git` CLI, `toml` for quoting strings, `tempfile` for the atomic write.

## Global Constraints

- Done when (issue #184): a corpus of every repository in the history benchmark is built with `codecite add` and indexes cleanly, with its index time and size recorded (`experiments/many-repos/`).
- `docs/design.md` names `askalono` for SPDX detection and says the manifest's `license` and `use` are a person's to confirm: `add` writes them as a proposal and its output says to check them.
- No change to indexing, search, ranking or existing command output. The history benchmark is still run (the CLI help gains a command) and its results committed; any repository's pooled `all` row for `codecite` lower on Recall@1, Recall@10 or MRR than `scorecard.md` on `origin/feat/one-ranking` is a loss.
- Before every push: `cargo fmt --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked && cargo deny check`.
- The owner is learning Rust: the PR body explains the idioms marked **Learn:** below.

## Decisions taken here (DEC-010 in the loop's journal)

- **License texts are embedded, not downloaded.** `askalono` ships no license data. A store built from 14 SPDX texts (MIT, Apache-2.0, BSD-2-Clause, BSD-3-Clause, ISC, 0BSD, Unlicense, Zlib, MPL-2.0, LGPL-2.1-only, LGPL-3.0-only, GPL-2.0-only, GPL-3.0-only, AGPL-3.0-only) costs about 200 KiB in the binary and works offline. A license outside the list is "not detected" and gets `approach_only`, which the design's table already says for "unknown".
- **Several license files:** the distinct ids are joined with ` AND ` in sorted order and `use` is the strictest class among them. `AND` claims every file's terms, which is the safe reading when codecite cannot tell `AND` from `OR`; the person edits it to `OR` where the project dual-licenses.
- **A URL already in the manifest is skipped**, reported and left at its pin. Re-pinning is a separate operation.
- **`name` is not written.** The index pipeline already derives a git source's name from the URL's last segment.
- **Missing manifest:** `add` creates it with `name = "<corpus>"`. An existing manifest for another corpus is an error, as `index` treats it.

## Files

- Create `crates/codecite-index/src/license.rs` — `detect_license(dir) -> DetectedLicense`.
- Create `crates/codecite-index/licenses/*.txt` and `crates/codecite-index/licenses/README.md` (where the texts come from).
- Create `crates/codecite-index/src/add.rs` — `add_sources(path, corpus, urls, clones) -> Result<Vec<Added>>`.
- Modify `crates/codecite-index/src/clone.rs` — `remote_head(url)`.
- Modify `crates/codecite-index/src/lib.rs`, `crates/codecite-index/src/error.rs`, `crates/codecite-index/Cargo.toml`, `Cargo.toml`, `Cargo.lock`.
- Modify `crates/codecite/src/main.rs` — `Command::Add`, `fn add`.
- Create `crates/codecite/tests/add.rs`.
- Modify `docs/usage.md` (new "Add repositories" section before "Index"), `docs/design.md` (Interfaces row for `add`, the licensing bullet, the `license` "optional until SPDX detection lands" line).
- Create `experiments/many-repos/` (`build.sh`, `history-all.toml`, `results.md`).
- Update `evals/history/results/`.

---

### Task 1: Pin a remote's current commit

**Files:** `crates/codecite-index/src/clone.rs`, `crates/codecite-index/src/lib.rs`

- [ ] **Step 1: Failing test.** In `clone.rs`'s tests, using `test_repo::{Repo, Remote}`: commit a file, push to a bare remote, and assert `remote_head(&remote.url())` equals the pushed commit. A second test: `remote_head("file:///nonexistent")` is `Err(CloneError::...)` naming the URL; a third: a remote with no commits is `Err(CloneError::NoHead { url })`.
- [ ] **Step 2: Run it** — `cargo test -p codecite-index remote_head` — fails to compile.
- [ ] **Step 3: Implement.**

```rust
/// The commit `url`'s default branch points at, as `git ls-remote` reports
/// it: what `codecite add` pins.
pub fn remote_head(url: &str) -> Result<String, CloneError> {
    // `--` ends the options, so a URL starting with `-` is never one.
    let output = Command::new("git")
        .args(["ls-remote", "--", url, "HEAD"])
        .env("GIT_TERMINAL_PROMPT", "0")
        .env_remove("GIT_DIR")
        .output()
        .map_err(|source| CloneError::GitSpawn { source })?;
    // non-zero exit → the existing git-failure variant with stderr;
    // first field of the first line, checked with is_lower_hex(_, 40),
    // else CloneError::NoHead { url }.
}
```

  Reuse the existing failure variant `run` returns for a failed git command (read `clone.rs` for its name and fields) rather than adding a second one; add only `NoHead`. Export `remote_head` from `lib.rs`.

  **Learn:** `Command::output()` captures stdout and stderr as `Vec<u8>`; `String::from_utf8_lossy` reads them without failing on bad bytes.
- [ ] **Step 4: Run** the three tests; they pass.
- [ ] **Step 5: Commit** `feat(index): pin a remote's current commit (#184)`.

### Task 2: Detect a checkout's license

**Files:** `crates/codecite-index/src/license.rs`, `crates/codecite-index/licenses/`, `crates/codecite-index/Cargo.toml`, `Cargo.toml`, `crates/codecite-index/src/lib.rs`

- [ ] **Step 1: Fetch the texts.** From `https://raw.githubusercontent.com/spdx/license-list-data/main/text/<id>.txt` for the 14 ids in Decisions, into `crates/codecite-index/licenses/<id>.txt`. Write `licenses/README.md`: the source URL, the license-list-data tag, and that these are the licenses' own texts, used only to recognize them.
- [ ] **Step 2: Dependency.** Workspace: `askalono = { version = "0.5", default-features = false }`; `codecite-index`: `askalono.workspace = true`. Run `cargo deny check`; every new crate's license must already be allowed (expect MIT/Apache-2.0). If one isn't, stop and park with the crate and its license.
- [ ] **Step 3: Failing tests** in `license.rs`:
  - `fs::write(dir/"LICENSE", <MIT text with "Copyright (c) 2026 Someone" filled in>)` → `DetectedLicense { spdx: Some("MIT"), class: Adapt }`.
  - `LICENSE` with GPL-3.0 text → `Some("GPL-3.0-only")`, `ApproachOnly`.
  - `LICENSE-MIT` and `LICENSE-APACHE` → `Some("Apache-2.0 AND MIT")`, `Adapt`.
  - `LICENSE-MIT` and `COPYING` (GPL-3.0) → `Some("GPL-3.0-only AND MIT")`, `ApproachOnly`.
  - No license file, or a `LICENSE` of unrelated prose → `spdx: None`, `ApproachOnly`.
  - A `LICENSE` directory, or a `docs/LICENSE` only → `None` (top level, regular files only).
- [ ] **Step 4: Implement.**

```rust
//! License detection for `codecite add`: the SPDX id of each license file
//! at a checkout's top level, matched with askalono against the texts
//! in `licenses/`, and the license class the design's table gives it.

/// What `add` proposes for a source's `license` and `use`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectedLicense {
    /// The detected ids joined with ` AND `, or `None` when no file matched.
    pub spdx: Option<String>,
    /// The strictest class among them; `approach_only` when none matched.
    pub class: LicenseClass,
}

/// The texts askalono matches against: (SPDX id, text, class).
const LICENSES: &[(&str, &str, LicenseClass)] = &[
    ("MIT", include_str!("../licenses/MIT.txt"), LicenseClass::Adapt),
    // … the other 13
];

/// Below this score a file is not that license.
const MIN_SCORE: f32 = 0.9;

fn store() -> &'static askalono::Store {
    static STORE: OnceLock<askalono::Store> = OnceLock::new();
    STORE.get_or_init(|| {
        let mut store = askalono::Store::new();
        for (id, text, _) in LICENSES {
            store.add_license((*id).to_owned(), askalono::TextData::from(*text));
        }
        store
    })
}

pub fn detect_license(dir: &Path) -> io::Result<DetectedLicense> { … }
```

  File names: case-insensitive prefixes `license`, `licence`, `copying`, `unlicense`; regular files only (`symlink_metadata`, never follow a link out of the checkout), at most 1 MiB each, read lossily. Collect ids into a `BTreeSet` so the join is sorted and deduplicated. The strictest class: `ApproachOnly` beats `Adapt`.

  **Learn:** `include_str!` bakes a file into the binary at compile time, so the texts ship with it and need no install step. `OnceLock` builds the store once, on first use, safely across threads. `BTreeSet` keeps its items sorted, which makes the output deterministic.
- [ ] **Step 5: Run** `cargo test -p codecite-index license` — passes. If a real text scores under 0.9 (e.g. BSD variants confusing each other), print the scores and adjust only `MIN_SCORE`, documenting why.
- [ ] **Step 6: Commit** `feat(index): detect a checkout's license with askalono (#184)`.

### Task 3: Add sources to a manifest

**Files:** `crates/codecite-index/src/add.rs`, `crates/codecite-index/src/error.rs`, `crates/codecite-index/src/lib.rs`

- [ ] **Step 1: Failing tests** in `add.rs` using `test_repo` remotes (one with an MIT `LICENSE`, one with none) and a `Clones` under a temp dir:
  - Missing manifest → created; it parses; corpus name set; one git source per URL with the remote's head commit, `license = "MIT"`/`use = "adapt"` and no `license`/`use = "approach_only"` respectively. The clone exists at `clones.path(url)`.
  - Existing manifest with a comment and a document source → the comment and source are byte-for-byte intact, the new table appended after them.
  - A URL already present → `Added { skipped: true, .. }`, the file unchanged.
  - Two URLs, the second unreachable → `Err`, the file unchanged (and still absent when it was).
  - A manifest for another corpus → `Err(Error::WrongCorpus { .. })`.
  - The same URL twice in one call → added once.
- [ ] **Step 2: Implement.**

```rust
/// One URL `add_sources` handled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Added {
    pub url: String,
    /// The pinned commit; for a skipped URL, the manifest's pin.
    pub commit: String,
    pub license: DetectedLicense,   // for a skipped URL, what the manifest says
    /// Already in the manifest, so left as it was.
    pub skipped: bool,
}

pub fn add_sources(path: &Path, corpus: &str, urls: &[String], clones: &Clones)
    -> crate::Result<Vec<Added>>
```

  Steps inside: read the file (`ErrorKind::NotFound` → start from `name = "<corpus>"\n`); parse it and check `name`; for each URL not already a git source's `url`: `remote_head`, `clones.checkout(&GitSource { commit, .. })`, `detect_license(checkout.path())`, drop the checkout, then append

```toml

[[source]]
kind = "git"
url = "<url>"
commit = "<sha>"
license = "<spdx>"        # omitted when not detected
use = "<class>"
```

  Quote every value with `toml::Value::String(s).to_string()` so any URL is valid TOML. `Manifest::parse` the result, then write it with `tempfile::NamedTempFile::new_in(parent)` + `persist(path)`.

  **Learn:** `persist` is a rename within one directory, which is atomic: a reader sees the old manifest or the new one, never half of it.
- [ ] **Step 3: Run** `cargo test -p codecite-index add_` — passes.
- [ ] **Step 4: Commit** `feat(index): add git sources to a manifest, pinned and licensed (#184)`.

### Task 4: The `add` command

**Files:** `crates/codecite/src/main.rs`, `crates/codecite/tests/add.rs`

- [ ] **Step 1: Failing test** `crates/codecite/tests/add.rs` (copy the `git`, `codecite`, `stdout` helpers from `tests/index.rs`): a local repo with an MIT `LICENSE` pushed to a bare remote; `codecite add mywork <file-url>` in a work dir; assert stdout, the manifest's contents, then `codecite index mywork` succeeds and `codecite search mywork <word>` cites the pinned commit. A second test: `codecite add mywork` with no URL fails with clap's usage; an unreachable URL fails, naming it, and writes no manifest.
- [ ] **Step 2: Implement.**

```rust
    /// Add git repositories to a corpus's manifest, pinned at their current
    /// commit with their license detected.
    #[command(after_long_help = "Examples:
  codecite add mywork https://github.com/owner/repo
  codecite add mywork https://github.com/a/one https://github.com/b/two
  codecite add mywork https://github.com/owner/repo --manifest ~/m.toml")]
    Add {
        corpus: String,
        #[arg(required = true)]
        urls: Vec<String>,
        /// The manifest to write [default: corpora/<CORPUS>.toml].
        #[arg(long)]
        manifest: Option<PathBuf>,
    },
```

  `fn add` checks the corpus name, resolves the path as `index` does (create `corpora/` when it is the default and missing), builds `Clones::new(dirs.cache_dir().join("repos"))`, calls `add_sources`, and prints one line per URL:

```text
added https://github.com/owner/repo @ 3f2a9c1e0b7d · MIT · adapt
kept https://github.com/owner/other @ 9a8b7c6d5e4f · already in corpora/mywork.toml
check each license and use, then run: codecite index mywork
```

  (`no license detected` in place of the id.) Add `add` to the start-here block only if the block lists setup commands; it lists query commands today, so leave it.
- [ ] **Step 3: Run** `cargo test -p codecite --test add` — passes. Then exercise it by hand against a real public repository in a temp dir (`codecite add t https://github.com/jamescrowley321/blind-peer-review`, then `index`), and one adversarial URL (`-x`, and `https://example.invalid/repo`).
- [ ] **Step 4: Commit** `feat(cli): codecite add (#184)`.

### Task 5: Docs

**Files:** `docs/usage.md`, `docs/design.md`

- [ ] `usage.md`: an "Add repositories" section before "Index": the command, the output, that `license`/`use` are proposals to check, that a known URL is kept at its pin, and that nothing is written when any URL fails.
- [ ] `design.md`: the Interfaces row (`codecite add <corpus> <url>... [--manifest <path>]`, "Add git sources to a manifest, pinned at the remote's current commit, with license and use proposed from SPDX detection"); the "Detection plus confirmation" bullet says detection runs at `add` (the mismatch warning on index runs is still to come, so don't claim it); "`license` is optional until SPDX detection lands" → "`license` is optional; `codecite add` fills it from detection".
- [ ] Commit `docs: codecite add (#184)`.

### Task 6: A corpus of every history repository

**Files:** `experiments/many-repos/build.sh`, `experiments/many-repos/history-all.toml`, `experiments/many-repos/results.md`

- [ ] `build.sh`: with the release binary, in a temp work dir, `codecite add history-all` the five repositories' URLs from `evals/history/*/manifest.toml`, copy the manifest it wrote to `history-all.toml` here, then `/usr/bin/time -v codecite index history-all --manifest …`, recording wall time, peak RSS, the index summary and the `.db` size (`stat -c %s`).
- [ ] Run it (background it; it clones five repositories). Write `results.md`: the commands, each source's pin and detected license, elapsed, size, peak RSS, and the date. Commit `chore(experiments): a corpus of every history repository with codecite add (#184)`.

### Task 7: History benchmark and gates

- [ ] `cargo build --release --locked -p codecite -p codecite-bench`, then `evals/history/run.sh` (background it). Compare every repository's pooled `all` `codecite` row with `git show origin/feat/one-ranking:evals/history/results/scorecard.md`. Any drop is a loss → park with both rows.
- [ ] Commit `evals/history/results/`: `chore(evals): rerun the history benchmark (#184)`.
- [ ] Gates: `cargo fmt --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked && cargo deny check`.
