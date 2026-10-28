# Tell agents to search specs in the spec's own terms

Issue: #155, follow-up 2 and arm B of
`docs/superpowers/specs/2026-10-23-hybrid-retrieval-design.md`. BM25 finds
none of the plain-words spec questions; an agent's rewrite into spec terms
finds 19 of 22. Nothing in codecite asks the agent to rewrite.

## Steps

1. **Test first.** In `crates/codecite-mcp/src/server.rs`, a test that starts
   a client session and asserts that the `initialize` response's
   instructions and the `search` tool's description from `tools/list` both
   say to search with the terms the spec would use (protocol, parameter,
   claim, endpoint and error names) and to search the original wording too
   only when it already holds spec terms. Run it and see it fail.
2. **Guidance.** Add the same two sentences, with one example, to the
   `search` tool's description and the server's instructions. Point the
   `query` parameter's doc (MCP and CLI) at spec terms too.
3. **Docs.** `docs/usage.md`: a paragraph under Search and a line under
   Connect an agent. `docs/design.md`: a Ranking bullet on the spec-retrieval
   approach that links the spec and the A/B result, and the Vectors row's
   "Revisit if" names this eval as the trigger for dense vectors.
4. **Check.** `cargo fmt --check`, `cargo clippy --workspace --all-targets
   --locked -- -D warnings`, `cargo test --workspace --locked`.
5. **PR** with the `eval` label, and the quick tier dispatched on
   `backchannel-logout`, `client-auth-uri` and `spec-questions`, each
   scorecard posted on the PR. The quick tier is directional; the full tier
   is the gate.
