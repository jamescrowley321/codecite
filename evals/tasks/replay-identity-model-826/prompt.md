The repository at `/work/project` is jamescrowley321/identity-model. Make the change this issue
asks for. Stop when you're done.

# spec vectors: re-land authorization-code vectors on the shared runners

Part of identity-model#801. Needs identity-model#803, #804, #805 (and #806 where live vectors are used).

Re-land the authorization-code vectors on the shared runners: vector JSON + fixtures (reused from identity-model#795) and one adapter per language. No per-capability runner files.

## Scope
- `spec/vectors/authorization-code.json` executable vectors and `spec/test-fixtures/` files from identity-model#795, canned or `op: "live"` as fits.
- One adapter each in the Python, Go and Rust runners; Python gaps as `KnownGap` entries with linked issues.
- Pure logic (PKCE S256, RFC 7636 Appendix B) is an in-process vector in the unit suite, not an HTTP vector.
- Carries the doc-comment case-count fix from identity-model#794 (`ACG-001..006` -> `ACG-001..005` in `go/pkg/token/doc.go`, `rust/src/token/mod.rs`; the `pkce.rs` test comment).

## Acceptance
- Every vector in `authorization-code.json` executes in all three languages (Python gaps as strict xfail with an issue).
- identity-model#795 closed as superseded, linking the replacing PR.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
