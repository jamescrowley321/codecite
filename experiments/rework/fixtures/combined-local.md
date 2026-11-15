## Blind peer review (local, codex)

This PR edits `.github/workflows/claude.yml`, so the `@claude` review skips it. The lenses ran locally instead, out-of-host on codex, against `git diff origin/main...HEAD` at 8f46cca.

| Lens | Result |
|---|---|
| Cold Read | PASS (0 findings) |
| Edge Cases | PASS (0 findings) |
| Acceptance Criteria | PASS (0 findings) |
| Security Review | PASS (0 findings) |
| Red Team | PASS (0 findings) |
| Policy & Provenance | PASS (0 findings) |
| OWASP Web Top 10 | PASS (0 findings) |
| OWASP LLM Top 10 | PASS (0 findings) |

**Verdict: PASS**

🤖 Generated with [Claude Code](https://claude.com/claude-code)
