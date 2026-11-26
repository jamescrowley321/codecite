# Generic retrieval benchmark

This experiment for #287 looks for codecite retrieval failures that repeat across unrelated repositories, before any more ranking changes. Each miss is assigned to the layer it comes from: extraction (the answer was never indexed as its own definition), candidate retrieval (it was indexed, but the query never matched it), rank (it matched, but other chunks scored higher), or latency. Every measurement is a local `codecite` or `codegraph` process, run under `unshare -rn` so it has no network. No model, agent or MCP server is in the loop. The plan is `docs/superpowers/plans/2026-11-22-generic-retrieval-benchmark.md`.

## Findings

1. **Every codecite miss in this workload is a rank miss.** Across all five repositories, no scored answer was missing from the index (extraction) and none was missing from the candidate set (candidate retrieval). The untruncated diagnostic found every answer the CLI missed somewhere in the production ranking, at rank 2 to 3,179. Search ORs every query term together, so recall is not the problem. Ordering is.
2. **Behaviour queries are the weak class, and the weakness repeats on every held-out repository.** On the three evaluation repositories, codecite search scores Hit@1 0.13, Hit@10 0.80 and MRR@10 0.40 on 15 behaviour queries. The same queries are worded the way a person describes the code. On the development repositories it scores Hit@1 0.17 and Hit@10 0.33 (6 queries). Exact names are solved: `def` and `search` both score Hit@1 1.00 on every exact-name case in both splits, and `def` scores 1.00 on qualified names too.
3. **Two mechanisms explain every search answer that isn't first.** For each one, the diagnostic compares the answer's own chunk with the chunks ranked above it. 
   - *Other source chunks share more of the query's words* (14 of 19 evaluation misses, in all three evaluation repositories, and 2 of 7 development misses). The answer is usually just below the top: rank 2 to 6 in 10 of the 14.
   - *The answer's chunk is long* (5 of 19 evaluation misses, in Django and ripgrep, and 4 of 7 development misses, in Keycloak and Linux). The answer's chunk is at least three times longer than the median chunk above it: a whole class (`HttpResponseBase`, 261 lines; Keycloak's `TokenEndpoint`, 253, and `UserResource`, 1,276) or a long function (`tcp_v4_connect`, 143). BM25's length normalisation sinks it below short chunks that repeat one query word.
   - Tests, docs and examples ranking above source never made up the majority of the chunks above an answer in any repository. So the role weights are not what fails here.
4. **Latency only fails at the scale of Linux.** On the held-out repositories (5k to 60k chunks), codecite search answers at p50 6 to 54 ms and p95 at most 76 ms, process start included. `def` answers in 4.6 to 6.5 ms everywhere. On Linux (1.37M chunks), exact-name search is p50 274 ms and p95 774 ms, and behaviour search is p50 561 ms and p95 957 ms. Cost tracks the match count, as #251 described. No held-out repository is that large, so the latency failure is development-only evidence (see Unmeasured strata).
5. **No-result controls are clean.** All 30 no-result calls across both tools returned nothing, and all 1,038 distinct codecite citations read back through `codecite read`.
6. **CodeGraph, for comparison.** Exact definition lookup with `node` scores Hit@1 1.00 on the evaluation repositories and 0.95 on development. Its two Linux misses are extraction misses: it has no node for the `PVSCSIRingReqDesc` struct, nor for `preadv2`, which a `SYSCALL_DEFINE` macro defines. Its `query` command searches symbol names, not text. On behaviour wording it scores Hit@1 0.00 and Hit@10 0.27 (evaluation), and this README does not count that as full-text discovery. It has no path filter, so path cases are `unsupported` for it, not misses. Each call costs 150 to 300 ms on the held-out repositories, mostly process start, and 0.7 to 2.1 s at p50 to p95 on Linux.

### What the evidence points to

The one repeated, generic diagnosis is rank on descriptive queries: the right definition is a candidate but sits a few places down, either because a longer chunk is normalised away or because nearby source shares more words. Per Task 3, this experiment does not change ranking. The candidate it supports is **scoring a definition's header separately from its body**: its signature and doc comment, indexed as their own FTS column or scored as the maximum of header and body. A long class or function would then not be penalised for its body's length, and a query that paraphrases a doc comment would match the field written to describe the code. The rule is about chunk structure, so it names no project, path or language. Before anyone builds it, these acceptance bars should be fixed in advance:

- no frozen history repository loses Recall@1 or MRR;
- exact and qualified Hit@1 on this workload stays at 1.00;
- behaviour Hit@1 improves on a *new* held-out repository, chosen after this round's holdouts move to development.

A latency change, an exact bounded top-k (#251's fourth step), should be judged on a held-out corpus above about 500k chunks. It should be accepted only with ordered-result, score and tie parity, and never counted as a relevance gain.

### Smaller generic defects seen on the way

- `def` orders results by role, then by the citation *string*, so `#L116` sorts before `#L82`. In Keycloak, the `TokenEndpoint` constructor (line 116) is listed before its class (line 82). Sorting by path and then by numeric line fixes it, with no corpus-specific rule.
- Two rules in ranking name a language or paths, against the general-purpose goal. The name-boost threshold has a Java-only clause, `BOOSTED_NAME_COUNT` in `crates/codecite-core/src/store/search.rs`, which counts a same-name method with its class only when `d.language = 'java'`. And role weights are read from path conventions (`tests/`, `examples/`, `docs`). Neither caused a miss here, but both are tuned rules. The constructor clause could be expressed structurally, as a method nested in a class of the same name in any language.

## Split and workload

`corpora.toml` pins five repositories:

| Split | Repository | Language | Domain | Pin | codecite chunks |
| --- | --- | --- | --- | --- | ---: |
| development | Linux | C | kernel | #255's pin | 1,369,065 |
| development | Keycloak | Java | identity server | #255's pin | 116,785 |
| evaluation | Django | Python | web framework | tag 6.0.5 | 59,729 |
| evaluation | ripgrep | Rust | text-search CLI | tag 16.0.0 | 4,839 |
| evaluation | Excalidraw | TypeScript | drawing app | tag v0.18.1 | 7,680 |

The pins and the 105 cases in `cases.json` were committed (`5f02fab`) before any measurement on the evaluation repositories. `run.py validate` rejects all of the following before any CLI is spawned: duplicate IDs, revisions or checkouts off the pin, a repository in both splits, labels pointing at a missing line or a line that doesn't name the symbol, no-result cases that have answers, and scored cases with none.

Each repository gets the same classes: exact name, qualified name (`Type.member`), ambiguous name (unscored; latency and totals only), behaviour (a paraphrase of the definition's doc comment, written before any CLI ran), path-scoped (behaviour with the target's directory as `--path`), and no-result controls. Linux and Keycloak also carry #255's frozen questions as regression data. `sample.py` drew the targets from source alone: a seeded draw of implementation files, one definition per file, kept in draw order unless a person rejected it as test code, generated code or a local closure. `draws.json` records every draw. The labels are paths and lines only. No source text is committed.

## Protocol

- `prepare.sh` builds private codecite and CodeGraph indexes from clean, pinned scratch checkouts (`git clone --shared`, then checkout). codecite's data and cache homes, and CodeGraph's `HOME`, are inside the run directory, so no shared cache or user checkout is touched. Index timings are in `results/index.json`.
- `run.py measure` runs one untimed sweep of all 333 calls, then six timed rounds. Each codecite call is paired with the CodeGraph call that answers the same question (`def`↔`node`, `search`↔`query`). Rounds shuffle the pairs with a fixed seed and alternate A/B and B/A order. Every call is one fresh process with nothing else of ours running, and times are startup-inclusive (the `unshare` wrapper adds the same small cost to both tools). Search uses `--limit 10 --json`. CodeGraph `query` uses `--limit 10 --json`, and `node` prints its default text. The binary was built from `c65b4d8` (`origin/main`).
- `run.py diagnose` scores the untimed sweep. For every codecite miss, and every answer not first, it runs `candidates.rs`, an experiment-only helper compiled against the locked release rlibs. The helper calls the production `Store::search` and `Store::definition` with an untruncated limit, so terms, scope, scoring and tie-breaks are the shipped ones. It reports the rank of every chunk covering the answer, what ranks above it, and the index rows (document, chunks, symbols) at that line. Before trusting it, the runner checks that the helper's top results match what the CLI printed. A missing candidate is never inferred from a capped list. CodeGraph misses are checked against its `nodes` table. Citation round trips run here, outside the timed runs.
- `run.py report` writes `results/`:
  - `per-query.jsonl`: per case, tool and operation, the rank, the outcome class, the cited top 10 (path and lines), the diagnosis and the mechanism;
  - `calls.jsonl`: every call's argv, exit code, byte counts, time and cited rows;
  - `summary.json` and `summary.md`;
  - `metadata.json`: versions, binary and input hashes, pins and load average.

Outcome classes: `hit`; `rank:in-output` (ranks 2 to 10); `rank:below-cut` (a candidate below the cut); `candidate:not-matched` (indexed, but not a candidate); `candidate:lookup-miss` (`def` has the symbol but did not return it); `extraction:not-indexed`; `extraction:not-a-definition`; `unresolved` (could not be proved, including any disagreement between the helper and the CLI); and, apart from those, `unsupported`, `error`, `correct-empty`, `false-positive` and `unscored`. Hit@k and MRR@10 count only scored positive cases.

## Unmeasured strata

These were recorded as unmeasured rather than widened after the fact:

- latency on a held-out corpus above about 100k chunks;
- edge queries (callers, callees, references), since neither tool's edges were in scope;
- free-text discovery for CodeGraph, which has no text search;
- dense or hybrid retrieval;
- sampled behaviour wording other than doc-comment paraphrases.

Five behaviour cases per evaluation repository and six timed samples per call are enough to spot repeated mechanisms, not to estimate tail latency or small quality differences.

## Reproduce

From the repository root, with Python 3.11 or later, Git, `unshare` and the `codegraph` CLI:

```sh
cargo build --release --locked -p codecite
run=$(mktemp -d); src=$run/src; mkdir -p "$src"
# a clean checkout of every repository in corpora.toml, at its pin:
#   git clone --no-checkout <url> "$src/<name>" && git -C "$src/<name>" checkout --detach <commit>
rustc --edition=2024 -O experiments/retrieval-generalization/candidates.rs -L dependency=target/release/deps \
  --extern codecite_core="$(echo target/release/deps/libcodecite_core-*.rlib)" \
  --extern rusqlite="$(echo target/release/deps/librusqlite-*.rlib)" -o "$run/candidates"
experiments/retrieval-generalization/prepare.sh "$run" target/release/codecite "$src"
python3 experiments/retrieval-generalization/run.py measure  --run "$run" --src "$src" --codecite target/release/codecite --rounds 6
python3 experiments/retrieval-generalization/run.py diagnose --run "$run" --src "$src" --codecite target/release/codecite --helper "$run/candidates"
python3 experiments/retrieval-generalization/run.py report   --run "$run" --src "$src"
python3 -m unittest experiments/retrieval-generalization/test_run.py
```

On this host, indexing took about 15 minutes, mostly Linux (codecite 233 s, CodeGraph 584 s and 9.7 GB peak RSS). Measurement took 9 minutes and diagnosis 30 s. Raw stdout and stderr stay in the run directory and are not committed.

## Scaling up and rotating holdouts

- **More repositories:** add a `[[repo]]` to `corpora.toml` with `split = "evaluation"`, run `sample.py <checkout> <name>`, write its cases by the rules above, and commit both *before* measuring. Choose a new domain and language, and at least one large repository (above 500k chunks) to measure latency on held-out data.
- **More cases:** raise `--n` in `sample.py` and take more draws per class, in draw order. Ten or more behaviour cases per repository would allow per-repository intervals.
- **More timing:** raise `--rounds`; keep it even so A/B and B/A are balanced.
- **Rotating:** once a design has been compared on the evaluation repositories, set them to `development` and pick fresh ones for the next comparison. A repository that has been looked at is no longer a holdout. Different questions about the same repository do not make a new holdout.
