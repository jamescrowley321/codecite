## Blind peer review (local fallback): REVIEW_FAILED run 39495964434 succeeded but posted no review (self-skip or permission denials)

```
Dispatching 8 lens(es) to codex, one process each: cold_read, edge_case, acceptance, security, red_team, policy, owasp_web, owasp_llm

  Cold Read            PASS    1 finding(s), 0 MUST FIX
  Edge Cases           PASS    1 finding(s), 0 MUST FIX
  Acceptance Criteria  PASS    0 finding(s), 0 MUST FIX
  Security Review      PASS    0 finding(s), 0 MUST FIX
  Red Team             PASS    0 finding(s), 0 MUST FIX
  Policy & Provenance  PASS    0 finding(s), 0 MUST FIX
  OWASP Web Top 10     PASS    0 finding(s), 0 MUST FIX
  OWASP LLM Top 10     PASS    1 finding(s), 0 MUST FIX

Verdict: PASS
Reviewed out-of-host by codex — a different model family from the one that wrote this diff.
```

### Findings

- **SHOULD FIX** `crates/codecite-core/src/store/corpora.rs:56`: The metadata call propagates errors before per-corpus error handling. Since known_corpora includes dangling .db symlinks, one such entry makes both the CLI and MCP listing fail, hiding every healthy corpus. Concurrent removal produces the same failure.
- **SHOULD FIX** `crates/codecite-core/src/store/corpora.rs:56`: [DEGRADED] Trigger: A dangling broken.db symlink exists beside a healthy corpus. known_corpora includes the symlink, but metadata(...)? aborts the entire listing on NotFound. The CLI returns no corpus JSON and MCP returns a whole-tool error, hiding healthy corpora. Concurrent deletion produces the same failure.
- **SHOULD FIX** `crates/codecite-mcp/src/server.rs:43`: [LLM06] Every call starts a full corpus scan through spawn_blocking without an admission limit. rmcp dispatches requests concurrently, so an agent issuing many overlapping calls can accumulate blocking jobs, SQLite connections and filesystem work, especially while databases are busy.
