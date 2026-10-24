"""Embed every spec chunk and query with each model, and keep each query's top 50
spec chunks by cosine similarity in out/vec-<model>.json.

Usage: uv run --locked python -I embed.py <db> <queries.toml> <model> [<model>...]
Models load only through models.lock.json (see models.py).
"""
import json, sys, time
from pathlib import Path

# -I leaves the script's directory off sys.path; the spike's modules live there.
sys.path.insert(0, str(Path(__file__).parent))

import numpy as np
from fastembed import TextEmbedding

from bench import OUT, queries, spec_chunks
import models

# Per-model query/document prefixes, from each model card.
BGE = ("Represent this sentence for searching relevant passages: ", "")
PREFIX = {
    "BAAI/bge-small-en-v1.5": BGE,
    "BAAI/bge-base-en-v1.5": BGE,
    "snowflake/snowflake-arctic-embed-s": BGE,
    "snowflake/snowflake-arctic-embed-m": BGE,
    "mixedbread-ai/mxbai-embed-large-v1": BGE,
    "nomic-ai/nomic-embed-text-v1.5": ("search_query: ", "search_document: "),
}
# 8k-token models: pad-to-longest at batch 32 needs tens of GB of attention
# scratch, which is what OOM-killed the first run. One chunk at a time is ~4 GB.
LONG_CONTEXT = {"nomic-ai/nomic-embed-text-v1.5"}
TOP = 50


def main(db, queries_path, names):
    qs = queries(queries_path)
    chunks = spec_chunks(db)
    OUT.mkdir(exist_ok=True)
    for name in names:
        qp, dp = PREFIX.get(name, ("", ""))
        slug = name.replace("/", "_")
        m = TextEmbedding(model_name=name, threads=16, specific_model_path=models.path(name))
        cache = OUT / f"emb-{slug}.npy"
        if cache.exists():
            E = np.load(cache)
        else:
            t = time.time()
            batch = 1 if name in LONG_CONTEXT else 32
            E = np.array(list(m.embed([f"{dp}{c['title']}\n\n{c['text']}" for c in chunks],
                                      batch_size=batch)))
            np.save(cache, E)
            print(f"{name}: embedded {len(chunks)} chunks in {time.time() - t:.0f}s")
        E = E / np.linalg.norm(E, axis=1, keepdims=True)
        Q = np.array(list(m.embed([qp + q["text"] for q in qs])))
        Q = Q / np.linalg.norm(Q, axis=1, keepdims=True)
        sims = Q @ E.T
        # Old versions are embedded (the caches hold every row) but never searched.
        sims[:, [i for i, c in enumerate(chunks) if not c["current"]]] = -2
        vec = {q["id"]: [chunks[i]["cite"] for i in np.argsort(-sims[qi])[:TOP]]
               for qi, q in enumerate(qs)}
        (OUT / f"vec-{slug}.json").write_text(json.dumps(vec))


if __name__ == "__main__":
    if len(sys.argv) < 4:
        raise SystemExit(__doc__)
    main(sys.argv[1], sys.argv[2], sys.argv[3:])
