"""Score ways of merging codecite's BM25 results with the vectors from embed.py.

Usage: uv run --locked python -I fusion.py <queries.toml> <codecite binary> [<corpus>]

Every variant but "plain RRF" is code-safe: it starts from codecite's own top
10 and refills only the spec slots, in order, so code results never move.
"""
import glob, json, sys
from pathlib import Path

# -I leaves the script's directory off sys.path; the spike's modules live there.
sys.path.insert(0, str(Path(__file__).parent))

from bench import OUT, cached, header, is_spec, queries, row, rrf, search

W = 40


def refill(base, stream):
    """`base` with each spec slot replaced, in order, by the next unused item of `stream`."""
    stream = iter(stream)
    out = []
    for c in base:
        if is_spec(c):
            try:
                nxt = next(stream)
                while nxt in out:
                    nxt = next(stream)
                out.append(nxt)
            except StopIteration:  # stream ran dry: keep BM25's hit
                if c not in out:
                    out.append(c)
        else:
            out.append(c)
    return out


def main(queries_path, binary, corpus="identity-bench"):
    qs = queries(queries_path)
    cites = lambda *extra: (lambda t: [r["citation"] for r in search(binary, corpus, t, *extra)])
    base = cached("bm25-top10.json", cites("--limit", "10"), qs)
    deep = cached("bm25-top50.json", cites("--limit", "50"), qs)
    spec = cached("bm25-spec50.json", cites("--only", "spec", "--limit", "50"), qs)
    top = cached("bm25-spec-top-score.json",
                 lambda t: next((r["score"] for r in search(binary, corpus, t, "--only", "spec",
                                                            "--limit", "1")), 0.0), qs)
    print(header(W))
    print(row("bm25 (codecite today)", qs, base, W))
    for f in sorted(glob.glob(str(OUT / "vec-*.json"))):
        vec = json.loads(Path(f).read_text())
        name = Path(f).stem[4:]
        print(f"-- {name}")
        variants = {
            # Not code-safe: one RRF over code and spec pushes code down.
            "plain RRF": lambda q: rrf([(deep[q], 1), (vec[q], 1)], 60),
            "spec slots, RRF k=10": lambda q: refill(base[q], rrf([(spec[q], 1), (vec[q], 1)], 10)),
            "spec slots, vectors": lambda q: refill(base[q], vec[q] + spec[q]),
        }
        for keep in (1, 2):
            def kept(q, keep=keep):
                first = [c for c in base[q] if is_spec(c)][:keep]
                rest = rrf([(spec[q], 1), (vec[q], 1)], 10)
                return refill(base[q], first + [c for c in rest if c not in first])
            variants[f"keep BM25 top {keep} spec, RRF"] = kept
        # Vectors only where BM25's best spec match scores under 12. It never
        # fires on plain-words queries: BM25 scores them 15.6-25.8.
        variants["gate: BM25 spec score < 12"] = lambda q: (
            base[q] if top[q] >= 12 else refill(base[q], vec[q] + spec[q]))
        for label, fn in variants.items():
            print(row(f"  {label}", qs, {q["id"]: fn(q["id"]) for q in qs}, W))


if __name__ == "__main__":
    if len(sys.argv) < 3:
        raise SystemExit(__doc__)
    main(*sys.argv[1:])
