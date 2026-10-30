The repository at `/work/project` is jamescrowley321/codecite. Make the change this issue
asks for. Stop when you're done.

# feat(mcp): diff a spec's pinned versions to show what changed in its rules

## Why
In the quick calibration of backchannel-logout, gpt-6.1-sol failed all 5 runs on one rule: Back-Channel Logout 1.0 errata set 1 made `exp` REQUIRED (§2.4), and the version before it didn't. Models learn from both versions. identity-model's own `validate_logout_token` follows the older one too. The corpus already pins both (`identity-bench.toml`: `openid-connect-backchannel-1_0`, current, and its pre-errata `-final`). But nothing tells an agent the two differ, or how.

## What
A version diff (an MCP tool, plus a CLI command). For a document, or one of its sections, it lists the normative statements that differ between two of its pinned versions, current against the latest older one by default:
- statements added, removed or changed;
- a requirement keyword's strength changing (OPTIONAL → REQUIRED, SHOULD → MUST);
- each with both citations.

Prose rewording with no normative change is left out, or listed apart.

`search` and `read` results from a document with an older pinned version say that one exists, and the server instructions point to the diff.

## Done when
1. **Test, Back-Channel Logout 1.0 §2.4 current against pre-errata:** the diff reports `exp` as added and REQUIRED, with both citations.
2. **Test:** a section with no normative change between versions reports none.
3. `docs/usage.md` documents the tool, and `docs/design.md` describes how versions are compared.
4. A quick-tier A/B run in CI on backchannel-logout has its scorecard posted on the PR.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
