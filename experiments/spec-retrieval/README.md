# Spec retrieval experiments (#95)

Experiments on finding spec sections that BM25 misses, without costing code
search anything: embeddings (the spike, #112), lexical variants, simulated agent
query rewrites, rerankers and learned sparse expansion. Kept with their data so
they can be re-run or scaled up. The design they back is
`docs/superpowers/specs/2026-10-23-hybrid-retrieval-design.md`.

Corpus: `identity-bench` (`evals/retrieval/identity-bench.toml`), 2,162 spec
chunks. Queries: the 100 labelled queries in `evals/retrieval/queries.toml`,
scored as `codecite-bench` scores them. codecite at `22391b0`.

## What's here

| | |
|---|---|
| `bench.py` | Shared: spec chunks, codecite search, scoring, RRF |
| `embed.py` | Dense embeddings per model; `--context` puts the doc title and section path first |
| `fusion.py` | The spike's fusion variants over codecite's results |
| `lexical.py` | Rebuilds codecite's FTS5 table (reproduces it exactly), then Porter, stopwords, a title column |
| `rewrite.py`, `rewrites.json`, `rewrite-input/` | Simulated agent query rewrites, with the exact prompt and inputs |
| `rerank.py` | Cross-encoders over spec candidates |
| `sparse.py` | Doc-only learned sparse expansion, scored from an inverted index |
| `options.py` | The design's table; asserts code results never move |
| `snapshot.py` | Copies every ranked list to `results/`, with the table and per-query ranks |
| `run-all.sh` | Every step above, in order |
| `titles.py` | Each spec's title (codecite doesn't store it yet) |
| `models.py`, `models.lock.json` | Pinned, hash-checked model downloads |
| `survey.md` | The literature and tooling survey, with sources |
| `results/` | Committed results: `table.md`, `per-query.csv`, `lists/` (every ranked list) |

## Results

Recall@10 by query class. Code results are unchanged in every variant except
plain RRF.

| | spec: plain words (22) | spec: keyword (18) | spec: requirement (15) | code: symbol / usage / behaviour (15 each) | all | MRR |
|---|---|---|---|---|---|---|
| BM25, codecite today | 0.00 | 0.89 | 0.53 | 1.00 / 0.80 / 0.47 | 0.58 | 0.34 |
| snowflake-arctic-embed-m, vectors fill the spec slots | 0.41 | 0.72 | 0.73 | 1.00 / 0.80 / 0.47 | **0.67** | 0.38 |
| bge-base-en-v1.5, keep BM25's top 2 spec hits, RRF for the rest | 0.14 | 0.89 | 0.80 | 1.00 / 0.80 / 0.47 | 0.65 | 0.35 |
| all-MiniLM-L6-v2, keep BM25's top spec hit, RRF for the rest | 0.18 | 0.78 | 0.73 | 1.00 / 0.80 / 0.47 | 0.63 | 0.35 |
| all-MiniLM-L6-v2, plain RRF over code and spec | 0.32 | 0.78 | 0.73 | 0.80 / 0.40 / 0.20 | 0.53 | 0.30 |

- **Vectors are the only thing that finds plain-words spec questions.** BM25 gets 0 of 22. arctic-embed-m in the spec slots gets 0.41, but gives up three keyword queries (0.89 → 0.72).
- **Code-safe fusion trades plain-words for keyword.** A top 10 holds about four spec results, and any slot given to vectors pushes out a BM25 hit. Keeping BM25's top two spec hits keeps keyword at 0.89 but leaves plain words at 0.14.
- **Gating on BM25's score doesn't work.** BM25 scores plain-words queries as high as keyword ones (best spec score 15.6–25.8), so the gate never fires where it's needed.
- **Long context earns nothing.** nomic-embed-text-v1.5 (8,192 tokens) does no better than the 512-token models, and at batch 32 it OOM-killed the machine. The small models are 23–130 MB and embed the corpus in about 3 minutes on a CPU.
- **Citations follow codecite's.** A spec chunk is cited by section number (`#section-3.2.2.9/step-3`), as `search` cites it and the labels name it, and only current document versions are searched. An earlier run cited the page's anchors (`#ImplicitTokenValidation/step-3`) and searched old versions too, so every vector hit in an OpenID spec scored as a miss and the vector rows read low.

`fusion.py` prints every model and variant.

The later experiments' results are in the design spec and `results/table.md`.

## Reproduce

```bash
cargo build --release --locked -p codecite
target/release/codecite index identity-bench --manifest evals/retrieval/identity-bench.toml
cd experiments/spec-retrieval
uv sync --locked
uv run --locked python -I models.py verify     # fetch the pinned models, check every hash
./run-all.sh                                   # every experiment, then snapshot.py
```

`run-all.sh` runs each step under a memory cap, one at a time. Steps cache in
`out/` (gitignored) and skip what is cached; delete a file there to re-run it. The embedding caches (`out/*.npy`,
35 MB) rebuild from the pinned models in about 3 minutes per model; a copy of the
last run's is kept outside the repo in `~/.local/share/codecite-spike/out/`.

Run anything heavy under a memory cap, for example
`systemd-run --user --scope -p MemoryMax=16G -p MemorySwapMax=0 uv run ...`.

## Supply chain

- **Python dependencies:** exact pins in `pyproject.toml`, resolved in `uv.lock`, always run with `--locked`.
- **Models:** pinned in `models.lock.json`, each by Hugging Face commit and the sha256 of every file. `models.py lock` checks each file against the hash the Hub publishes for that commit (the LFS sha256, or the git blob id for small files). Loading downloads only that commit into `~/.cache/codecite-spike/hf` (mode 0700) and refuses any file whose sha256 differs from the lock.
- **Formats:** only ONNX and tokenizer JSON are loaded, with no remote code.

The same rules for codecite itself are #109.
