# Security Baseline Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement #108. Make codecite public, as oss-admin's Terraform already says it should be, and gate every PR to `main` on the security checks the owner's other repositories run: workflow audit, secret scan, vulnerability scan, dependency review, CodeQL and Scorecard, plus Dependabot, SECURITY.md and branch protection.

**Architecture:** The new checks live in two workflow files beside `ci.yml`.
- `security.yml` holds the fast, repo-local gates: zizmor and gitleaks.
- `codeql.yml` holds the GitHub code-scanning jobs: CodeQL and Scorecard.

OSV-Scanner and dependency review get a file each (`osv-scanner.yml`, `dependency-review.yml`), as in identity-model, because each one's triggers differ from the others. Branch protection and private vulnerability reporting are set in oss-admin (`terraform/codecite`), never in the GitHub UI (AGENTS.md). The order matters, because GitHub offers code scanning, dependency review, secret scanning and branch protection to a free account only once the repository is public. So the visibility flip comes first, and the required checks come last, after each check has run once and its name is known.

**Tech Stack:** GitHub Actions; zizmor 1.31.1 (zizmor-action v0.7.1); gitleaks 8.30.1 (gitleaks-action v3.0.0); OSV-Scanner reusable workflows v2.7.0; dependency-review-action v5.0.0; CodeQL action v4.39.0; scorecard-action v2.5.0; Terraform with `integrations/github ~> 6.0` in oss-admin.

**Spec:** Issue #108; `docs/design.md#security-and-privacy`. The patterns copied come from `~/repos/auth/identity-model/.github/workflows/{codeql,dependency-review,osv-scanner,scorecard}.yml`, `~/repos/auth/identity-stack/.github/{workflows/security.yml,dependabot.yml}` and `~/repos/auth/oss-admin/terraform/identity-model/branch_protection.tf`.

## Global Constraints

- Every `uses:` is a full 40-hex commit SHA with a `# vX.Y.Z` comment. Pins:
  - `zizmorcore/zizmor-action@85b2b9fb8e3325e7a23d7d0a9d7ba5ae0f999e3e # v0.7.1`, with input `version: 1.31.1`
  - `gitleaks/gitleaks-action@e0c47f4f8be36e29cdc102c57e68cb5cbf0e8d1e # v3.0.0`
  - `google/osv-scanner-action/.github/workflows/osv-scanner-reusable{,-pr}.yml@b1d267aea5211fa648d9d442fa64b90dbdcbdd36 # v2.7.0`
  - `actions/dependency-review-action@a1d282b36b6f3519aa1f3fc636f609c47dddb294 # v5.0.0`
  - `github/codeql-action/{init,analyze,upload-sarif}@6f2ac81f5cd81fe351a197ba0b00d91d03cd2aee # v4.39.0`
  - `ossf/scorecard-action@fa7117db83c52d787ae5a89f350999701505d78e # v2.5.0`
  - `actions/upload-artifact@cf430e030ddbb5b0abf93d22962f4752f3646cd9 # v7`
  - `actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1`, as `ci.yml` has it
- No pinned release may be less than 7 days old on the day it is pinned. This matches Dependabot's `cooldown: default-days: 7`.
- Every workflow has a top-level `permissions:` block (`contents: read`, or `{}` where every job declares its own), and each job grants only what it uses.
- Every `actions/checkout` sets `persist-credentials: false`, unless a later step in that job pushes with the token. None does today.
- `pull_request` triggers have no `branches:` filter, so stacked PRs that target feature branches still get every gate (as `ci.yml` notes).
- Job `name:`s are the required-check contexts in oss-admin. Once Task 6 lands, renaming a job breaks merges until Terraform changes too.
- No names of the owner's other clients or projects in any file, commit, issue or PR. Write "another project".

## Review Focus

- **A fork PR on the now-public repository.** It runs `pull_request` workflows with a read-only token and no secrets. Every gate must still pass or fail on its merits rather than error on a missing secret. gitleaks needs no license on a personal-account repository; Task 3 asserts its job uses only `GITHUB_TOKEN`.
- **A comment from a stranger containing `@claude`.** `claude.yml` must do nothing. The OWNER/MEMBER/COLLABORATOR gate already enforces this. Task 2 keeps it and records why in the zizmor ignore, and the PR body quotes the `if:` lines so a reviewer can check it.
- **A Dependabot PR.** It must get every gate, and must not be able to skip the blind-peer-review merge gate by spoofing the actor. Task 2 replaces `github.actor == 'dependabot[bot]'` with `github.event.pull_request.user.login == 'dependabot[bot]'`.
- **A required check that never reports**, for example a renamed job or a path-filtered workflow. Every PR would then be stuck forever. None of the new workflows uses `paths:` filters, and Task 6 takes the check names from a real run, never from memory.
- **The planted test secrets.** A fake `sk-or-v1-…` key in `evals/lib/sandbox.test.mjs`, and private-key headers in `crates/codecite-index/src/walk.rs` and its plan. They must not fail gitleaks, and a real secret added next to them must still fail it. Task 3 ignores the three historic findings by fingerprint (not by path) and shows a fresh planted secret failing.

---

### Task 1: Pre-flight and make the repository public

Nothing is committed to codecite in this task. Going public cannot be taken back (forks, caches and search indexes keep what they saw), so every check here must be clean before the owner approves the flip.

**Files:**
- Read only: the full git history, issues, PRs and PR comments of `jamescrowley321/codecite`.
- Apply (owner): `~/repos/auth/oss-admin/terraform/codecite`. It already says `visibility = "public"`; the apply corrects the drift.

- [ ] **Step 1: Scan the full history for secrets**

Run: `gitleaks git --log-opts=--all --redact -v .`
Expected: exactly three findings, all planted fixtures:
```
a304f6a8e52f48a2991e6640e644f927873c5bce:evals/lib/sandbox.test.mjs:generic-api-key:218
d6b6d5885f1a1fc7327766629a0b12399c855cd5:crates/codecite-index/src/walk.rs:private-key:520
d6b6d5885f1a1fc7327766629a0b12399c855cd5:docs/superpowers/plans/2026-10-19-local-source-walk.md:private-key:699
```
Any other finding stops the task. Rotate the secret, then rewrite the history or stay private; that call is the owner's.

- [ ] **Step 2: Scan history, issues and PRs for other projects' names**

The owner supplies the names in `~/.config/codecite/private-names.txt`, one per line, outside the repository.

Run:
- `git grep -i -I -n -F -f ~/.config/codecite/private-names.txt $(git rev-list --all) | cut -c1-200`
- `gh issue list --state all --limit 500 --json number,title,body,comments` and `gh pr list --state all --limit 500 --json number,title,body,comments`, piped through `grep -i -F -f` on the same file.

Expected: no output. Every hit is fixed before the flip: edit the issue or PR text, or report it to the owner if it's in history.

- [ ] **Step 3: Owner applies the visibility change**

In Terraform Cloud, run a plan for `terraform/codecite`. It must show exactly one change, `visibility: "private" -> "public"` on `github_repository.this`, plus the secret-scanning blocks becoming effective. The owner confirms the apply.

- [ ] **Step 4: Verify**

Run: `gh api repos/jamescrowley321/codecite --jq '{visibility, ss: .security_and_analysis.secret_scanning.status, pp: .security_and_analysis.secret_scanning_push_protection.status}'`
Expected: `{"visibility":"public","ss":"enabled","pp":"enabled"}`

---

### Task 2: Audit the workflows with zizmor and fix its findings

**Files:**
- Create: `.github/workflows/security.yml` (job `zizmor`)
- Create: `.github/zizmor.yml`
- Modify: `.github/workflows/ci.yml` (every checkout), `.github/workflows/blind-peer-review.yml:25-26` and `:119`, `.github/workflows/claude.yml` (checkouts)

**Interfaces:**
- Produces: the workflow `Security` with the job `zizmor` (`name: zizmor`). Task 3 adds a job to the same file. Task 6 requires the context `zizmor`.

- [ ] **Step 1: Record today's findings**

Run: `uvx zizmor@1.31.1 .github/workflows`
Expected: FAIL with 9 unsuppressed findings (3 high, 6 medium):
- `artipacked` ×5 in `ci.yml`
- `excessive-permissions` ×2 at `blind-peer-review.yml:25-26`
- `bot-conditions` at `blind-peer-review.yml:119`
- `dangerous-triggers` at `claude.yml:83`

- [ ] **Step 2: Fix them**

- `artipacked`: add `with: persist-credentials: false` to every checkout.
- `excessive-permissions`: move `blind-peer-review.yml`'s top-level `pull-requests: write` and `checks: write` down to the jobs that post (keep the top level at `contents: read`). Use the action's README for which mode posts.
- `bot-conditions`: replace `github.actor == 'dependabot[bot]'` with `github.event.pull_request.user.login == 'dependabot[bot]'`.
- `dangerous-triggers`: in `.github/zizmor.yml`, ignore `dangerous-triggers` for `claude.yml` only, with a comment. The comment says every trigger in that workflow requires the author's association to be OWNER, MEMBER or COLLABORATOR (`claude.yml:108-113`), so a stranger's comment runs nothing, and that removing the trigger would remove `@claude` on PRs.

- [ ] **Step 3: Add the CI job**

`security.yml`:
- `on: pull_request`, `push: branches: [main]`
- `permissions: contents: read`
- job `zizmor`: checkout, then `zizmor-action` with `version: 1.31.1`, `advanced-security: false` (so findings fail the job instead of only reaching the Security tab), `persona: regular` and `min-severity: low`

- [ ] **Step 4: Verify**

Run: `uvx zizmor@1.31.1 .github/workflows`
Expected: `No findings to report` (with the one ignore counted as suppressed).

- [ ] **Step 5: Commit**

```bash
git add .github/workflows .github/zizmor.yml
git commit -m "ci: audit workflows with zizmor and drop persisted checkout credentials"
```

---

### Task 3: Scan for secrets and vulnerable dependencies

**Files:**
- Modify: `.github/workflows/security.yml` (add job `gitleaks`)
- Create: `.gitleaksignore`, `.github/workflows/osv-scanner.yml`, `.github/workflows/dependency-review.yml`

**Interfaces:**
- Consumes: `security.yml` from Task 2.
- Produces: the contexts `gitleaks` and `dependency-review`, and OSV-Scanner's PR job. Its context comes from the reusable workflow; Task 6 reads it off a run.

- [ ] **Step 1: Show a planted secret fails before the ignore exists**

In a scratch branch, commit a file with a fresh fake key: `sk-or-v1-` followed by 64 hex digits.

Run: `gitleaks git --log-opts=--all .`
Expected: FAIL with 4 findings (the 3 historic ones and the new one). Drop the scratch branch.

- [ ] **Step 2: Ignore the three historic fixtures by fingerprint**

`.gitleaksignore` holds the three fingerprints from Task 1 Step 1, one per line, under a comment naming each as a planted test fixture.

Run: `gitleaks git --log-opts=--all .`
Expected: `no leaks found`.

- [ ] **Step 3: Add the jobs**

- `security.yml` job `gitleaks`:
  - checkout with `fetch-depth: 0` and `persist-credentials: false`
  - `gitleaks-action` with `env: GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}` and nothing else, since a personal-account repository needs no `GITLEAKS_LICENSE`
- `osv-scanner.yml`: identity-model's file, verbatim apart from the comments:
  - PR diff job `scan-pr`, gating only newly introduced vulnerabilities
  - push/weekly job `scan-scheduled` with `fail-on-vuln: false`
  - `permissions: {}` at the top
- `dependency-review.yml`: identity-model's file, with `fail-on-severity: high` and `comment-summary-in-pr: on-failure`. Add `allow-licenses` with exactly `deny.toml`'s `[licenses] allow` list, so a PR that adds a crate under another license fails here as it does in `cargo deny`. The `deny.toml` exceptions stay in `deny.toml`, and dependency review gets them through `allow-dependencies-licenses`, as purls (`pkg:cargo/option-ext`, `pkg:cargo/webpki-roots`, …), one per `deny.toml` exception.

- [ ] **Step 4: Verify locally**

Run: `uvx zizmor@1.31.1 .github/workflows && osv-scanner scan source -r . ; echo rc=$?`
Expected: zizmor reports no findings. OSV-Scanner lists the scanned lockfiles (`Cargo.lock`, `evals/fixtures/node-oidc/package-lock.json`). Record its result in the PR body: either `rc=0`, or the advisories found, each checked against `cargo deny check advisories`.

- [ ] **Step 5: Commit**

```bash
git add .gitleaksignore .github/workflows
git commit -m "ci: scan for secrets, vulnerable dependencies and disallowed licenses"
```

---

### Task 4: CodeQL and Scorecard

**Files:**
- Create: `.github/workflows/codeql.yml`, `.github/workflows/scorecard.yml`

**Interfaces:**
- Produces: the contexts `Analyze (actions)` and `Analyze (rust)`. Scorecard is not a required check, since it runs on `main` only.

- [ ] **Step 1: Write `codeql.yml`**

Model it on identity-model's `codeql.yml`, with these differences:
- matrix `language: [actions, rust]`
- `init` takes `build-mode: none`, and there is no `autobuild` step (both languages are analysed from source)
- checkout sets `persist-credentials: false`
- cron `"0 6 * * 1"` (weekly)

- [ ] **Step 2: Write `scorecard.yml`**

Copy identity-model's `scorecard.yml` (triggers `branch_protection_rule`, push to `main`, weekly), with `publish_results: true`.

- [ ] **Step 3: Verify**

Run: `uvx zizmor@1.31.1 .github/workflows`
Expected: no findings. Then open the PR (Task 7, Step 3) and check that `gh pr checks` shows `Analyze (actions)` and `Analyze (rust)` passing.

- [ ] **Step 4: Commit**

```bash
git add .github/workflows/codeql.yml .github/workflows/scorecard.yml
git commit -m "ci: add CodeQL for Rust and Actions, and OpenSSF Scorecard"
```

---

### Task 5: Dependabot and SECURITY.md

**Files:**
- Create: `.github/dependabot.yml`, `SECURITY.md`

- [ ] **Step 1: Write `dependabot.yml`**

`version: 2` with three entries:
- `cargo` at `/`
- `github-actions` at `/`
- `npm` at `/evals/fixtures/node-oidc`

Each entry is `schedule: interval: weekly` with `cooldown: default-days: 7`, `open-pull-requests-limit: 5`, and `commit-message: prefix: "fix(deps)"` for `cargo` and `npm`, `"ci"` for actions. Groups are split by update type, as identity-stack's are: one group for `minor` and `patch` updates, majors as their own PRs. No auto-merge workflow; every Dependabot PR goes through the same review as any other.

- [ ] **Step 2: Write `SECURITY.md`**

Adapt `~/repos/auth/identity-model/SECURITY.md`:
- Reporting goes through GitHub private vulnerability reporting (`https://github.com/jamescrowley321/codecite/security/advisories/new`), acknowledged within 3 business days and assessed within 10.
- Supported versions: the latest release on crates.io.
- In scope: the `codecite` binary, the MCP server, manifest fetching and caching.
- Out of scope: the contents of indexed third-party corpora, and the `evals/` harness, which runs only on the owner's machine.

- [ ] **Step 3: Verify**

Run: `python3 -I -c "import yaml,sys; yaml.safe_load(open('.github/dependabot.yml'))" && grep -c "security/advisories/new" SECURITY.md`
Expected: no error, then `1`. After merge: `gh api repos/jamescrowley321/codecite/dependabot/alerts` returns `[]` or a list, not 403.

- [ ] **Step 4: Commit**

```bash
git add .github/dependabot.yml SECURITY.md
git commit -m "chore: add Dependabot with a 7-day cooldown and a security policy"
```

---

### Task 6: Require the gates on `main` (oss-admin)

This is a separate PR in `~/repos/auth/oss-admin`, on its own branch and worktree, opened after codecite's PR from Tasks 2–5 has run its checks once.

**Files:**
- Create: `~/repos/auth/oss-admin/terraform/codecite/branch_protection.tf`
- Modify: `~/repos/auth/oss-admin/terraform/codecite/{variables.tf,terraform.tfvars,README.md}`

**Interfaces:**
- Consumes: the check contexts from Tasks 2–4, read off a real run.

- [ ] **Step 1: Read the real check names**

Run: `gh pr checks <codecite PR> --json name,workflow --jq '.[]|"\(.workflow) / \(.name)"'`
Expected: lines including `fmt`, `clippy`, `test`, `deny`, `zizmor`, `gitleaks`, `dependency-review`, `Analyze (actions)`, `Analyze (rust)`, and OSV-Scanner's PR job. Copy the names exactly as printed.

- [ ] **Step 2: Write `branch_protection.tf`**

Follow identity-model's `branch_protection.tf:59-96`: a `github_branch_protection` on `main` with these settings:
- `required_status_checks` with `strict = var.required_status_checks_strict` and `contexts = var.required_status_check_contexts`
- `required_linear_history = true` (the repository is squash-only already)
- `require_conversation_resolution = true`
- `allows_deletions = false` and `allows_force_pushes = false`
- `enforce_admins = false`, so the owner can still recover a broken `main`

There is no existing protection to adopt, so no `import` block. The contexts in `terraform.tfvars` are the Step 1 names, minus `bench` and the blind-peer-review jobs (advisory by design, per `ci.yml` and `blind-peer-review.yml`).

- [ ] **Step 3: Enable private vulnerability reporting**

Run: `terraform providers schema -json | jq '.provider_schemas[].resource_schemas.github_repository.block.attributes | has("private_vulnerability_reporting")'`. Then:
- If `true`, set it on `github_repository.this` in `repository.tf`.
- If `false`, the provider can't manage it. Record that in `README.md` and have the owner enable it once with `gh api -X PUT repos/jamescrowley321/codecite/private-vulnerability-reporting`. `SECURITY.md` links to it.

- [ ] **Step 4: Verify**

The Terraform Cloud plan shows the new `github_branch_protection` and nothing else (or plus `private_vulnerability_reporting`). After apply:
`gh api repos/jamescrowley321/codecite/branches/main/protection --jq '.required_status_checks.contexts'` lists the Step 1 names.

- [ ] **Step 5: Commit**

```bash
git add terraform/codecite
git commit -m "feat(codecite): require the CI and security gates on main"
```

---

### Task 7: Say in design.md what is enforced, then open the PR

**Files:**
- Modify: `docs/design.md`, the "Supply chain" bullet under "Security and privacy"

- [ ] **Step 1: Rewrite the bullet**

It now says, replacing the current bullet:
- Actions are pinned by commit SHA, and Dependabot bumps them, `cargo` and `npm` after a 7-day cooldown.
- PRs to `main` are gated on `cargo-deny`, zizmor, gitleaks, OSV-Scanner, dependency review and CodeQL, as required checks in oss-admin.
- An embedding model is pinned by Hugging Face commit and per-file sha256, and fetched through the document fetcher (#109).
- Release signing, SBOM and provenance are #110. Until #110 lands, releases are published by hand and are not signed.

- [ ] **Step 2: Run the gates**

Run: `cargo fmt --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked && cargo deny check && uvx zizmor@1.31.1 .github/workflows && gitleaks git --log-opts=--all .`
Expected: all pass, then `No findings to report` and `no leaks found`.

- [ ] **Step 3: Commit and open the PR**

```bash
git add docs/design.md docs/superpowers/plans/2026-10-23-security-baseline.md
git commit -m "docs: state the enforced supply-chain controls"
git push -u origin ci/security-baseline
gh pr create --title "ci: gate PRs on security scans" --body "Part of #108 ..."
```
The PR body quotes the `claude.yml` `if:` lines (Review Focus) and the Task 3 OSV-Scanner result. #108 closes when Task 6's oss-admin apply is verified, not when this PR merges.
