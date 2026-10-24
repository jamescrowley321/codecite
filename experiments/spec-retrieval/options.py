"""Score the hybrid-retrieval options for the design (#95) in one table.

Usage: uv run --locked python -I options.py <queries.toml> [--markdown]

Reads what lexical.py, embed.py, rerank.py and rewrite.py left in out/, after
fusion.py has cached codecite's own results. Every row is code-safe: it starts
from codecite's top 10 and refills only the spec slots, so code never moves.
"""
import json, sys
from pathlib import Path

# -I leaves the script's directory off sys.path; the spike's modules live there.
sys.path.insert(0, str(Path(__file__).parent))

from bench import CLASSES, DEPTH, OUT, hit, is_spec, queries, rrf, scores
from fusion import refill

W = 58


def load(name):
    p = OUT / f"{name}.json"
    return json.loads(p.read_text()) if p.exists() else None


def keep_first(base, stream, keep=1):
    """BM25's best `keep` spec results stay, then `stream`."""
    first = [c for c in base if is_spec(c)][:keep]
    return refill(base, first + [c for c in stream if c not in first])


def rows():
    base = load("bm25-top10")
    spec = load("bm25-spec50")
    out = [("BM25, codecite today", base)]
    ok = lambda *names: all(load(n) is not None for n in names)
    for lex in ("stop", "porter", "porter-stop", "title3", "title10", "title3-porter-stop",
                "title10-porter-stop"):
        if ok(f"lex-{lex}"):
            out.append((f"lexical: {lex}", {q: refill(base[q], l) for q, l in load(f"lex-{lex}").items()}))
    for vec in ("snowflake_snowflake-arctic-embed-m", "ctx-snowflake_snowflake-arctic-embed-m",
                "BAAI_bge-base-en-v1.5", "ctx-BAAI_bge-base-en-v1.5",
                "thenlper_gte-base", "ctx-thenlper_gte-base",
                "snowflake_snowflake-arctic-embed-s", "ctx-snowflake_snowflake-arctic-embed-s",
                "BAAI_bge-small-en-v1.5", "ctx-BAAI_bge-small-en-v1.5",
                "sentence-transformers_all-MiniLM-L6-v2", "ctx-sentence-transformers_all-MiniLM-L6-v2"):
        v = load(f"vec-{vec}")
        if v is None:
            continue
        out.append((f"{vec}: vectors fill spec slots", {q: refill(base[q], v[q] + spec[q]) for q in v}))
        out.append((f"{vec}: keep BM25 top 1, RRF k=10",
                    {q: keep_first(base[q], rrf([(spec[q], 1), (v[q], 1)], 10)) for q in v}))
    for p in sorted(OUT.glob("rr-*.json")):
        r = json.loads(p.read_text())
        out.append((f"rerank {p.stem[3:]}", {q: refill(base[q], r[q] + spec[q]) for q in r}))
        out.append((f"rerank {p.stem[3:]}, keep BM25 top 1", {q: keep_first(base[q], r[q] + spec[q]) for q in r}))
    for p in sorted(OUT.glob("rw-*.json")):
        r = json.loads(p.read_text())
        out.append((f"agent rewrite {p.stem[3:]}", {q: refill(base[q], r.get(q, spec[q])) for q in base}))
    for lex in ("baseline", "porter-stop", "title3-porter-stop"):
        if not ok(f"lex-{lex}", f"lex-{lex}-rw"):
            continue
        orig, rw = load(f"lex-{lex}"), load(f"lex-{lex}-rw")
        out.append((f"agent rewrite, {lex}: only", {q: refill(base[q], rw.get(q, orig[q])) for q in base}))
        out.append((f"agent rewrite, {lex}: RRF k=10 with the original",
                    {q: refill(base[q], rrf([(orig[q], 1), (rw.get(q, []), 1)], 10)) for q in base}))
    return out


def markdown(name, qs, ranked):
    s = scores(qs, ranked)
    cells = [f"{s[c][1]:.2f}" for c in CLASSES[:3]]
    cells.append(" / ".join(f"{s[c][1]:.2f}" for c in CLASSES[3:6]))
    cells += [f"{s['all'][1]:.2f}", f"{s['all'][2]:.2f}"]
    return f"| {name} | " + " | ".join(cells) + " |"


def main(queries_path, *flags):
    qs = queries(queries_path)
    md = "--markdown" in flags
    if md:
        print("| | spec: plain words (22) | spec: keyword (18) | spec: requirement (15) "
              "| code: symbol / usage / behaviour | all | MRR |\n|---|---|---|---|---|---|---|")
    else:
        print(f"{'Recall@10':{W}}" + "".join(f"{c.split('-')[-1][:6]:>8}" for c in CLASSES) + "     MRR")
    base = load("bm25-top10")
    for name, ranked in rows():
        # Code-safe by construction; check it anyway.
        assert all([c for c in ranked[q][:DEPTH] if not is_spec(c)] ==
                   [c for c in base[q][:DEPTH] if not is_spec(c)] for q in base), name
        if md:
            print(markdown(name, qs, ranked))
        else:
            s = scores(qs, ranked)
            print(f"{name:{W}}" + "".join(f"{s[c][1]:>8.2f}" for c in CLASSES) + f"  {s['all'][2]:>6.2f}")


if __name__ == "__main__":
    if len(sys.argv) < 2:
        raise SystemExit(__doc__)
    main(*sys.argv[1:])
