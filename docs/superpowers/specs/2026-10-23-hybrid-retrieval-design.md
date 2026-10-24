# Hybrid retrieval for spec corpora

This answers #95: what codecite should add to BM25 so agents find spec
sections they describe in their own words. It builds on the embeddings spike
(#112, with its scoring fix #115). Every number here comes from scripts in
`experiments/spec-retrieval/`.

## Results

Recall@10 on the retrieval benchmark: 100 labelled queries, `identity-bench`,
codecite at `22391b0`. Every row is code-safe. It starts from codecite's own
top 10 and refills only the spec slots (4.2 of 10 on average), so code results
are the same in every row: symbol 1.00, usage 0.80, behaviour 0.47.

| | spec: plain words (22) | spec: keyword (18) | spec: requirement (15) | all | MRR |
|---|---|---|---|---|---|
| BM25, codecite today | 0.00 | 0.89 | 0.53 | 0.58 | 0.34 |
| **Lexical** | | | | | |
| Porter stemming + query stopwords | 0.00 | 0.89 | 0.67 | 0.60 | 0.35 |
| Porter only | 0.00 | 0.89 | 0.53 | 0.58 | 0.34 |
| Title column (doc title › section path), weight 3, + Porter + stopwords | 0.00 | 0.83 | 0.73 | 0.60 | 0.35 |
| Title column, weight 10, + Porter + stopwords | 0.00 | 0.83 | 0.73 | 0.60 | 0.35 |
| **The agent rewrites the query** (no model in codecite) | | | | | |
| Rewrite only, BM25 as today | 0.86 | 0.83 | 0.80 | 0.80 | 0.42 |
| **Rewrite only, Porter + stopwords** | **0.86** | **0.89** | **0.87** | **0.82** | **0.43** |
| Rewrite and original, RRF k=10, Porter + stopwords | 0.64 | 0.89 | 0.73 | 0.75 | 0.38 |
| **Dense vectors, raw query** | | | | | |
| arctic-embed-m (110M), vectors fill the spec slots | 0.41 | 0.72 | 0.73 | 0.67 | 0.38 |
| bge-base (110M), keep BM25's top 2 spec hits, RRF k=10 | 0.14 | 0.89 | 0.80 | 0.65 | 0.35 |
| all-MiniLM-L6 (23M), keep BM25's top spec hit, RRF k=10 | 0.18 | 0.78 | 0.73 | 0.63 | 0.35 |
| **Cross-encoder over spec candidates** (BM25 top 20 ∪ arctic-m top 20, raw query) | | | | | |
| ms-marco-MiniLM-L-6-v2 (23M) | 0.14 | 0.89 | 0.80 | 0.65 | 0.36 |

What the rows mean:

- **Porter + stopwords** stems a spec's FTS5 table (`tokenize='porter unicode61'`) and drops common English words from the query before it is ORed. "Is client authentication required" no longer matches every chunk that says "is".
- **Title column** adds a fourth FTS5 column holding the document's title, the parent section's title and the section's own title.
- **Agent rewrite** stands in for what the calling agent would do if codecite told it to search specs in their own terms. A separate model saw only the 55 spec query texts, under anonymous ids, never the labels or the corpus. It wrote one 3–10 word query per question in spec vocabulary (`rewrites.json`). Code queries are not rewritten.
- **Code-safe fusion** is the spike's: keep BM25's best spec hit or hits, then reciprocal rank fusion of BM25's and the vectors' spec lists for the remaining spec slots.

## What the numbers say

1. **The gap is vocabulary, and the agent can close it.** BM25 finds 0 of 22 plain-words questions because they share no terms with the section. The best embedding finds 9. A one-line rewrite into spec terms finds 19, with no model, index or storage change in codecite. The caller is an LLM that already knows the domain's vocabulary. codecite only has to ask it to use that vocabulary.
2. **Porter + stopwords is the free lexical win.** Requirement queries go from 0.53 to 0.67 alone and to 0.87 with the rewrite, and the rewrite gets back the keyword query it otherwise loses (0.83 → 0.89). Stemming alone does nothing. The stopwords matter because the query is an OR of every term.
3. **Fusing the rewrite with the original query costs plain-words recall** (0.86 → 0.64). With about four spec slots, the original's lexical noise takes two of them. Searching the rewrite alone beats fusing.
4. **Vectors help only when nothing better is available.** arctic-embed-m is the best embedding, but in the spec slots it gives up three keyword queries. Every code-safe fusion lands at 0.14–0.18 on plain words. With a rewrite, vectors have nothing left to add on this benchmark.
5. **A title column costs a keyword query** and adds nothing a rewrite doesn't. Not adopting it.
6. **A small MS MARCO reranker keeps keyword queries but doesn't find plain-words ones.** 13 of 22 plain-words answers are in its 40-candidate pool, but it puts only 3 in the top slots.

## Not yet measured

These runs were still going, or failed, when this was written. The spike scripts do each of them, and their rows join the table when the runs finish.

- **Contextual headers for embeddings** (`embed.py --context`): the doc title and section path go before the text.
- **Stronger rerankers:** jina-reranker-v1-turbo/tiny, ms-marco-MiniLM-L-12 and bge-reranker-base (`rerank.py`). Reranker latency also needs an idle machine. The MiniLM-L6 run shared 22 cores with three other jobs, so its timings say nothing.
- **Doc-only learned sparse** (`sparse.py`, opensearch-neural-sparse-encoding-doc-v3-gte): OOM-killed at a 12 GB cap with batch 16 on this 8k-context model. It needs batch 1 or truncation to 512 tokens.
- **Porter's effect on code.** The lexical rows refill only spec slots. Stemming the shared `chunk_fts` table would also change code ranking. Either give specs their own FTS5 table or rerun `codecite-bench` on code before shipping.

## Survey

The constraints are a single Rust binary, CPU only, offline after one pinned
fetch, no API at index or query time, and one SQLite file per corpus. No
published benchmark covers RFC or OpenID section retrieval, so the 100 queries
above are the only directly relevant evidence. BEIR figures are nDCG@10 and
don't compare with Recall@10.

| Approach | Needs | Rust path | Published evidence | Verdict |
|---|---|---|---|---|
| Agent rewrites the query (query2doc/HyDE, done by the caller) | Nothing in codecite; tool text that asks for it | MCP tool description and server instructions | BRIGHT: BM25 on GPT-4o reasoning queries 26.5 vs 14.8 nDCG@10 for the raw query ([RaDeR](https://arxiv.org/abs/2505.18405)); query2doc +3–15% for BM25 ([2303.07678](https://arxiv.org/abs/2303.07678)) | **Adopt.** Measured best: plain words 0.86 |
| Porter stemming, query stopwords | An FTS5 tokenizer option | `tokenize='porter unicode61'`; a stopword list in `fts_query` | Standard IR practice | **Adopt for specs.** Requirement 0.53 → 0.67 |
| Title/anchor column, BM25F-style weights | A fourth FTS5 column | `bm25()` column weights ([FTS5](https://www.sqlite.org/fts5.html)) | No RFC numbers | **Skip.** Costs a keyword query |
| Requirement-keyword (MUST/SHALL) boost | `strength` is already stored per chunk | SQL | None published | Not tried on its own. "must" stays in the query and matches MUST in the text; the stopword list drops "shall" and "should", which is worth revisiting |
| Expansion from the spec's own Terminology sections | A gloss table per spec | SQL | None published; only covers defined terms | Low priority; the rewrite does it better |
| RM3 / pseudo-relevance feedback | Nothing new | SQL | Worse on SciFact, 0.679 → 0.646 nDCG@10 ([2510.25488](https://arxiv.org/abs/2510.25488)) | **Skip.** It expands from BM25's own hits, which are wrong for plain-words queries |
| doc2query-- (questions generated at index time, as a low-weight column) | A T5 decoder; about 1–14 CPU-hours per 10–50k sections (estimate) | ort or candle with a hand-written decoding loop; not in fastembed-rs | MS MARCO MRR@10 0.185 → 0.323, in-domain ([2301.03266](https://arxiv.org/abs/2301.03266)) | **Skip** unless a pre-generated, pinned expansion file counts as the one fetch |
| Doc-only learned sparse (opensearch doc-v3-gte, 133M) | Model at index time only; the query side is a tokenizer plus IDF | fastembed-rs `SparseModel::OpenSearchNeuralSparseDocV3Gte`; a `(term, chunk, weight)` side table scored with SUM in SQL | BEIR-13 0.546; SciFact 0.725 vs BM25 0.690 ([model card](https://huggingface.co/opensearch-project/opensearch-neural-sparse-encoding-doc-v3-gte), [2411.04403](https://arxiv.org/abs/2411.04403)) | Unmeasured (OOM). Only worth it if agents don't rewrite |
| SPLADE++ (query-side model) | 110M forward pass per query | fastembed-rs `SPLADEPPV1` | About level with doc-only models on SciFact and NFCorpus ([2411.04403](https://arxiv.org/abs/2411.04403)) | **Skip.** Try doc-only first |
| BGE-M3 sparse | 568M model on both sides | fastembed-rs `Bgem3Embedding` | Re-weights terms already in the text, with no expansion ([2402.03216](https://arxiv.org/abs/2402.03216)) | **Skip.** Wrong mechanism for zero term overlap |
| Dense single-vector (arctic-embed-m, bge-base, MiniLM) | 23–130 MB model; 384–768 floats per chunk | fastembed-rs + sqlite-vec | Measured above | **Defer.** Fallback if agents don't rewrite |
| Late interaction (answerai-colbert-small, 33M) | One vector per token: about 1.4–2.7 GB per 50k sections uncompressed | ort plus hand-written MaxSim; [pylate-rs](https://github.com/lightonai/pylate-rs) | SciFact 0.740 vs bge-small 0.713 ([2510.14880](https://arxiv.org/abs/2510.14880)) | **Skip** as an index. At most a reranker |
| Cross-encoder over the top spec candidates | 23–280 MB model; a forward pass per candidate | fastembed-rs `TextRerank` (bge-reranker-base, jina-turbo) or ort | BEIR rerank gains over BM25; MS MARCO rerankers have hurt technical corpora ([2604.15484](https://arxiv.org/abs/2604.15484)) | MiniLM-L6 measured: keyword safe, no plain-words gain. Larger ones unmeasured |
| Contextual headers (doc title + section path) | Document titles, which codecite doesn't store yet | Index-time string | Anthropic's contextual retrieval cut top-20 failures 35%, but with LLM-written context ([post](https://www.anthropic.com/news/contextual-retrieval)) | Unmeasured; only matters if vectors ship |
| Late chunking, long-context embedders | 8k-context model | — | Flat with nomic ([2409.04701](https://arxiv.org/abs/2409.04701)) | **Skip.** nomic's 8k context didn't help here either |
| Exact grep, `read` of neighbours, several queries per call | Tools | #107 (grep); `read` exists | An indexed retriever beat deep agentic search on repo QA, 65.2% vs 46.2% ([SWE-QA](https://arxiv.org/abs/2608.01507)) | Complements the rewrite: the agent searches in spec terms, then reads |
| Code: SCIP, call graphs (M6), LSP (M7) | Providers | Planned | RepoGraph and related work gain over lexical retrieval ([2410.14684](https://arxiv.org/abs/2410.14684)) | Out of scope; specs come first |

## Recommendation

**Make the agent the query expander, and make spec BM25 forgiving.** No
embeddings for now.

1. **Tell agents to search specs in the spec's own terms.** The MCP `search`
   tool description and the server instructions say this: for a spec question in
   plain words, search with the terms the spec would use (protocol, parameter,
   claim, endpoint and error names), not the user's words. Search the original
   wording too only when it already holds spec terms. Searching the rewrite
   alone measured better than fusing it with the original.
2. **Stem spec text and drop query stopwords.** Do it for spec chunks only,
   until code is measured: a spec FTS5 table with `porter unicode61`, and a
   stopword filter on the spec query.
3. **Keep vectors off by default.** The decisions-table row "Vectors:
   sqlite-vec, off by default" stands. If the A/B eval shows agents pass plain
   words through verbatim despite the guidance, the next arm is arctic-embed-m
   filling the spec slots (plain words 0.41), and contextual headers and a
   stronger reranker get measured first.

## The A/B arms

The Inspect eval (`feat/inspect-eval`) decides adoption, not this table. The
arms to run, each against the no-codecite baseline:

- **A: codecite today** (`22391b0` or later main).
- **B: A + recommendations 1 and 2:** the spec-vocabulary guidance in the tool
  text, plus Porter and stopwords for specs.

B's question is whether real agents rewrite when asked, and whether that shows
up as correctness, tokens or time. Score spec-question tasks separately, since
code tasks shouldn't move.

## Follow-up issues to file

None are filed yet; James decides.

1. **Spec search: Porter stemming and query stopwords.** A spec-only FTS5
   table, or a measurement showing code is unaffected. Add the lexical rows to
   `codecite-bench`.
2. **MCP: guide agents to search specs in spec vocabulary.** The tool
   description and server instructions; arm B of the A/B eval.
3. **Index: store each spec's title** from its `<title>` or first heading.
   Contextual headers and any title-aware ranking need it. The spike uses a
   hand-written table (`titles.py`).
4. **Spike: finish the unmeasured rows.** Contextual headers, sparse at batch
   1, stronger rerankers, and reranker latency on an idle machine.
5. **Conditional: dense vectors for spec chunks** (arctic-embed-m through
   fastembed-rs and sqlite-vec, filling spec slots). File only if arm B shows
   agents don't rewrite.

## Caveats

- **Small buckets.** One plain-words query is 0.045 and one keyword query is
  0.056. Differences of one or two queries are noise.
- **The rewrite is one sample from one strong model.** It is a ceiling for what
  the guidance can deliver, not what every agent will do. The A/B eval measures
  the real thing.
- **The benchmark's plain-words queries are paraphrased on purpose** so they
  share no spec terms. Real questions sit between these and the keyword ones.
- **Code-safe by construction.** Every row keeps code's slots, so no row says
  anything about code ranking. `options.py` asserts it.

## Reproduce

From `experiments/spec-retrieval/` (its README has the setup):

```bash
uv sync --locked
uv run --locked python -I models.py verify
./run-all.sh
```

`run-all.sh` runs every experiment in order and snapshots the results into
`results/` (`table.md`, `per-query.csv`, and every ranked list in `lists/`).

`lexical.py` rebuilds codecite's FTS5 table and reproduces `bm25-spec50.json`
exactly before changing anything.
