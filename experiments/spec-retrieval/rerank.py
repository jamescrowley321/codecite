"""Rerank each query's spec candidates with a cross-encoder, and keep the order in
out/rr-<reranker>-<first>-<second>-<k>.json.

Usage: uv run --locked python -I rerank.py <db> <queries.toml> <reranker> <first> <second> <k>

The candidates are the top <k> of out/<first>.json and of out/<second>.json (two
spec lists, such as lex-baseline and vec-<model>), merged without repeats. The
original query text is what the model scores them against. Each is scored as the
document's title, the section path and the text (embed.py's --context unit).
Only spec candidates are reranked; code never reaches the model.
"""
import json, sys, time
from pathlib import Path

# -I leaves the script's directory off sys.path; the spike's modules live there.
sys.path.insert(0, str(Path(__file__).parent))

import numpy as np
from fastembed.rerank.cross_encoder import TextCrossEncoder

from bench import OUT, queries, spec_chunks
from embed import unit
import models


def main(db, queries_path, reranker, first, second, k):
    k = int(k)
    slug = f"{reranker.replace('/', '_')}-{first}-{second}-{k}"
    if (OUT / f"rr-{slug}.json").exists():
        return print(f"{slug}: cached")
    qs = queries(queries_path)
    text = {c["cite"]: unit(c, True) for c in spec_chunks(db) if c["current"]}
    lex = json.loads((OUT / f"{first}.json").read_text())
    vec = json.loads((OUT / f"{second}.json").read_text())
    m = TextCrossEncoder(model_name=reranker, threads=8, specific_model_path=models.path(reranker))
    out, times = {}, []
    for q in qs:
        cands = list(dict.fromkeys(lex[q["id"]][:k] + vec.get(q["id"], [])[:k]))
        t = time.time()
        s = np.array(list(m.rerank(q["text"], [text[c] for c in cands], batch_size=16)))
        times.append(time.time() - t)
        out[q["id"]] = [cands[i] for i in np.argsort(-s, kind="stable")]
    (OUT / f"rr-{slug}.json").write_text(json.dumps(out))
    print(f"{slug}: median {1000 * np.median(times):.0f} ms, p90 {1000 * np.percentile(times, 90):.0f} ms"
          f" per query, {np.mean([len(v) for v in out.values()]):.0f} candidates")


if __name__ == "__main__":
    if len(sys.argv) != 7:
        raise SystemExit(__doc__)
    main(*sys.argv[1:])
