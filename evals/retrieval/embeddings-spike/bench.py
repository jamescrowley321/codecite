"""Shared pieces: spec chunks from a built corpus, codecite search, and the scoring
codecite-bench uses (Recall@1, Recall@10 and MRR per query class)."""
import json, sqlite3, subprocess, tomllib
from pathlib import Path

HERE = Path(__file__).parent
OUT = HERE / "out"
DEPTH = 10
CLASSES = ["spec-plain", "spec-keyword", "spec-requirement", "code-symbol", "code-usage",
           "code-behaviour", "all"]


def queries(path):
    return tomllib.loads(Path(path).read_text())["query"]


def spec_chunks(db):
    """Every spec section and step, with the citation codecite gives it."""
    con = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
    rows = con.execute("""
        select c.id, s.name, substr(s.revision,1,12), c.kind, c.number, c.anchor,
               p.anchor, coalesce(c.title, p.title, ''), c.text
        from chunk c join document d on d.id=c.document_id join source s on s.id=d.source_id
        left join chunk p on p.id=c.parent_id
        where s.kind='document'""").fetchall()
    out = []
    for cid, name, rev, kind, number, anchor, panchor, title, text in rows:
        frag = anchor if kind == "section" else f"{panchor}/step-{number}"
        out.append(dict(id=cid, cite=f"{name}@sha256:{rev}#{frag}", title=title, text=text))
    return out


def search(binary, corpus, text, *extra):
    r = subprocess.run([binary, "search", corpus, text, "--json", *extra],
                       capture_output=True, text=True, check=True)
    return json.loads(r.stdout)["results"]


def cached(name, fn, qs):
    """`fn(query text)` for every query, kept in out/<name> after the first run."""
    p = OUT / name
    if p.exists():
        return json.loads(p.read_text())
    out = {q["id"]: fn(q["text"]) for q in qs}
    OUT.mkdir(exist_ok=True)
    p.write_text(json.dumps(out))
    return out


def parse_cite(c):
    target, _, frag = c.partition("#")
    name, _, rev = target.partition("@")
    if rev.startswith("sha256:"):
        return ("spec", name, frag)
    _, _, path = rev.partition(":")
    if frag.startswith("L"):
        a, _, b = frag[1:].partition("-L")
        return ("code", path, (int(a), int(b or a)))
    return ("other",)


def is_spec(c):
    return parse_cite(c)[0] == "spec"


def hit(answer, cite):
    c = parse_cite(cite)
    target, _, frag = answer.partition("#")
    if frag.startswith("section-"):
        return c[0] == "spec" and c[1] == target and (c[2] == frag or c[2].startswith(frag + "/"))
    line = int(frag[1:])
    return c[0] == "code" and c[1] == target and c[2][0] <= line <= c[2][1]


def scores(qs, ranked):
    """{class: (Recall@1, Recall@10, MRR)}, plus "all"."""
    by = {}
    for q in qs:
        rank = next((i + 1 for i, c in enumerate(ranked[q["id"]][:DEPTH])
                     if any(hit(a, c) for a in q["answers"])), None)
        for key in (q["class"], "all"):
            by.setdefault(key, []).append(rank)
    return {k: (sum(r == 1 for r in v) / len(v), sum(r is not None for r in v) / len(v),
                sum(1 / r for r in v if r) / len(v)) for k, v in by.items()}


def rrf(weighted, k):
    """Reciprocal rank fusion over [(ranked list, weight)]."""
    s = {}
    for ranked, w in weighted:
        for i, c in enumerate(ranked):
            s[c] = s.get(c, 0) + w / (k + i + 1)
    return sorted(s, key=lambda c: -s[c])


def header(width):
    return f"{'Recall@10':{width}}" + "".join(f"{c.split('-')[-1][:6]:>8}" for c in CLASSES) + "     MRR"


def row(name, qs, ranked, width):
    s = scores(qs, ranked)
    return f"{name:{width}}" + "".join(f"{s[c][1]:>8.2f}" for c in CLASSES) + f"  {s['all'][2]:>6.2f}"
