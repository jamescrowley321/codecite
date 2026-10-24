<!-- Literature and tooling survey for #95, written 2026-10-23 by a research agent
working from the web. Kept verbatim. Its baseline figures (dense alone 0.32,
fusion 0.18 plain / 0.67 requirement) predate the citation fix in #115; the
measured numbers are in docs/superpowers/specs/2026-10-23-hybrid-retrieval-design.md
and results/. Latency figures marked "est." are the agent's estimates, not
measurements. -->

# Retrieval options for codecite spec search: a survey

This survey covers approaches that could raise Recall@10 on **plain-words** spec queries (BM25 0.00, best dense model 0.32, BM25+dense RRF 0.18) without hurting **keyword** queries (BM25 0.89) or **requirement** queries (BM25 0.53, RRF 0.67).

Every approach has to fit codecite's constraints: a single Rust binary, CPU only, offline after one pinned model fetch, no network API at index or query time, and one SQLite file per corpus.

## How to read the numbers

- **BEIR comparisons are nDCG@10. Your gate is Recall@10 on 100 queries.** A BEIR gain of +2 nDCG is noise next to your plain-words gap. Your plain-words bucket is probably 20–35 queries, so one query is about 3–5 recall points. Report confidence intervals or paired bootstrap before you trust a delta under about 0.1.
- **The closest BEIR datasets to RFC prose are SciFact, NFCorpus, TREC-COVID and SciDocs.** They are technical text, but they are not normative standards text. I found **no published benchmark for retrieving RFC, IETF or OpenID sections**, so your 100-query set is the only directly relevant evidence.
- **Plain-words queries at 0.00 mean the query shares no terms with the target section.** Anything that only reorders lexical candidates cannot help them. That rules out PRF/RM3 (which needs initial hits) and cross-encoder reranking of BM25-only candidates. Only three things can help:
  1. Adding vocabulary on the document side: doc expansion, or learned sparse models with expansion terms.
  2. Adding vocabulary on the query side: query rewriting, HyDE, or the agent's own reformulation.
  3. Semantic first-stage retrieval: dense or late interaction.
- **Latency and throughput figures marked "est."** are my own estimates from model size, not measurements. Benchmark them on the target CPU.

---

## 1. Lexical retrieval beyond BM25

### 1a. Section titles and anchor paths as a weighted FTS5 column (BM25F-style)
- **What it needs:** nothing new. FTS5 `bm25(tbl, w1, w2, ...)` takes per-column weights. They scale the per-column phrase frequency, so the result is close to BM25F, though not exactly BM25F, because FTS5 normalises length per column. You can store the weights as the default with `INSERT INTO ft(ft, rank) VALUES('rank','bm25(10.0, 5.0)')`. FTS5 also lets you register custom auxiliary ranking functions through `fts5_api->xCreateFunction`, which rusqlite can call. Source: https://www.sqlite.org/fts5.html
- **Expected effect:** helps keyword and requirement queries whose words appear in a heading, such as "Authorization Response" or "Refreshing an Access Token". It does almost nothing for plain-words queries, because a title is still the spec's own vocabulary.
- **Evidence:** standard BM25F literature. I found no RFC-specific numbers.
- **Cost:** zero. Tune the weights on the keyword and requirement buckets.

### 1b. Query expansion from the corpus's own terminology and definitions sections
- **The idea:** RFCs have a "Terminology" or "Definitions" section, often RFC 2119 boilerplate plus terms like "client", "resource owner", "authorization grant". OpenID specs have a similar section. Build a term-to-definition-gloss table at index time. At query time, if the query's words match a definition's gloss, add the defined term to the query. For example, "the app asking for access" would match the gloss of "client".
- **Rust path:** pure SQL or Rust, using a small FTS5 table over the definition glosses.
- **Evidence:** I found no published numbers for this exact technique. It is a corpus-specific thesaurus, the classic fix for vocabulary mismatch. Its coverage is limited to defined terms; most section topics, like "token replay protection", are not definitions.
- **Verdict:** cheap and worth a try, but expect a modest lift on plain-words queries.

### 1c. RM3 and other pseudo-relevance feedback
- **Evidence is mixed and comes from small effects:**
  - SciFact BM25 0.679 → RM3 0.646 nDCG@10, which is worse (arXiv 2510.25488, https://arxiv.org/pdf/2510.25488).
  - SciFact 69.2 → 68.4 and NFCorpus 29.4 → 32.8 nDCG@20 (arXiv 2603.11008, https://arxiv.org/pdf/2603.11008).
- **Structural problem:** PRF expands from the top-k documents, which are themselves BM25 hits. With zero lexical overlap the feedback documents are wrong, so PRF can't fix plain-words queries. It can also drift keyword queries.
- **A variant worth one test:** dense PRF. Take the top-3 *dense* hits, extract their high-idf terms, and re-query BM25. This is cheap, but the 0.32 dense ceiling bounds it.
- **Verdict:** low priority.

### 1d. Boosts for RFC 2119 requirement keywords
- **The idea:** at index time, tag each sentence or step that contains MUST, MUST NOT, SHALL, SHOULD or REQUIRED. When the query looks like "what must X do", or the MCP caller passes `kind=requirement`, filter to those passages or boost them. You already split specs into steps, so this could be a column or flag on each step.
- **Evidence:** none published. It is a domain heuristic.
- **Why it may still be worth doing:** your requirement bucket is the one where RRF already helps (0.53 → 0.67). Requirement passages are a small fraction of the text, so a filter shrinks the candidate pool a lot.
- **Cost:** zero.

### 1e. doc2query, docT5query and doc2query-- (expansion generated at index time)
- **Numbers:**
  - On MS MARCO, BM25 0.185 → Doc2Query (n=40) 0.277 → Doc2Query-- (n=80, keep the top 30% of queries by ELECTRA score) 0.323 MRR@10.
  - On TREC DL19, 0.499 → 0.626 → 0.670 nDCG@10.
  - Filtering also cut index size by about 33% and query time by about 23%. Source: https://arxiv.org/abs/2301.03266
  - This is in-domain (the T5 model was trained on MS MARCO), so treat it as an upper bound.
  - The SIGIR'24 reproducibility study extends the comparison to 5 BEIR datasets: https://jmmackenzie.io/pdf/mzzm24-sigir.pdf
- **Why it fits:** doc2query is the textbook fix for plain-words queries. Generated questions are in user vocabulary, and they go straight into FTS5 as an extra low-weight column, so query time needs no model.
- **CPU cost (est.):** generation is the expensive part.
  - docT5query took about 40 TPU-hours for 8.8M passages × 5 queries (https://cs.uwaterloo.ca/~jimmylin/publications/Nogueira_Lin_2019_docTTTTTquery-v2.pdf).
  - On a modern 8–16 core CPU with T5-base in ONNX int8, expect about 0.3–1 s per passage for 10 sampled queries.
  - For 10k–50k sections that is roughly 1–14 CPU-hours. That is feasible as a one-time indexing step, but not for incremental indexing on every edit.
- **Rust path:** an encoder-decoder with sampling. ort can run exported T5 ONNX, but you would hand-write the decoding loop. candle has T5. rust-bert has T5 generation via libtorch, which is heavy and hurts the single-binary goal. None of this is in fastembed-rs.
- **Alternative that removes the CPU cost:** the spec corpus (RFCs, OpenID) is public and versioned. You could generate expansions once, offline, with a strong LLM, and ship them as a pinned, checksummed data artifact alongside the model fetch. Whether that counts as "one-time pinned fetch" is a product decision, not a technical one.
- **Mismatch risk:** T5 doc2query was trained on MS MARCO web questions, and its output on dense normative text is untested.

---

## 2. Learned sparse retrieval

### 2a. Doc-only, inference-free models (the most relevant option here)
In these models a BERT-sized encoder runs only over documents, at index time. The query side is a tokenizer plus an IDF lookup table (`idf.json`), so there is no model at query time. Document vectors include **expansion terms**: vocabulary that is not in the passage, which is exactly the mechanism plain-words queries need.

| Model | Params | BEIR-13 avg | SciFact | NFCorpus | TREC-COVID | SciDocs | fastembed-rs |
|---|---|---|---|---|---|---|---|
| opensearch-neural-sparse-encoding-doc-v3-gte | 133M | 0.546 | 0.725 | 0.360 | 0.734 | 0.167 | **yes** (`SparseModel::OpenSearchNeuralSparseDocV3Gte`) |
| opensearch …-doc-v3-distill | 67M | 0.517 | 0.708 | 0.345 | 0.724 | 0.163 | no (use ort directly) |
| opensearch …-doc-v2-distill | 67M | 0.528* | 0.722 | 0.347 | 0.775 | 0.173 | no |
| opensearch …-doc-v2-mini | 23M | 0.497 | 0.699 | 0.336 | 0.709 | 0.164 | no |
| naver/splade-v3-doc | 110M | 0.470 | 0.688 | 0.338 | 0.681 | 0.152 | no |
| BM25 (same paper's table) | – | ~0.44† | 0.690 | 0.327 | 0.688 | 0.165 | – |

\*The v3-distill card's table shows v2-distill above v3-distill, which contradicts the v3 announcement blog (0.504). This is an internal inconsistency in OpenSearch's own reporting.
†BM25 values are from arXiv 2411.04403, Table 4.

Sources:
- https://huggingface.co/opensearch-project/opensearch-neural-sparse-encoding-doc-v3-distill
- https://huggingface.co/opensearch-project/opensearch-neural-sparse-encoding-doc-v3-gte
- https://opensearch.org/blog/advancing-search-with-opensearch-v3-neural-sparse-models-and-a-multilingual-retrieval-model/
- https://arxiv.org/html/2411.04403v1
- https://huggingface.co/naver/splade-v3-doc
- https://docs.rs/fastembed/latest/fastembed/enum.SparseModel.html

**Skeptical read:**
- On scientific BEIR, doc-only sparse beats BM25 by only about 2–4 nDCG points.
- Most of the vocabulary-mismatch benefit shows up on QA-style sets like NQ and FiQA.
- The query side is pure lexical with IDF weights. If the query term never appears in any document's expansion, recall is still zero. Whether "the app" expands to "client" in RFC 6749 is an empirical question. **This is the cheapest high-information experiment on the list.**

**Latency:**
- Query side: the 2411.04403 paper reports about 1.1× BM25 latency (OpenSearch, P99). Query cost is effectively BM25's.
- Index side (est.): a BERT-base-sized model, 133M, on CPU at 256 tokens runs about 15–40 passages/s on 8 cores with ORT fp32. 50k sections would take roughly 20–60 min.
- Disk (est.): about 530 MB fp32 or about 135 MB int8 for v3-gte; about 270 MB fp32 for the 67M models. RAM at index time is about 1 GB.

**Storing it in SQLite:**
- **Side table (recommended):** `sparse(term_id INTEGER, passage_id INTEGER, w REAL)` with a `(term_id)` index. Keep the top ~100–300 terms per passage, since these models are tuned for sparsity. Scoring is plain SQL:
  `SELECT passage_id, SUM(w * :idf_q) FROM sparse WHERE term_id IN (...) GROUP BY passage_id ORDER BY 2 DESC LIMIT 50`.
  Queries have about 5–15 WordPiece terms, so this is a handful of index range scans. That should be milliseconds for 50k passages. Size (est.): 50k × 200 × about 12 bytes is about 120 MB before SQLite overhead. Quantize w to an 8-bit int to shrink it.
- **FTS5 with term repetition (round(w·k) copies):** possible, but FTS5's BM25 then re-applies idf and length normalisation on top of the learned weights. That distorts the learned weights and inflates the index. Avoid it. If you must use FTS5, write a custom auxiliary rank function that reads the weights from a side table.
- **Fusion:** keep BM25 and the learned-sparse score as two lists and fuse them. Do not replace BM25, which protects the 0.89 keyword score.

### 2b. Full SPLADE (query-side model): SPLADE++ ED, SPLADE-v3
- Quality is higher, but you need a 110M forward pass per query on CPU (est. 20–60 ms at short query lengths). That is fine for latency, but it adds query-time model loading.
- fastembed-rs supports `SPLADEPPV1` (`prithivida/Splade_PP_en_v1`).
- The same paper puts inference-free models 3–5 points below siamese SPLADE on BEIR, and the gap on SciFact or NFCorpus is near zero (2411.04403, Table 4). It is not worth the query-time cost before you try 2a.

### 2c. BGE-M3 sparse output
- M3 Sparse beats BM25 on MIRACL (53.9 vs 31.9) and on MLDR, which is long documents (62.2 vs 53.6, English 62.1 vs 57.0). On MIRACL, dense is better (69.2). Source: https://arxiv.org/html/2402.03216
- It needs the 568M XLM-R-large model on *both* sides (est. 2.2 GB fp32, about 150–400 ms per query on CPU).
- Its sparse weights are re-weightings of terms that are already present, with **no expansion**. That makes it no better than BM25 at zero term overlap.
- fastembed-rs supports it (`Bgem3Embedding` returns dense, sparse and ColBERT in one pass).
- **Verdict:** too heavy, and the wrong mechanism for plain-words queries.

---

## 3. Late interaction (ColBERT family)

| Model | Params | Dim | BEIR avg (paper's 18-set) | SciFact | NFCorpus | TREC-COVID | FiQA |
|---|---|---|---|---|---|---|---|
| answerai-colbert-small-v1 | 33M | 96 | 0.534 | 0.740 | 0.369 | 0.831 | 0.410 |
| mxbai-edge-colbert-v0-32m | 32M | 64 | 0.521 | 0.740 | 0.362 | 0.775 | 0.390 |
| mxbai-edge-colbert-v0-17m | 17M | 48 | 0.490 | 0.719 | 0.352 | 0.713 | 0.326 |
| ColBERTv2 | 110M | 128 | 0.488 | 0.693 | 0.338 | 0.733 | 0.356 |
| bge-small-en-v1.5 (dense, ref.) | 33M | 384 | 0.517 | 0.713 | 0.349 | 0.759 | 0.403 |

Sources:
- https://arxiv.org/html/2510.14880 (one consistent evaluation of all rows)
- https://huggingface.co/answerdotai/answerai-colbert-small-v1 (its own card reports slightly different numbers: SciFact 74.77, TREC-COVID 84.59; do not mix the two sources)

- **What it buys:** answerai-colbert-small has the same parameter count as bge-small and scores about +2–7 nDCG on the scientific sets. Late interaction is usually more robust out of domain than single-vector dense.
- **It is still semantic matching.** Your dense models top out at 0.32 on plain-words queries. A +10–20% relative gain (est.) would still leave you around 0.35–0.40.
- **Storage:** one vector per token.
  - fp16 at 96 dims is about 192 B/token. 10k docs × 300 tokens is about 549 MB uncompressed (paper's figure for answerai-small); mxbai-17m at 48 dims is 275 MB.
  - For 50k sections that is about 1.4–2.7 GB, which is too big for "one SQLite file" unless you compress.
  - PLAID residual compression (2-bit) brings ColBERTv2 to about 20–36 B/token. You can also binarize or quantize to 1 bit per dim (12 B/token at 96 dims).
- **Recommended use:** don't index with it. **Use it as a reranker.** Encode the top-50 candidates' tokens at query time (est. 50 × 256 tokens on a 33M model is about 150–400 ms CPU), or cache token embeddings only for spec passages. This competes directly with a cross-encoder (section 4).
- **Rust path:**
  - `lightonai/pylate-rs` runs ColBERT/PyLate inference in Rust (MIT; Rust + WASM; I did not confirm its backend): https://github.com/lightonai/pylate-rs
  - Or export to ONNX and run with ort, doing MaxSim by hand. MaxSim is about 20 lines of Rust.
  - fastembed-rs exposes ColBERT vectors only through BGE-M3, not answerai or mxbai.
  - No PLAID index exists in pure Rust that I could verify. For a reranker you don't need one.

---

## 4. Cross-encoder reranking of the top 50

**Caveat:** a reranker only reorders candidates. Measure **Recall@50 or Recall@100 of (BM25 ∪ dense)** on plain-words queries first. If the gold section isn't in the pool, no reranker helps. Your dense-alone 0.32@10 suggests the union at 50 is meaningfully higher, which is where reranking pays off.

| Model | Params | Max len | BEIR | fastembed-rs |
|---|---|---|---|---|
| ms-marco-MiniLM-L-6-v2 | 22.7M | 512 | 48.64 (17-set, Jina table); 0.481 vs BM25 0.405 (BEIR rerank of BM25 top-100) | no |
| ms-marco-MiniLM-L-12-v2 | 33M | 512 | about +0.5–1 over L-6 (est.) | no |
| jina-reranker-v1-tiny-en | 33M, 4 layers | 8k | 48.54; LoCo 70.29 | no |
| jina-reranker-v1-turbo-en | 37.8M, 6 layers | 8k | 49.60; LoCo 69.21 | **yes** (`JINARerankerV1TurboEn`) |
| mxbai-rerank-xsmall-v1 | 70.8M | 512 | 43.9 vs Lucene 38.0 (11-set) | no (ONNX on HF, run via ort) |
| bge-reranker-base | 278M | 512 | 47.89 (Jina table); 41.6 (mxbai table) | **yes** (`BGERerankerBase`) |
| bge-reranker-v2-m3 | 568M | 8k | stronger, multilingual | **yes** (`BGERerankerV2M3`), but heavy on CPU |
| jina-reranker-v2-base-multilingual | 278M | 1k | – | **yes** |

Sources:
- https://huggingface.co/jinaai/jina-reranker-v1-turbo-en (17-set BEIR and LoCo table)
- https://huggingface.co/mixedbread-ai/mxbai-rerank-xsmall-v1
- https://arxiv.org/pdf/2305.14499 (NAIL; BEIR leaderboard rerank numbers)
- https://docs.rs/fastembed/latest/fastembed/enum.RerankerModel.html

The two tables use different BEIR subsets and first stages, so **do not compare numbers across them**.

**CPU latency per 50 pairs:** I found no trustworthy published CPU measurement. Sources range from about 25 ms to 750 ms for MiniLM-L6. One measured GitHub run gives a 751 ms median for 50 pairs (https://github.com/luc-pimentel/infra-docs-rag/pull/7). My estimates, for 50 pairs at about 256 tokens on 8 cores with ORT:

| Model | Est. latency per 50 pairs |
|---|---|
| MiniLM-L6 | about 80–200 ms |
| jina-turbo (6 layers) | about 100–250 ms |
| MiniLM-L12 / mxbai-xsmall | about 200–400 ms |
| bge-reranker-base | about 0.6–1.5 s |
| v2-m3 | about 2–4 s |

Measure these before choosing.

**Domain-transfer warning:** one paper reports that off-the-shelf MS MARCO cross-encoders *degraded* nDCG by 0.3–3.1% on technical corpora, adding 560–2100 ms (vstash, https://arxiv.org/pdf/2604.15484). RFC text is formal and long, and MS MARCO rerankers see short web passages. **Test against your keyword bucket. A reranker can easily drop the 0.89.** A safe pattern is to rerank only the spec slots and keep the BM25 top-k as a protected floor, or to fuse the reranker score with BM25 rank instead of replacing it.

---

## 5. Chunking and context

- **Anthropic Contextual Retrieval** (https://www.anthropic.com/news/contextual-retrieval). The metric is top-20 failure rate (1 − Recall@20), averaged over codebases, fiction, ArXiv and science papers:

  | Setup | Failure rate | Reduction |
  |---|---|---|
  | Baseline | 5.7% | – |
  | Contextual embeddings | 3.7% | −35% |
  | + contextual BM25 | 2.9% | −49% |
  | + reranking (Cohere, 150 → 20) | 1.9% | −67% |

  - The context was 50–100 tokens per chunk, written by Claude 3 Haiku. That is an LLM at index time, which **violates your constraint**.
  - The "codebases-vague" category reportedly improved least, about 41%, per a secondary critique (https://medium.com/almond-ai/responding-to-anthropics-contextual-retrieval-for-rag-apps-why-context-is-not-all-you-need-af503985aa55). I did not verify this against Anthropic's appendix.
- **The constraint-compatible version is deterministic contextual headers.** Prepend `"RFC 6749 — The OAuth 2.0 Authorization Framework > 4. Obtaining Authorization > 4.1.2 Authorization Response"` to each section before embedding, and to an FTS5 column. Specs have rich, accurate section paths, so this gets much of the benefit without an LLM. I found no published ablation that isolates title or path-only headers. Cost: zero model change, but you re-embed. **Do this before anything else involving embeddings.**
- **Late chunking (Jina)** (https://arxiv.org/html/2409.04701v3):
  - Gains are about +1–4 nDCG on SciFact, NFCorpus and TREC-COVID with jina-v2-small and jina-v3.
  - With nomic-embed-text-v1 it is essentially flat (SciFact 70.7 → 70.6; NFCorpus 35.3 → 35.3).
  - That matches your finding that nomic's 8k context didn't help.
  - It needs a long-context embedder, and the gains concentrate at small chunk sizes. Your sections are already semantically delimited. Low priority.
- **Parent-document and multi-granularity retrieval:** index steps or sentences for matching, and return or cite the parent section. This helps requirement queries, where the gold item is a single MUST sentence. With RRF across granularities, aggregate to the parent with max, or a sum over the top few children. Cheap, no published RFC numbers.
- **LoCo** (https://arxiv.org/pdf/2402.07440): this long-document retrieval benchmark is where chunking hurts. Your corpus is pre-sectioned, so it's not very relevant.

---

## 6. Agentic and iterative search: what helps the agent more than first-stage ranking

The caller is an LLM agent. **Query-side LLM rewriting is free under your constraints**, because the agent's model does it, not codecite. That is the single most important structural point in this survey.

- **BRIGHT** (reasoning-intensive retrieval; https://arxiv.org/html/2407.12883v4):
  - BM25 scores 14.3 nDCG@10 overall. The best retriever scores about 24.3.
  - Using an LLM's reasoning or rewrite as the query helps BM25 the most of any retriever, and LLM-written queries are out of distribution for trained dense models.
  - RaDeR reports BM25 on GPT-4o chain-of-thought queries at 26.5 vs 14.8 for the raw query (https://arxiv.org/pdf/2505.18405).
  - Reproduction note: BRIGHT's BM25 uses query-side BM25 weighting (https://arxiv.org/pdf/2509.02558).
- **query2doc** (https://arxiv.org/abs/2303.07678): appending an LLM-written pseudo-document to the query improves BM25 by 3–15% on MS MARCO and TREC DL, with no training. The largest gains are on hard queries.
- **HyDE** (https://arxiv.org/abs/2212.10496): an LLM writes a hypothetical document, which is then embedded. DL19 nDCG@10 is 61.3 vs BM25 50.6. On TREC-COVID it is a wash (59.3 vs 59.5).
- **What this means for codecite:** in the MCP tool description, instruct the agent to (a) restate the question in the spec's normative vocabulary, or write a one-sentence "what the RFC text would say", and (b) send 2–3 alternative queries. codecite then fuses the results. This targets exactly the plain-words bucket, where the gap is vocabulary. **Your eval harness can simulate it** by having the eval LLM rewrite the 100 queries once, offline.
- **Grep vs vector search for agents:**
  - "Is Grep All You Need?" (https://arxiv.org/html/2605.15184v1): with results inline, grep beat vector search in all 10 harness/model pairs on LongMemEval (116 questions). Claude Code + Opus 4.6 was close, 76.7 vs 75.0. With file-based delivery, vector won 5 of 10.
  - The harness moved accuracy as much as the retriever did.
  - The domain is conversational memory, not specs or code. The paper's own tables are internally inconsistent, and the authors note that paraphrased scientific text may favour dense retrieval.
- **Deep agentic search vs indexed retrieval on repo QA** (SWE-QA; https://arxiv.org/abs/2608.01507): semantic-search retrieval answered 65.2% of questions correctly vs 46.2% for Claude-Code-style planner/subagent deep agentic search, at under half the cost per correct answer. 41.8% of the agentic failures happened at the planner-to-subagent handoff. This is evidence that a good index plus an exact read tool beats pure grep exploration.
- **SWE-bench localization:**
  - BM25 file recall is about 39–41%, Agentless with an LLM is about 66–73%, and fine-tuned code embeddings are about 72%.
  - LocAgent: https://arxiv.org/pdf/2503.09089
  - Nemotron-CORTEXA: https://research.nvidia.com/labs/adlr/cortexa
  - Agent query reformulation over BM25 reports +35% first-file ranking.
- **Practical takeaways:**
  - Return section titles and anchors with every hit so the agent can follow up exactly.
  - Expose read-neighbour and read-section tools (you have `read`).
  - Expose an exact or regex search mode for when the agent knows a term.
  - Accept multiple queries per call.

---

## 7. Code side (summary only)

- **RepoCoder** (https://arxiv.org/abs/2303.12570): iterative retrieve-generate-retrieve gives more than 10% over in-file completion on RepoEval. Gains shrink in repositories with little duplicated code.
- **RepoGraph** (https://arxiv.org/abs/2410.14684), **RepoScope** (call-chain context from static analysis only, https://arxiv.org/html/2507.14791v2) and **GraphCodeAgent** (https://arxiv.org/html/2504.10046v2) use structure (imports, calls, definitions) as retrieval or expansion signals, and all report gains over lexical or dense retrieval.
- **LSP-style tools in agents** (GitHub Next Repo Mind, https://githubnext.com/projects/repo-mind/): on SWE-bench Pro, resolution went from 53.1% to 59.2% *on the instances where agents used the tools*, but agents rarely chose to use them.
- **CoIR** (https://arxiv.org/abs/2407.02883): code-specialised embedders dominate. A later critique notes every CoIR dataset has exactly one relevant document per query.
- **Takeaway for codecite:** a SCIP or tree-sitter symbol table gives definition and reference edges. Use them as (a) a boost for the defining symbol (you already rank definitions first), (b) one-hop expansion of hits to callers and callees as follow-up candidates, and (c) a tool the agent can call explicitly. This is lower priority than spec search, per your goals.

---

## Ranked shortlist for plain-words spec recall without hurting keyword queries

1. **Have the agent reformulate the query, and fuse the results** (query2doc/HyDE-style, done by the caller).
   - Zero model, zero index cost, and fully within the constraints, because codecite never calls an LLM.
   - Strongest published evidence that it helps BM25 specifically: BRIGHT 14.8 → 26.5; query2doc +3–15%.
   - Keyword queries are unaffected if the original query is always one of the fused lists.
   - **Test:** have an LLM rewrite each of the 100 queries into 2–3 spec-vocabulary variants, run BM25 and dense on each, then RRF.
2. **Doc-only learned sparse (opensearch-neural-sparse-encoding-doc-v3-gte, or v3-distill / v2-mini for speed) in a SQL side table, fused with BM25.**
   - The query side is a tokenizer plus an IDF lookup, so no model runs at query time.
   - fastembed-rs supports v3-gte today.
   - Its expansion terms are the right mechanism for zero-overlap queries.
   - Published gains over BM25 on scientific BEIR are small (+2–4 nDCG), so a real test on your data is essential.
   - **Test:** `sentence_transformers.SparseEncoder` with the HF model, encode about 10–50k sections, score with IDF-weighted dot product, and inspect whether plain-words query terms hit the expansions.
3. **Deterministic contextual headers (doc title + section path) for dense embeddings, plus a title/path FTS5 column with bm25() weights.**
   - Trivial cost.
   - Anthropic's contextual-embedding result (−35% failures) is the motivating evidence, though theirs used LLM-written context.
   - The header part should help both dense (plain-words) and BM25 (keyword).
   - **Test:** re-embed with headers using your existing fastembed/sentence-transformers setup.
4. **Rerank the spec slots of a (BM25 ∪ dense ∪ sparse) top-50 pool with a small reranker, fused rather than replacing.**
   - Candidates: bge-reranker-base or jina-reranker-v1-turbo, both in fastembed-rs `TextRerank`, or answerai-colbert-small via ort as a MaxSim reranker.
   - First check pool Recall@50 on plain-words queries. Watch the keyword bucket, because MS MARCO rerankers have hurt technical corpora.
   - **Test:** `sentence_transformers.CrossEncoder` and `fastembed.TextCrossEncoder`, and PyLate for ColBERT; measure CPU ms per 50 pairs.
5. **doc2query-- expansion (generated once and stored as a low-weight FTS5 column).**
   - Strong in-domain evidence (MS MARCO MRR 0.185 → 0.323).
   - Hours of CPU for generation, and an awkward Rust decoder story.
   - Most attractive if you allow a pre-generated, pinned expansion artifact for the public spec corpus.
   - **Test:** `doc2query/msmarco-t5-base-v1` via HF on a 2–5k section sample. Check whether it writes useful questions for normative text before scaling up.

**Not recommended for this goal:**
- RM3/PRF: needs initial lexical hits, and results on SciFact are mixed.
- BGE-M3 sparse: no expansion, and a 568M model at query time.
- Full ColBERT indexing: GB-scale storage per corpus.
- Late chunking: flat with nomic, and your sections are already well delimited.
- Swapping in larger single-vector dense models: you already saw that plateau.

**Cheapest experiment order in Python:**
1. Agent rewrite simulation (no models).
2. Contextual headers re-embed.
3. opensearch doc-v3 sparse encode and score.
4. Measure union Recall@50, then rerank.
5. Doc2query on a sample.

Steps 1–3 can run in one afternoon against the existing 100-query set.
