## Blind peer review: Cold Read (#860, round 3)

**Verdict:** BLOCK (1 finding(s), 1 MUST FIX). Reviewed out-of-host by codex; base `origin/fix/859-hash-presence-per-bound-value`.

Found a fail-open signing-key selection vulnerability.

### MUST FIX: `go/pkg/jwks/jwks.go:235`

When every key has use="enc", narrow restores the entire set and a sole encryption key is returned as a signing key. JWT validation does not reject its use, so a token signed with an encryption-only private key can authenticate successfully. Rust introduces the same fallback.

**Recommendation:** Return an error when no signing-eligible key remains instead of restoring encryption keys. Apply this to both implementations and replace the tests that require encryption-key fallback.

### Critic disposition

- **`use: enc` fallback (out of scope, follow-up #888, owner decision pending):** this is the same finding as rounds 1 and 2. py, the certified reference, has the same fallback (`_no_kid_candidates`). In all three languages, a token whose `kid` names an `enc` key already verifies against it. `use` is OPTIONAL in RFC 7517 §4.2, so refusing `enc` keys is hardening that has to be decided for every language and both resolution paths. That decision is #888, which has been escalated to the owner. Diverging Go/Rust from py here would leave the kid path open and break parity, which is this story's whole point (DEC-015). Not blocking.
- The round-2 in-scope part (the contract text said "MUST narrow to signing keys") is fixed in this round. `spec/capabilities.md` and the JWKS-009 `then` now say each narrowing step applies only when it leaves a key, cite #888, and match all three implementations.
