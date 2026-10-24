# Embeddings spike (#95)

A throwaway spike, kept so its numbers can be reproduced. It asks whether local
embedding vectors, merged with codecite's BM25 search, find spec sections that
BM25 misses, without costing code search anything.

Corpus: `identity-bench` (`../identity-bench.toml`), 2,162 spec chunks. Queries:
the 100 labelled queries in `../queries.toml`, scored as `codecite-bench` scores
them. codecite at `22391b0`.

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

## Reproduce

```bash
cargo build --release --locked -p codecite
target/release/codecite index identity-bench --manifest evals/retrieval/identity-bench.toml
cd evals/retrieval/embeddings-spike
uv sync --locked
uv run --locked python -I models.py verify     # fetch the pinned models, check every hash
uv run --locked python -I embed.py ~/.local/share/codecite/identity-bench.db ../queries.toml \
  BAAI/bge-small-en-v1.5 sentence-transformers/all-MiniLM-L6-v2 snowflake/snowflake-arctic-embed-s
uv run --locked python -I fusion.py ../queries.toml ../../../target/release/codecite
```

Run anything heavy under a memory cap, for example
`systemd-run --user --scope -p MemoryMax=16G -p MemorySwapMax=0 uv run ...`.

## Supply chain

- **Python dependencies:** exact pins in `pyproject.toml`, resolved in `uv.lock`, always run with `--locked`.
- **Models:** pinned in `models.lock.json`, each by Hugging Face commit and the sha256 of every file. `models.py lock` checks each file against the hash the Hub publishes for that commit (the LFS sha256, or the git blob id for small files). Loading downloads only that commit into `~/.cache/codecite-spike/hf` (mode 0700) and refuses any file whose sha256 differs from the lock.
- **Formats:** only ONNX and tokenizer JSON are loaded, with no remote code.

The same rules for codecite itself are #109.
