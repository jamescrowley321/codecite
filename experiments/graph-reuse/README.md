# CodeGraph reuse benchmark

This experiment for #295 asks whether graph data that CodeGraph extracts outside codecite adds trustworthy, useful navigation beyond codecite, before anyone builds an importer (milestone "M8 CodeGraph and Graphify adapters"). It covers what the generic retrieval benchmark (#287, PR #294) left out: edge queries (callers, callees and references). Every measurement is a local `codecite`, `codegraph` or `normalize.py` process, run under `unshare -rn` so it has no network. No model, agent or MCP server is in the loop, and nothing is imported into a codecite store. The plan is `docs/superpowers/plans/2026-11-22-codegraph-reuse-benchmark.md`.

## Findings

1. **Don't build a CodeGraph importer now.** No CodeGraph arm met the predeclared bars on any evaluation repository, let alone two. Django fails on reference recall (0.40) and one falsely unique answer. ripgrep fails on precision (0.89 at best) and on recall against SCIP (0.89 against 1.00, with no edge SCIP missed). Excalidraw fails on precision (0.79), forbidden targets and one falsely unique answer. These precision and recall figures are bar 1's and bar 2's quantity: audited callers and callees cases only, summed per repository from `results/per-case.jsonl` (ripgrep `codegraph/normalized-trusted` 25 TP, 3 FP, 3 FN; Excalidraw 11 TP, 3 FP, 4 FN). The "Per repository" table in `results/summary.md` also counts reference cases, which is why it shows ripgrep at 0.90. Reusing the CLI fails worst (point 3).
2. **The stored callers edges are good. Callees and references are not.** On the evaluation repositories, `codegraph/normalized-trusted` scores callers precision 1.00 and recall 0.85 (34 of 40 edges, none wrong), against 0.86 and 0.80 for `codecite+scip/id`. Callees reach only 0.76 precision: calls to `Ok`, `Err` and `Some` bind by bare name to unrelated repository definitions, and `this.elementsChange.applyTo(...)` binds to the calling method itself. Per-site references reach 0.73 recall (0.40 trusted), because CodeGraph stores an import as an edge to an import node rather than to the symbol. SCIP gets 1.00. CodeGraph also missed every `super()` call in Django's `combine_expression` case (0 of 3).
3. **The CLI loses most of what the database knows.** `codegraph callers/callees` scores precision 0.57 against 0.88 for the same database read by node ID. It mixes type, constant and import references into callers and callees, and prints no call site and no confidence. For a name with several definitions it answers for one, or merges them, without saying so (all 5 such cases, Linux included, while `codecite+scip/name` listed the candidates in all 4 on the evaluation repositories). It has no references command. It exits 0 when a symbol is not found, and each call costs 140 to 650 ms, mostly process start. So reusing the CLI is out.
4. **Where CodeGraph found edges SCIP missed, the cause was a codecite defect, not a gap in SCIP.** On Excalidraw, the normalized arm found at least 8 required edges that `codecite+scip/id` lacks (callers and callees recall 0.36 for `codecite+scip/id` there). Every one of them is a call to a `const f = () => ...` function. scip-typescript gives such a function a term symbol, not a method symbol, so codecite stores the call as `references`, and `graph callers` walks only `calls`. Fixing codecite's SCIP edge rules is cheaper and more precise than an importer. The other codecite defects this run exposed, all generic and all for #269 (#243 for the last), are:
   - **Bare-name lookup counts scip-python parameters.** `f().(x)` matches `f`, so every Django name query was `ambiguous` (13 of 13). Only by-ID queries worked.
   - **A reference to a method that is not a call becomes a `calls` edge from whatever encloses it**: a Rust `use` line (the module becomes a "caller"), a TypeScript import, and Django's `parser.UnparsedEntityDeclHandler = self.unparsed_entity_decl`.
   - **A scip-python attribute definition becomes the enclosing caller.** In `self._regex, ... = _route_to_regex(...)`, the caller is reported as `_regex`, not `RoutePattern.__init__`.
   - **scip-typescript gives a class's static and instance methods of the same name one symbol**, so their callers merge, and the instance method can't be found by location.
5. **Without SCIP, codecite has no edges at all.** The tree-sitter-only corpus answered `unsupported` for every edge case (48 of 48). That is the only ground where CodeGraph is the sole source. On the controls (Keycloak, Linux; 5 edges), the trusted filter kept all 5 and dropped every wrong edge. Linux's 7 wrong edges were calls through `->direction_output` function-pointer fields, stored with no confidence. This is control data and too small to claim anything (see "What it would take").
6. **Source verification works and matters.** Every file of the three evaluation snapshots matched the checkout by hash, except one ripgrep symlink. Keycloak has 2 files whose bytes differ from CodeGraph's recorded hash (both ISO-8859 test resources), and Linux has 25 symlinks and one file over 16 MiB. A CodeGraph file node's span runs one line past the end of its file, so file-level edges fail span checks and drop out of the trusted arm, which is the right outcome. All 213 codecite citations read back. All five timed rounds reproduced the sweep's output byte for byte. No snapshot changed.
7. **Cost.** `normalize.py` answers in 26 to 34 ms (p50 to p95) on every repository, Linux included, once the snapshot has been hashed (2.9 s for Linux's 4.7 GB). codecite answers in 4 to 8 ms. Producing the graph costs CodeGraph 1.9 to 584 s and 0.5 to 9.5 GB peak RSS. The SCIP indexers took 6 to 79 s and 0.6 to 3.9 GB on the three evaluation repositories, and they are unavailable for Java and C without a build.

### Edge kinds

| Edge kind | CodeGraph evidence (evaluation, trusted) | Worth importing? |
| --- | --- | --- |
| callers (`calls`, incoming) | P 1.00, R 0.85; `codecite+scip/id` P 0.86, R 0.80 | Only where no SCIP indexer runs, after fresh holdouts in such languages |
| callees (`calls`, outgoing) | P 0.76: bare-name binding of constructors and self-calls | No |
| references and imports | R 0.40 (0.73 untrusted); imports point at import nodes | No; SCIP is complete here |
| ambiguity and unknowns | Falsely unique on 2 of 5 indirect or unknown cases | Never above `ambiguous` (0) confidence |

### What it would take

Before any production importer, per the plan's Task 3:

- Rerun these bars on fresh holdouts in languages without a local SCIP indexer, since that is the only ground left once codecite's SCIP edge rules are fixed.
- If they pass, the boundary is an **externally normalized, versioned interchange file**, not CLI reuse and not direct SQLite parsing. The CLI drops sites, confidence and ambiguity. The SQLite schema (version 9, extraction 25) is internal to CodeGraph, so coupling to it means pinning both versions and a freshness contract.
- Import only `calls` edges with stored confidence of at least 0.7 whose endpoints verify against source, at `ambiguous` (0) or `inferred` (1) confidence, never `extracted` (2): bare-name binding produced every wrong edge here.
- Agree on per-source and per-provider ownership and provider-local replacement in one atomic publish (#243), and on graph edge semantics, including what a reference to a method that isn't a call is (#269).
- Snapshot authentication (a pinned hash, as here) and freshness; SCIP precedence, with SCIP-only outputs unchanged when no CodeGraph artifact is named; and ordinary search untouched when the opt-in is absent. The importer's own manifest and CLI changes need a separate reviewed design.

## Errata

- `ripgrep/callers/search_preprocessor`'s target line was 244, the doc comment above `fn search`, copied from a tree-sitter span start. The audit found the extra edge at line 245, and the label was corrected to 245 (the case's `erratum` field records it). The validator accepted 244 because the doc comment contains the word `search`, so it can't catch a label on a comment line that names the symbol. No other label changed after measurement.

## Split

`corpora.toml` reuses #294's five repositories, split and pins, so both benchmarks describe the same source:

| Split | Repository | Language | Pin | SCIP baseline |
| --- | --- | --- | --- | --- |
| evaluation | Django | Python | tag 6.0.5 | scip-python 0.6.8 |
| evaluation | ripgrep | Rust | tag 16.0.0 | rust-analyzer 1.97.1 `scip` |
| evaluation | Excalidraw | TypeScript | tag v0.18.1 | scip-typescript 0.6.2 |
| control | Keycloak | Java | #255's pin | unavailable (scip-java needs a Maven build) |
| control | Linux | C | #255's pin | unavailable (scip-clang needs a configured build) |

#294 measured definitions and search on the evaluation repositories, so they are no longer fresh for ranking questions. No edge output of either tool on them had been read when the cases were frozen, and edges are the only thing scored here. That is weaker than a fresh holdout: rotate in new repositories before claiming more (see "Scaling up"). The CodeGraph databases are #294's, built by `codegraph init` 1.6.0 (schema 9, extraction 25) and copied, after checking for an empty WAL, into private pinned checkouts. Their SHA-256s, and the SCIP files', are in `corpora.toml`.

## Cases

`cases.json` holds 48 cases: 42 on the evaluation repositories (15 ripgrep, 13 Django, 14 Excalidraw) and 6 controls. They were labelled from the pinned source alone, before any edge output of either tool was read:

- `sample.py` drew definitions in seed order from each corpus's tree-sitter symbols, with every other line naming them. `draws.json` records each draw as paths and lines. A person read every mention and decided whether it binds to the drawn definition.
- Classes: `direct_call` (21), `identical_name` (8: callers or callees that share a bare name, or a bare-name query whose name has several definitions), `non_call_reference` (6: imports and attribute references, via `refs`), `qualified` (4: `Type.member`), `indirect` (3: dynamic dispatch, where the declaration or any implementation is acceptable and one implementation alone is falsely unique), `unknown` (2: a call through a parameter, which no repository definition is the target of) and `miss` (4: nothing calls the definition).
- An endpoint is a path and the line that names the definition; a target can also be a span (`within`) for an anonymous function. Each expected edge carries its call sites. `forbid` lists known wrong targets, usually same-named definitions, so a merge by name is visible. Edges a reader could reasonably dispute (calls inside a macro body, an attribute read of a function object, self dispatch that a subclass might override, an instantiation) are `required: false`: they neither raise recall nor lower precision.
- SCIP is corroboration, not the oracle: every extra edge any arm returned is checked against the source after the run and recorded in `audit.json` as `false` or `label-miss`.

`run.py validate` rejects, before any CLI runs: duplicate IDs, unknown classes, checkouts off their pin or dirty, symbols, targets, sites or reference lines that are missing or don't name the symbol, and miss or unknown cases that have answers.

## Arms

| Arm | What it runs | Asked by |
| --- | --- | --- |
| `codecite` | `codecite graph callers/callees`, `codecite refs` on the tree-sitter-only corpus | name |
| `codecite+scip/name` | the same commands on the corpus with its SCIP index | name |
| `codecite+scip/id` | the same, with the SCIP ID of the definition holding the case's line (from an untimed `codecite def`) | ID |
| `codegraph/cli` | `codegraph callers/callees --json --limit 50` (`codegraph` has no references command) | name (`Type::member` when qualified) |
| `codegraph/normalized` | `normalize.py` over the private CodeGraph snapshot, every stored edge | path and line |
| `codegraph/normalized-trusted` | the same output, only edges that are source-verified with stored confidence at least 0.7 | path and line |

Definition lookup (`codecite def`, `codegraph query`) runs for every case too, as #294 did, for timing; #294 already scored it.

## Normalization

`normalize.py` is a scratch reader, not an importer. It refuses an artifact whose SHA-256, schema version, producer version or extraction version differs from its pin, whose index state is not `complete`, which has a nonempty WAL, or whose database path is a symlink, and it opens the file with SQLite's `immutable=1`, so nothing is ever written. Every query goes through stored node IDs: a symbol is found by path and line (or an exact qualified name), and a bare name that matches several nodes is reported as ambiguous with every ID. An edge is `verified` only when both endpoint files are inside the checkout (no `..`, absolute path or symlink), their bytes match the hash CodeGraph recorded, both spans fit their files and the site lies inside the source's span. Certainty is carried, never promoted: `extracted` for stored confidence of at least 0.7, `low` below it, `unknown` without one, and never `exact`. A pair-only edge keeps `site: null`. Sites CodeGraph could not bind (`unresolved_refs`) are reported, not dropped. A name appearing on the site line is never used as evidence of the binding.

## Predeclared bars

Fixed before any arm was compared, after the labels were found feasible.

Build a CodeGraph importer only if, on at least two of the three evaluation repositories, a CodeGraph arm meets all of these on audited cases:

1. callers and callees edge precision of at least 0.90;
2. recall within 0.05 of `codecite+scip/id`, or at least three source-verified correct edges that `codecite+scip/id` misses;
3. no falsely unique answer on an indirect or unknown case, and no hit on a `forbid` target;
4. an empty answer on every miss case;
5. reference (per-site) recall of at least 0.80.

Then choose the boundary:

- **Reuse the CLI, no import**, if `codegraph/cli` meets 1, 3 and 4 and exposes every bare-name ambiguity rather than answering for one definition.
- **An externally normalized, versioned interchange file** for the first importer spike, if a normalized arm meets the bars and the CLI does not.
- **Direct SQLite parsing** only if the bars are met and a bounded schema and freshness contract makes the coupling worth it.
- **No importer** if no CodeGraph arm meets them. A repository where an arm is unsupported or unavailable is inconclusive for that arm, not a loss.

Controls (Keycloak, Linux) are reported separately and never count toward the two-repository rule.

## Protocol

- `prepare.sh` copies each CodeGraph snapshot into a private pinned checkout and builds two codecite corpora per repository, without and with its SCIP index (`scip_sha256` pinned). codecite's data and cache homes are inside the run directory and every tool runs under `unshare -rn`.
- The SCIP indexes were produced in separate scratch checkouts: scip-python and scip-typescript with no network and no installed dependencies; rust-analyzer after `cargo fetch --locked` into a private `CARGO_HOME`, then with no network. rust-analyzer runs build scripts and proc macros, so it is only for trusted source (the design's Security section); ripgrep's release tag was treated as trusted for this run.
- `run.py measure` hashes each snapshot once against its pin, resolves the SCIP IDs for `codecite+scip/id` (untimed), then runs one untimed sweep of all 324 calls, whose outputs are scored, and five timed rounds shuffled with a fixed seed, alternating direction, one fresh process per call. Times include process start. Each round's stdout is compared with the sweep's.
- `run.py check`, outside the timed runs, reads every codecite citation back through `codecite read`, checks that every `codegraph` CLI endpoint's path and start line exist in the checkout, and checks every file in each CodeGraph file table against the checkout through `normalize.py`.
- `run.py report` scores the sweep, applies `audit.json`, and writes `results/`: `per-case.jsonl` (per case and arm: status, cited answer, score, extras), `calls.jsonl` (argv with local paths replaced by `$RUN`, `$SRC` and `$HERE`, exit code, byte counts, times), `summary.json`, `summary.md` and `metadata.json`.

Statuses are kept apart: `ok`, `unsupported` (the tool has no such operation, or codecite has no SCIP for the symbol), `unavailable` (no SCIP index exists for the repository, so the arm was not run), `ambiguous` (the tool refused a name with several definitions and listed them), `not_found`, `unresolved` (the case's SCIP ID could not be found) and `error`. Only `ok` answers carry edges; an exposed ambiguity counts its expected edges as missed, since the caller has to ask again.

## Reproduce

From the repository root, with Python 3.11 or later, Git, `unshare`, the `codegraph` CLI 1.6.0 and the SCIP indexers named in `corpora.toml`:

```sh
cargo build --release --locked -p codecite
run=$(mktemp -d); src=$run/src; mkdir -p "$src" "$run/scip"
# a clean checkout of every repository in corpora.toml, at its pin, under $src/<name>;
# #294's prepare.sh builds the CodeGraph databases (in $cgsnap/<name>/.codegraph/);
# SCIP indexes from separate scratch checkouts into $run/scip/<name>.scip (see Protocol)
experiments/graph-reuse/prepare.sh "$run/run" target/release/codecite "$src" "$cgsnap" "$run/scip"
python3 -I experiments/graph-reuse/run.py validate --src "$src"
python3 -I experiments/graph-reuse/run.py measure --run "$run/run" --src "$src" --codecite target/release/codecite --rounds 5
python3 -I experiments/graph-reuse/run.py check   --run "$run/run" --src "$src" --codecite target/release/codecite
python3 -I experiments/graph-reuse/run.py report  --run "$run/run" --src "$src"
python3 -m unittest experiments/graph-reuse/test_graph_reuse.py
```

`report` reads the index and SCIP timings from `$run/run/index/` and `$run/logs/`. On this host the sweep and five timed rounds took about 3 minutes, hashing the snapshots included; `check` took a few seconds more. Raw stdout and stderr stay in the run directory and are not committed.

## Scaling up

- **Fresh holdouts:** the evaluation repositories here have now been read for edges too. Pick new ones, especially in languages with no local SCIP indexer, since that is where CodeGraph could matter. Add a `[[repo]]` to `corpora.toml`, run `sample.py`, label the cases by the rules above, and commit both before measuring.
- **More cases:** raise `--n` in `sample.py`. Ten or more callers cases per repository would give per-repository intervals; the indirect and unknown classes need more than the 5 here before their falsely unique rate means much.
- **After codecite's SCIP fixes:** rerun unchanged, to see how much of CodeGraph's edge over `codecite+scip/id` on Excalidraw is left.
- **Another producer version:** pin it in `[producer.codegraph]`. `normalize.py` refuses any other schema, producer or extraction version until its expectations are reviewed.
