"""Search codecite with the agent's own rewrite of each spec query, and keep the
merged spec lists in out/rw-<variant>.json.

Usage: uv run --locked python -I rewrite.py <codecite binary> [<corpus>]

rewrites.json holds one rewrite per spec query, written by a model that saw only
the query texts (under anonymous ids), never the labels: what an agent would
type into a keyword search after reading a plain question. The agent calls
search twice, once with each text; code results come from the first call.
"""
import json, sys
from pathlib import Path

# -I leaves the script's directory off sys.path; the spike's modules live there.
sys.path.insert(0, str(Path(__file__).parent))

from bench import HERE, OUT, rrf, search


def main(binary, corpus="identity-bench"):
    rewrites = json.loads((HERE / "rewrites.json").read_text())
    spec = json.loads((OUT / "bm25-spec50.json").read_text())
    cache = OUT / "bm25-rewrite-spec50.json"
    if cache.exists():
        rw = json.loads(cache.read_text())
    else:
        rw = {q: [r["citation"] for r in search(binary, corpus, t, "--only", "spec", "--limit", "50")]
              for q, t in rewrites.items()}
        cache.write_text(json.dumps(rw))
    variants = {
        "only": lambda q: rw[q],
        "RRF k=10 with the original": lambda q: rrf([(spec[q], 1), (rw[q], 1)], 10),
        "original's top 1, then rewrite": lambda q: spec[q][:1] + rw[q],
    }
    for name, fn in variants.items():
        (OUT / f"rw-{name}.json").write_text(json.dumps({q: fn(q) for q in rw}))
        print("wrote", name)


if __name__ == "__main__":
    if len(sys.argv) < 2:
        raise SystemExit(__doc__)
    main(*sys.argv[1:])
