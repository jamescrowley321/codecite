"""Expand every spec chunk with a learned sparse model and keep each query's top 50
spec chunks by dot product in out/vec-sparse-<model>.json.

Usage: uv run --locked python -I sparse.py <db> <queries.toml> <model> [<model>...]

For the inference-free opensearch doc model only documents go through the
network; a query is its tokens weighted by IDF, so codecite would store
(term, chunk, weight) rows and score queries in SQL with no model at query time.
Chunks are embedded with embed.py's --context unit.
"""
import json, sys, time
from types import SimpleNamespace
from pathlib import Path

# -I leaves the script's directory off sys.path; the spike's modules live there.
sys.path.insert(0, str(Path(__file__).parent))

import numpy as np
from fastembed import SparseTextEmbedding

from bench import OUT, queries, spec_chunks
from embed import unit
import models

TOP = 50


def postings(embs):
    """token -> [(chunk index, weight)]: the side table codecite would keep."""
    index = {}
    for i, e in enumerate(embs):
        for t, w in zip(e.indices.tolist(), e.values.tolist()):
            index.setdefault(t, []).append((i, w))
    return index


def main(db, queries_path, names):
    qs = queries(queries_path)
    chunks = spec_chunks(db)
    current = np.array([c["current"] for c in chunks])
    for name in names:
        slug = "sparse-" + name.replace("/", "_")
        m = SparseTextEmbedding(model_name=name, threads=16, specific_model_path=models.path(name))
        cache = OUT / f"emb-{slug}.json"
        if cache.exists():
            D = [SimpleNamespace(indices=np.array(i, dtype=int), values=np.array(v))
                 for i, v in json.loads(cache.read_text())]
        else:
            t = time.time()
            # One chunk at a time: at batch 16 this 8k-context model was OOM-killed under 12 GB.
            D = list(m.embed([unit(c, True) for c in chunks], batch_size=1))
            cache.write_text(json.dumps([[e.indices.tolist(), e.values.tolist()] for e in D]))
            print(f"{name}: expanded {len(chunks)} chunks in {time.time() - t:.0f}s, "
                  f"{np.mean([len(e.indices) for e in D]):.0f} terms per chunk")
        Q = list(m.query_embed([q["text"] for q in qs]))
        index = postings(D)
        sims = np.zeros((len(qs), len(chunks)))
        for qi, e in enumerate(Q):
            for t, qw in zip(e.indices.tolist(), e.values.tolist()):
                for i, dw in index.get(t, ()):
                    sims[qi, i] += qw * dw
        sims[:, ~current] = -1
        vec = {q["id"]: [chunks[i]["cite"] for i in np.argsort(-sims[qi], kind="stable")[:TOP]]
               for qi, q in enumerate(qs)}
        (OUT / f"vec-{slug}.json").write_text(json.dumps(vec))


if __name__ == "__main__":
    if len(sys.argv) < 4:
        raise SystemExit(__doc__)
    main(sys.argv[1], sys.argv[2], sys.argv[3:])
