#!/usr/bin/env bash
# Every spec retrieval experiment, in order, then a snapshot into results/.
# Usage: ./run-all.sh [<corpus db>]   (from experiments/spec-retrieval, after
# `uv sync --locked` and `uv run --locked python -I models.py verify`)
set -euo pipefail
cd "$(dirname "$0")"
DB=${1:-$HOME/.local/share/codecite/identity-bench.db}
Q=../../evals/retrieval/queries.toml
BIN=../../target/release/codecite
VEC=snowflake/snowflake-arctic-embed-m
DENSE=(BAAI/bge-small-en-v1.5 sentence-transformers/all-MiniLM-L6-v2 snowflake/snowflake-arctic-embed-s
       BAAI/bge-base-en-v1.5 snowflake/snowflake-arctic-embed-m thenlper/gte-base)
RERANKERS=(Xenova/ms-marco-MiniLM-L-6-v2 jinaai/jina-reranker-v1-tiny-en jinaai/jina-reranker-v1-turbo-en
           Xenova/ms-marco-MiniLM-L-12-v2 BAAI/bge-reranker-base)

# Under a memory cap: an uncapped 8k-context model once took the desktop down.
py() { uv run --locked python -I "$@"; }
capped() { systemd-run --user --scope -q -p MemoryMax="$1" -p MemorySwapMax=0 uv run --locked python -I "${@:2}"; }

capped 16G embed.py "$DB" "$Q" "${DENSE[@]}"
capped 16G embed.py --context "$DB" "$Q" "${DENSE[@]}"
py fusion.py "$Q" "$BIN"   # also caches codecite's own results
py lexical.py "$DB" "$Q"
py lexical.py "$DB" "$Q" rewrites.json
py rewrite.py "$BIN"
for m in "${RERANKERS[@]}"; do
  capped 8G rerank.py "$DB" "$Q" "$m" lex-baseline "vec-${VEC//\//_}" 20
done
capped 12G sparse.py "$DB" "$Q" opensearch-project/opensearch-neural-sparse-encoding-doc-v3-gte
py snapshot.py "$Q"
