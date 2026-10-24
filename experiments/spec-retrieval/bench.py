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
    """Every spec section and step, old versions included, with the citation
    codecite gives it: by section number, as `spec_citation` in search.rs does,
    not by the page's anchor. Rows keep the order the embedding caches were
    written in; search only the `current` ones."""
    con = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
    rows = con.execute("""
        select c.id, s.name, substr(s.revision,1,12), c.kind, c.number, c.anchor,
               p.anchor, coalesce(c.title, p.title, ''), c.text, p.number, s.is_current,
               p.title
        from chunk c join document d on d.id=c.document_id join source s on s.id=d.source_id
        left join chunk p on p.id=c.parent_id
        where s.kind='document'
        order by c.id""").fetchall()
    con.close()
    out = []
    for cid, name, rev, kind, number, _, _, title, text, pnumber, current, ptitle in rows:
        frag = f"section-{pnumber}/step-{number}" if kind == "step" else f"section-{number}"
        out.append(dict(id=cid, cite=f"{name}@sha256:{rev}#{frag}", title=title, text=text,
                        doc=name, parent_title=ptitle if kind == "step" else None,
                        current=bool(current)))
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
    if c[0] != "code" or not frag.startswith("L"):
        return False
    line = int(frag[1:].partition("-L")[0])
    return c[1] == target and c[2][0] <= line <= c[2][1]


def scores(qs, ranked):
    """{class: (Recall@1, Recall@10, MRR)}, plus "all"; a class with no queries scores 0."""
    by = {c: [] for c in CLASSES}
    for q in qs:
        rank = next((i + 1 for i, c in enumerate(ranked[q["id"]][:DEPTH])
                     if any(hit(a, c) for a in q["answers"])), None)
        for key in (q["class"], "all"):
            by.setdefault(key, []).append(rank)
    return {k: (sum(r == 1 for r in v) / len(v), sum(r is not None for r in v) / len(v),
                sum(1 / r for r in v if r) / len(v)) if v else (0, 0, 0) for k, v in by.items()}


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
