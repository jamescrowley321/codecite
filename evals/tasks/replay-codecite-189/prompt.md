The repository at `/work/project` is jamescrowley321/codecite. Make the change this issue
asks for. Stop when you're done.

# feat(cli): make the CLI an agent's natural search tool

## What

Compact default output (one line per hit with its citation, `--json` for structure), `--help` for every command written for an agent learning the tool cold, and a short instructions snippet for AGENTS.md or CLAUDE.md in `docs/usage.md`. The MCP instructions lose their spec-specific advice. CLI query time is measured against `rg` on the history benchmark's repositories.

## Done when

The history benchmark doesn't lose on any repository, and output bytes per query and query time are reported next to `rg`'s.

Part of the general-purpose design (`docs/superpowers/specs/2026-10-29-general-purpose-design.md`).\n\n🤖 Generated with [Claude Code](https://claude.com/claude-code)
