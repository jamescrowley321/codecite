#!/usr/bin/env bash
# SessionStart hook: ready a Claude Code cloud session to build and check
# codecite. Local sessions skip it; a local machine is already set up.
#
# Never blocks the session: each step reports and moves on. Its output lands in
# the session's context, so it says what is missing rather than failing.
set -uo pipefail
[[ "${CLAUDE_CODE_REMOTE:-}" == true ]] || exit 0
cd "${CLAUDE_PROJECT_DIR:-.}" || exit 0

note() { echo "cloud-setup: $*"; }

# rust-toolchain.toml pins the channel and adds rustfmt and clippy.
if command -v rustup > /dev/null; then
  if rustup toolchain install > /dev/null 2>&1; then
    note "toolchain $(rustc --version)"
  else
    note "rustup could not install the pinned toolchain; check the environment's network access"
  fi
else
  note "no rustup; cargo fmt and clippy may be missing ($(rustc --version 2>&1))"
fi

if cargo fetch --locked > /dev/null 2>&1; then
  note "crates fetched"
else
  note "cargo fetch failed; check the environment's network access"
fi

# CI and the lens reviews come back through GitHub. Say which route works.
if gh auth status > /dev/null 2>&1; then
  note "gh is authenticated"
else
  note "gh is not authenticated: read PRs, checks and reviews with the GitHub connector tools"
fi
exit 0
