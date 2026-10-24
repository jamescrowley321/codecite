| | spec: plain words (22) | spec: keyword (18) | spec: requirement (15) | code: symbol / usage / behaviour | all | MRR |
|---|---|---|---|---|---|---|
| BM25, codecite today | 0.00 | 0.89 | 0.53 | 1.00 / 0.80 / 0.47 | 0.58 | 0.34 |
| lexical: stop | 0.00 | 0.89 | 0.60 | 1.00 / 0.80 / 0.47 | 0.59 | 0.34 |
| lexical: porter | 0.00 | 0.89 | 0.53 | 1.00 / 0.80 / 0.47 | 0.58 | 0.34 |
| lexical: porter-stop | 0.00 | 0.89 | 0.67 | 1.00 / 0.80 / 0.47 | 0.60 | 0.35 |
| lexical: title3 | 0.00 | 0.83 | 0.60 | 1.00 / 0.80 / 0.47 | 0.58 | 0.34 |
| lexical: title10 | 0.00 | 0.83 | 0.60 | 1.00 / 0.80 / 0.47 | 0.58 | 0.33 |
| lexical: title3-porter-stop | 0.00 | 0.83 | 0.73 | 1.00 / 0.80 / 0.47 | 0.60 | 0.35 |
| lexical: title10-porter-stop | 0.00 | 0.83 | 0.73 | 1.00 / 0.80 / 0.47 | 0.60 | 0.35 |
| snowflake_snowflake-arctic-embed-m: vectors fill spec slots | 0.41 | 0.72 | 0.73 | 1.00 / 0.80 / 0.47 | 0.67 | 0.38 |
| snowflake_snowflake-arctic-embed-m: keep BM25 top 1, RRF k=10 | 0.18 | 0.83 | 0.67 | 1.00 / 0.80 / 0.47 | 0.63 | 0.35 |
| BAAI_bge-base-en-v1.5: vectors fill spec slots | 0.36 | 0.50 | 0.73 | 1.00 / 0.80 / 0.47 | 0.62 | 0.36 |
| BAAI_bge-base-en-v1.5: keep BM25 top 1, RRF k=10 | 0.14 | 0.83 | 0.80 | 1.00 / 0.80 / 0.47 | 0.64 | 0.36 |
| thenlper_gte-base: vectors fill spec slots | 0.27 | 0.39 | 0.80 | 1.00 / 0.80 / 0.47 | 0.59 | 0.34 |
| thenlper_gte-base: keep BM25 top 1, RRF k=10 | 0.18 | 0.72 | 0.80 | 1.00 / 0.80 / 0.47 | 0.63 | 0.35 |
| snowflake_snowflake-arctic-embed-s: vectors fill spec slots | 0.27 | 0.72 | 0.80 | 1.00 / 0.80 / 0.47 | 0.65 | 0.37 |
| snowflake_snowflake-arctic-embed-s: keep BM25 top 1, RRF k=10 | 0.14 | 0.78 | 0.73 | 1.00 / 0.80 / 0.47 | 0.62 | 0.35 |
| BAAI_bge-small-en-v1.5: vectors fill spec slots | 0.18 | 0.72 | 0.67 | 1.00 / 0.80 / 0.47 | 0.61 | 0.37 |
| BAAI_bge-small-en-v1.5: keep BM25 top 1, RRF k=10 | 0.14 | 0.83 | 0.73 | 1.00 / 0.80 / 0.47 | 0.63 | 0.36 |
| sentence-transformers_all-MiniLM-L6-v2: vectors fill spec slots | 0.23 | 0.50 | 0.67 | 1.00 / 0.80 / 0.47 | 0.58 | 0.31 |
| sentence-transformers_all-MiniLM-L6-v2: keep BM25 top 1, RRF k=10 | 0.18 | 0.78 | 0.73 | 1.00 / 0.80 / 0.47 | 0.63 | 0.35 |
| rerank Xenova_ms-marco-MiniLM-L-6-v2-lex-baseline-vec-snowflake_snowflake-arctic-embed-m-20 | 0.14 | 0.89 | 0.80 | 1.00 / 0.80 / 0.47 | 0.65 | 0.36 |
| rerank Xenova_ms-marco-MiniLM-L-6-v2-lex-baseline-vec-snowflake_snowflake-arctic-embed-m-20, keep BM25 top 1 | 0.14 | 0.89 | 0.80 | 1.00 / 0.80 / 0.47 | 0.65 | 0.36 |
| agent rewrite RRF k=10 with the original | 0.64 | 0.89 | 0.60 | 1.00 / 0.80 / 0.47 | 0.73 | 0.36 |
| agent rewrite only | 0.86 | 0.83 | 0.80 | 1.00 / 0.80 / 0.47 | 0.80 | 0.42 |
| agent rewrite original's top 1, then rewrite | 0.82 | 0.83 | 0.73 | 1.00 / 0.80 / 0.47 | 0.78 | 0.38 |
| agent rewrite, baseline: only | 0.86 | 0.83 | 0.80 | 1.00 / 0.80 / 0.47 | 0.80 | 0.42 |
| agent rewrite, baseline: RRF k=10 with the original | 0.64 | 0.89 | 0.60 | 1.00 / 0.80 / 0.47 | 0.73 | 0.36 |
| agent rewrite, porter-stop: only | 0.86 | 0.89 | 0.87 | 1.00 / 0.80 / 0.47 | 0.82 | 0.43 |
| agent rewrite, porter-stop: RRF k=10 with the original | 0.64 | 0.89 | 0.73 | 1.00 / 0.80 / 0.47 | 0.75 | 0.38 |
| agent rewrite, title3-porter-stop: only | 0.77 | 0.89 | 0.87 | 1.00 / 0.80 / 0.47 | 0.80 | 0.43 |
| agent rewrite, title3-porter-stop: RRF k=10 with the original | 0.55 | 0.89 | 0.93 | 1.00 / 0.80 / 0.47 | 0.76 | 0.38 |
