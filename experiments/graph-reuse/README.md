# CodeGraph reuse benchmark

This experiment for #295 asks whether graph data that CodeGraph extracts outside codecite adds trustworthy, useful navigation beyond codecite, before anyone builds an importer (milestone "M8 CodeGraph and Graphify adapters"). It covers what the generic retrieval benchmark (#287, PR #294) left out: edge queries (callers, callees and references). Every measurement is a local `codecite`, `codegraph` or `normalize.py` process, run under `unshare -rn` so it has no network. No model, agent or MCP server is in the loop, and nothing is imported into a codecite store. The plan is `docs/superpowers/plans/2026-11-22-codegraph-reuse-benchmark.md`.

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
