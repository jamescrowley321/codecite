"""Embed every spec chunk and query with each model, and keep each query's top 50
spec chunks by cosine similarity in out/vec-<model>.json.

Usage: uv run --locked python -I embed.py [--context] <db> <queries.toml> <model> [<model>...]
Models load only through models.lock.json (see models.py).

Each chunk is embedded as `title\n\ntext`. With --context, the document's title
and a step's section title go in front (`OAuth 2.0 Token Revocation > Revocation
Request\n\n...`), and the outputs are named emb-ctx-*/vec-ctx-*.
"""
import json, sys, time
from pathlib import Path

# -I leaves the script's directory off sys.path; the spike's modules live there.
sys.path.insert(0, str(Path(__file__).parent))

import numpy as np
from fastembed import TextEmbedding

from bench import OUT, queries, spec_chunks
from titles import TITLES
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


def unit(c, context):
    if not context:
        return f"{c['title']}\n\n{c['text']}"
    path = [TITLES.get(c["doc"], c["doc"]), c["parent_title"], c["title"]]
    return " > ".join(t for t in path if t) + "\n\n" + c["text"]


def main(db, queries_path, names, context=False):
    qs = queries(queries_path)
    chunks = spec_chunks(db)
    OUT.mkdir(exist_ok=True)
    for name in names:
        qp, dp = PREFIX.get(name, ("", ""))
        slug = ("ctx-" if context else "") + name.replace("/", "_")
        m = TextEmbedding(model_name=name, threads=16, specific_model_path=models.path(name))
        cache = OUT / f"emb-{slug}.npy"
        if cache.exists():
            E = np.load(cache)
            if len(E) != len(chunks):
                raise SystemExit(f"{cache} has {len(E)} rows but the corpus has {len(chunks)} "
                                 f"spec chunks; delete it and re-embed")
        else:
            t = time.time()
            batch = 1 if name in LONG_CONTEXT else 32
            E = np.array(list(m.embed([dp + unit(c, context) for c in chunks],
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
    args = sys.argv[1:]
    context = args[:1] == ["--context"]
    args = args[context:]
    if len(args) < 3:
        raise SystemExit(__doc__)
    main(args[0], args[1], args[2:], context)
