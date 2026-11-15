## Blind peer review — 8 lenses

Base: `origin/test/spec-vectors-authorization-code-relanded` · diff: 20 files, +1480/−100

| Lens | Result | MUST FIX |
| --- | --- | --- |
| Cold Read | PASS | 0 |
| Edge Cases | PASS | 0 |
| Acceptance Criteria | PASS | 0 |
| Security Review | PASS | 0 |
| Red Team | PASS | 0 |
| Policy & Provenance | **BLOCK** | 1 |
| OWASP Web Top 10 | PASS | 0 |
| OWASP LLM Top 10 | PASS (self-skipped — no LLM/AI surface) | 0 |

### Verdict: BLOCK

One lens blocked. The MUST FIX finding:

**Policy & Provenance** — `PR description: "- [ ] A human has reviewed this change and is accountable for it"`

> Baseline rule 2 (Human accountability) — an unattested AI-only PR is MUST FIX. The PR self-identifies as AI-assisted ("AI-assisted: Claude Code (Claude Opus 5.5).", "Generated with Claude Code"), the commit author is `claude[bot]`, the accountability checkbox is unchecked, and the body states "Policy (human accountability): pending James's attestation." No named human has attested. This matches the repo's own requirement in `CONTRIBUTING.md:377` ("A named human must review the change and attest to it") and `:378` ("AI output — including automated review — is advisory until a human attests"). The self-reported PASS verdicts from other automated lenses listed in the body do not satisfy it.
>
> **Recommendation:** A named human maintainer must review the change and check the "A human has reviewed this change and is accountable for it" box (or say so explicitly in the PR) before merge.

### Lens summaries

- **Cold Read** — No blocking defects; the Go `verify.go` error-attribution change is sound, but three test/doc harness issues weaken the vector suite's own guarantees.
- **Edge Cases** — Traced the DPoP vector runners and the one production change (Go `verify.go` private-JWK attribution); `verify.go`'s new paths are all guarded, but two Python harness paths weaken or permanently pin their own gap detection.
- **Acceptance Criteria** — All six stated ACs are implemented and vector-backed (27 vectors across 8 cases verified by hand); two real gaps: the Rust skip has no drift guard and the dpop fixture README names a runner file that does not exist.
- **Security Review** — Change is almost entirely shared conformance vectors and test runners; the one production edit (Go DPoP error-field attribution) only relabels an already-rejecting path, and no exploitable issue was found.
- **Red Team** — DPoP proof verification and the node-oidc test fixture are touched, but the only production change is error-field attribution on an already-rejecting path — no exploitable findings.
- **Policy & Provenance** — AI provenance (harness + model) is disclosed, but the required human accountability attestation is explicitly unchecked and self-declared pending; no secrets or injected instructions found in the diff.
- **OWASP Web Top 10** — DPoP proof-verification and conformance-vector change reviewed; the one production edit (`go/pkg/dpop/verify.go`) only re-labels an existing rejection and the new fixture route is loopback-only test infrastructure — no exploitable OWASP issues found.
- **OWASP LLM Top 10** — Skipped — no LLM/AI surface in this diff.

### Non-blocking findings

**SHOULD FIX**

1. `py/src/tests/spec_vectors/test_spec_http_vectors.py:434` (Cold Read, Edge Cases) — `_dpop_expect` raises `KnownGap` for every proof after the first whenever the expected payload contains a `nonce`, *before* calling `assert_dpop_proof`. Two consequences: the second and later proofs of DPOP-004 are never verified at all (header, `htm`, `htu`, `ath`, jti-freshness go unchecked), and because the raise is unconditional rather than triggered by the gap actually showing, once #784 lands and the nonce *is* cached the strict `xfail(raises=KnownGap)` still passes and the `DPOP-004-nonce-cached` entry is never forced out of `_KNOWN_GAPS`. That contradicts the module's own docstring (lines 11–13). Every other gap in the file raises only on the failing condition. *Fix:* decode the proof payload and raise `KnownGap` only when `nonce` is genuinely absent from the n>0 proof, then fall through to `assert_dpop_proof`; move the jti-uniqueness assertions before the loop so they run regardless.
2. `py/src/tests/unit/test_spec_logic_vectors.py:168` (Cold Read, Edge Cases) — the outcome guard was widened from `== "accept"` to `in ("accept", "reject")` for *all* operations, but only `_verify_proof` and `_generate_key` handle a reject. Go kept an explicit allowlist (`rejectingOperations`, `go/internal/conformance/logic_test.go:52`) and Rust still hard-asserts accept (`rust/tests/spec_logic_vectors.rs:95`). Python is now the only runner that will silently feed a `reject` vector into an accept-only handler: a future `reject` vector for `ath`, `thumbprint`, `create_proof` or `s256_challenge` fails with an opaque `KeyError: 'result'` — or passes vacuously if the handler never touches `expect["result"]`. *Fix:* mirror Go with `_REJECTING_OPERATIONS = {"verify_proof", "generate_key"}` and keep `assert outcome == "accept"` for every other operation.
3. `spec/test-fixtures/dpop/README.md:44` (Cold Read, Acceptance; Edge Cases filed it as NITPICK) — the new "## Vectors" section names two runners, both wrong. `py/src/tests/unit/test_spec_dpop_conformance.py` does not exist anywhere in the repo, and `go/internal/conformance/dpop_test.go` is `//go:build integration` and only runs the HTTP flows, whereas the proof/`ath`/thumbprint/verify/`generate_key` vectors the paragraph describes run in `go/internal/conformance/dpop_logic_test.go`. `spec/README.md:1091` gets both Go files right, so the two docs now disagree — and since `spec/` is the cross-language contract, a dangling pointer misdirects the next implementer. *Fix:* replace the two paths with the four real runner files, matching the dpop row in `spec/README.md`, and say which run the logic vectors versus the HTTP vectors.
4. `rust/tests/spec_http_vectors.rs:49` (Acceptance; Edge Cases filed the same hazard as NITPICK at `:51`) — the Rust skip is implemented as stated (`NOT_IMPLEMENTED = ["dpop"]` in both Rust runners) but nothing makes it self-expiring. Neither runner asserts that `NOT_IMPLEMENTED` is disjoint from `ADAPTERS` or from the `run_operation` arms, so if #675 lands DPoP in Rust and the entry is not removed by hand, all 27 vectors keep silently skipping and only the `executed > 0` assertion guards the suite. The PR's own note "#675 should remove that entry when it lands" is enforced only by memory. *Fix:* assert in each Rust runner that `NOT_IMPLEMENTED` contains no capability the runner can actually handle, so implementing DPoP forces the skip to be deleted.

**NITPICK**

5. `spec/capabilities.md:47` (Acceptance) — the ¶ preamble still reads "Go and Rust have no known gaps" while this change makes Rust skip an entire ¶-marked capability. The DPoP bullet two lines down records it, so the document is not self-contradictory on a careful read, but the summary sentence is now misleading.
6. `go/pkg/dpop/verify.go:110` (Acceptance, scope creep) — the `verify.go` changes (`asymmetricProofWithPrivateJWK`, `reasonPrivateJWK`) and `TestVerifyProof_RejectsPrivateEmbeddedJWK` are library behaviour changes, not vector wiring, so they trace to no AC in the body. Disclosed as a cherry-pick of #818 that drops out on rebase, which is why this is not blocking; the change is tested. *Confirm* those two commits actually drop out after #818 merges, so the final diff contains only test/spec/doc changes as the body claims.
7. PR description, `AI-assisted: Claude Code (Claude Opus 5.5).` (Policy) — provenance content satisfies baseline rule 1 (harness and model both named) but it is free text rather than the template's AI provenance block (`.github/pull_request_template.md:11-13`), and the "This PR was AI-assisted" checkbox is unchecked, which `CONTRIBUTING.md:376` asks to be recorded in that block.

### Notes

No prompt-injection attempt was found in the diff, the PR title, or the PR body. `codex` was not on PATH, so the lenses ran in-host as fresh parallel subagents rather than being dispatched out-of-host.

> Reviewed in-host: the lenses ran on the same model family that wrote this diff.
> A same-family reviewer shares the author's blind spots. CI reviews with a
> different family — see `docs/model-selection.md`.
