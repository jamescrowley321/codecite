# CodeGraph Reuse Benchmark Implementation Plan

> **For agentic workers:** Use superpowers:subagent-driven-development or superpowers:executing-plans. Evidence collection uses local direct CLIs, source checks and read-only data inspection, never agent runs.

**Goal:** Establish whether externally extracted graph data adds trustworthy, useful navigation beyond Codecite before choosing an importer.

**Architecture:** Compare source-grounded graph cases on baseline Codecite, CodeGraph CLI, and a scratch external normalization of the same CodeGraph snapshot. Import nothing into the production database in this experiment. Make a later provider decision on measured coverage, attributable evidence, compatibility and operational cost.

**Tech stack:** Python 3.15 standard library, Git, SQLite, local locked Codecite release, one pinned external CodeGraph build.

**Spec:** `docs/design.md` (Provider model and Security); coordinate with #243, #269 and #237. Preserve the agent eval in #275. Use the repository split and artifact protocol in `2026-11-22-generic-retrieval-benchmark.md`.

## Global constraints

- Codecite does not install, link, launch, or auto-sync CodeGraph. An optional artifact is externally produced and pinned to a specific producer/schema and source revision.
- Never write into or attach an external database to a production Store. Use a private consistent snapshot, fixed read-only schema queries, resource bounds, source-file hashes, and path containment. An incomplete, unpinned, unsupported, or unverifiable artifact is not evidence.
- An endpoint is revision, source, path, span and kind, with qualified identity where present. A bare name is not identity. Do not merge edges across overloads, providers or projects based on spelling.
- Carry provider, version, edge kind, source/target IDs, site and uncertainty through normalization. Missing provenance is not exact. Do not invent per-site references from pair-only relationships.

## Review focus

- Identical bare names in distinct symbols or repositories must never merge.
- Symlink escapes, changed sources, nonempty WAL and stale versions cannot become trusted source citations.
- A name mentioned at the recorded line does not establish the target binding.
- An absent SCIP index is unavailable baseline coverage, not zero recall or a competing ranking loss.
- Copying extra rows must not contaminate precise SCIP results or enlarge normal text-search latency.

### Task 1: Freeze graph answer sets and reproducible inputs

**Files:** `experiments/graph-reuse/{README.md,cases.json,corpora.toml}`.

- [ ] Select source-only cases across different repositories before looking at graph output. Cover direct calls, non-call references/imports, identical names, qualified queries, indirect or polymorphic calls, unknown, and deliberate miss cases. Label commit, source span/site, acceptable target set, and certainty. Use compiler/SCIP evidence where available as corroboration, not oracle.
- [ ] Pin the external CLI/package and schema. Prepare fresh isolated, consistent artifact copies and record their SHA-256, inclusion policy, producer version, revision, and per-file hashes. Keep Linux/Keycloak as controls and require repository-disjoint cases for any claim of generalization.

### Task 2: Test existing CLI reuse versus importing normalized evidence

**Files:** `experiments/graph-reuse/{run.py,normalize.py,test_graph_reuse.py}`; consult `crates/codecite-core/src/store/{graph,scip}.rs`.

- [ ] Test two same-named callers with disjoint targets and a negative edge; expect separate source IDs, exact site output, exposed ambiguity, no invented certainty, and explicit unsupported for missing SCIP. Reject fixtures with unknown schema, escaped paths, invalid spans, and mismatched source bytes or pin.
- [ ] Capture comparable raw `codecite def/refs/graph callers/graph callees` and `codegraph query/callers/callees` commands. Compare provider output with a scratch parser that follows stored node IDs, not CLI names. Keep baseline with matching SCIP separate from baseline without SCIP. Run fixed local commands with visible limits; an expanded result list is not evidence of unique attribution.
- [ ] Score definition and edge precision/recall only against audited cases; report coverage, ambiguous/unknown cases, falsely unique outcomes, unsupported/truncated outputs, startup-inclusive p95, source/citation validity, index bytes, ingest time and peak RSS separately. Check candidate agreement and discrepancies against source. No promoted confidence based on mere name or path agreement.

### Task 3: Use the evidence to choose an integration boundary

- [ ] Choose external tool reuse if qualified lookup supplies sufficient evidence without an import. If explicit stable source IDs and site-level evidence improve holdout navigation but the CLI loses that information, prefer an externally normalized, versioned interchange file for the first importer spike. Consider direct SQLite parsing only if a bounded schema and freshness contract justifies the coupling.
- [ ] Before any production importer, agree on per-source/provider ownership (#243), known graph edge semantics (#269), snapshot authentication and freshness, provider-local replacement in one atomic publish, and SCIP precedence. Test that disagreements and ambiguity are visible; use verified source identity before merging evidence. Require unchanged SCIP-only outputs and no mutation of ordinary search when opt-in is absent.
- [ ] Recommend import only if it adds source-verified attributable relationships on multiple holdout repositories without introducing falsely unique answers. Predeclare numeric bars after label feasibility, before comparing candidates. Treat unsupported/insufficient coverage as inconclusive. A production migration and manifest/CLI changes require a separate reviewed design.
