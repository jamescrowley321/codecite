# LICENSE and NOTICE Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement #6: add the Apache-2.0 `LICENSE` text and a `NOTICE` file at the repo root, and point to both from the README.

**Architecture:** Two plain-text files at the root, where GitHub, crates.io and license scanners look for them. `LICENSE` is the canonical Apache License 2.0 text, byte for byte. `NOTICE` holds only the project name and copyright line, as the Apache guidance asks. The workspace already declares `license = "Apache-2.0"` in `Cargo.toml`, so no manifest changes.

**Tech Stack:** Plain text; `sha256sum` to check the license text.

**Spec:** issue #6, `docs/design.md#decisions-and-open-questions` (License: Apache-2.0).

## Global Constraints

- `LICENSE` is the unmodified text of `https://www.apache.org/licenses/LICENSE-2.0.txt`, SHA-256 `cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30`. The same bytes ship as `/usr/share/common-licenses/Apache-2.0` on Debian and Ubuntu. Do not fill in the appendix's `[yyyy] [name of copyright owner]` placeholder; the copyright line lives in `NOTICE`.
- `NOTICE` is exactly these two lines, with a trailing newline:
  ```
  codecite
  Copyright 2026 James Crowley
  ```
  2026 is the year of the first commit; James Crowley is the repository owner and author of every commit.
- No changes to `Cargo.toml`, `deny.toml` or any crate.

## Review Focus

- The license text is the exact canonical text. A reformatted or truncated copy stops GitHub and `askalono`-style SPDX matchers from detecting Apache-2.0. Checked in Task 1 Step 2 by hash.
- GitHub's license detection reads only the root `LICENSE`; a file named `LICENSE.txt` or `LICENSE.md` also works, but the README links must match the name used. Checked in Task 1 Step 4 by resolving every README link.
- The README no longer says the license is "to be added". Checked in Task 1 Step 4 by grep.
- The gates stay green: these files touch no Rust code, but `cargo deny check` reads the workspace's license field. Checked in Task 1 Step 5.

---

### Task 1: `LICENSE`, `NOTICE` and the README pointer

**Files:**
- Create: `LICENSE`
- Create: `NOTICE`
- Modify: `README.md:37-39` (the `## License` section)

**Interfaces:**
- Consumes: nothing.
- Produces: root `LICENSE` and `NOTICE` that later crates and release archives can include.

- [ ] **Step 1: Confirm the files are missing**

Run: `test -e LICENSE || test -e NOTICE; echo $?`
Expected: `1`.

- [ ] **Step 2: Create `LICENSE` and check its hash**

Copy the canonical text: `curl -sSfo LICENSE https://www.apache.org/licenses/LICENSE-2.0.txt` (offline fallback: `cp /usr/share/common-licenses/Apache-2.0 LICENSE`).

Run: `sha256sum LICENSE`
Expected: `cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30  LICENSE`.

Adversarial: `sed 's/Version 2.0/Version 2.1/' LICENSE | sha256sum` gives a different hash, so the check catches a one-word edit.

- [ ] **Step 3: Create `NOTICE`**

Content as in Global Constraints.

Run: `cat -A NOTICE`
Expected: exactly `codecite$` and `Copyright 2026 James Crowley$`.

- [ ] **Step 4: Point the README at both files**

Replace the body of `## License` with one sentence: codecite is licensed under the Apache License 2.0, linking `[LICENSE](LICENSE)`, and attribution notices are in `[NOTICE](NOTICE)`.

Run: `grep -n "to be added" README.md; grep -o '](\(LICENSE\|NOTICE\))' README.md | sort -u`
Expected: no `to be added` match; both `](LICENSE)` and `](NOTICE)` present, and each target exists.

- [ ] **Step 5: Run the gates**

Run: `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo deny check`
Expected: all pass, `advisories ok, bans ok, licenses ok, sources ok`.

- [ ] **Step 6: Commit**

```bash
git add LICENSE NOTICE README.md
git commit -m "docs: add the Apache-2.0 LICENSE and NOTICE"
```
