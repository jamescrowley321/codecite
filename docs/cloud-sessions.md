# Running issues in Claude Code cloud sessions

codecite issues can be worked unattended in [Claude Code cloud
sessions](https://code.claude.com/docs/en/claude-code-on-the-web). Each session
takes one issue and follows `AGENTS.md` ("Working an issue end to end"). CI and
the blind peer review (`claude.yml`, on James's Claude subscription) give the feedback, and the eval workflow runs the A/B eval.
James merges.

## The queue

- **Order:** the pinned issue #125 gives the order. Issues ready for a session
  carry the `cloud-ready` label, and each has a "Done when" checklist.
- **Defaults:** where an issue needed a design call, its checklist states a
  default a reviewer can check, and James can change it in review.
- **Issues that wait on James:** #125 lists the ones without the label and
  what each needs first.

## Setting up the environment

Once, at claude.ai/code:

1. Connect GitHub. The Claude GitHub App on this repo is best.
2. Create an environment with network access **Trusted**. It needs no
   environment variables or secrets: the eval key lives in CI as the Actions
   secret `OPENROUTER_EVAL_API_KEY`, not on the cloud machine.

On start, `.claude/hooks/cloud-setup.sh` installs the toolchain pinned in
`rust-toolchain.toml`, fetches crates and reports whether `gh` is logged in.
It only acts when `CLAUDE_CODE_REMOTE=true`, so local sessions skip it.

## Starting a session

```
claude --cloud "Work codecite#<n> per AGENTS.md 'Working an issue end to end'"
```

Issues the queue lists as independent can run in parallel, one session each.
`claude --teleport` brings a cloud session to a local terminal.

To start several at once, `scripts/cloud-launch.sh 111 117 105` starts one
session per issue and prints each link. `claude --cloud` needs a terminal, so
the script runs each launch under `script`, and it appends every link to
`~/.local/state/codecite/cloud-sessions.log`. Once it returns, nothing runs
locally.

`scripts/cloud-status.sh` shows where each open PR stands: its merge state
(`BEHIND` needs `gh pr update-branch <n>`, `DIRTY` has conflicts), the
verdict of the latest blind peer review (`stale` when commits landed after it),
the issue it closes, and whether the eval scorecard is posted. Sessions watch their own PRs, so a PR
comment reaches the session working it.

A session opens a PR against `main` and answers the review. After pushing
fixes it comments `@claude review this PR` for a fresh one; the session posts
as James, so the workflow's owner check lets it through. If the
change touches indexing, search, ranking or the CLI's output, it adds the
`eval` label, and the eval workflow posts the quick A/B scorecard on the PR.
Sessions never merge.

## If the first session looks wrong

- **The setup hook says `gh` is not logged in.** `AGENTS.md` tells the session
  to read PRs, checks and reviews with the GitHub connector tools instead.
- **`rustup` or `cargo fetch` fails at start.** The Trusted allowlist didn't
  cover the download. Switch the environment's network access to **Full**.

## What stays off the cloud machine

- **The A/B eval.** A cloud machine has about 4 vCPU and 16 GB, too little
  for a panel, and it holds no eval key. Run the eval through the eval
  workflow or on the operator's machine (`evals/README.md`).
- **Ralph as a long loop.** A loop dies when an idle cloud machine is
  reclaimed. To run Ralph in the cloud, make it a scheduled task (`/schedule`,
  at most hourly) that runs one iteration, and keep its state in the repo or
  on a branch, not in a git-ignored file.
- **User-level configuration.** Cloud sessions load the repo's `CLAUDE.md`,
  `AGENTS.md` and `.claude/`, not the operator's `~/.claude/` (CLAUDE.md,
  memory, plugins, skills). A rule a session must follow belongs in
  `AGENTS.md`.
