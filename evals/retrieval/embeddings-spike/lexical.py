"""Rebuild codecite's full-text index in a scratch database with changes to it, and
keep each query's top 50 spec chunks per variant in out/lex-<variant>.json.

Usage: uv run --locked python -I lexical.py <db> <queries.toml> [<rewrites.json>]

The baseline variant rebuilds codecite's own table (symbol, path, terms; weights
10, 5, 1; every current and old chunk, code included, so BM25's statistics
match) and must reproduce bm25-spec50.json. The others add a title column that
holds the section's title, its parent's and the document's, stem with Porter,
or drop stopwords from the query. With rewrites.json, each spec query is searched
by its rewrite instead (see rewrite.py), into out/lex-<variant>-rw.json.
"""
import json, re, sqlite3, sys
from pathlib import Path

# -I leaves the script's directory off sys.path; the spike's modules live there.
sys.path.insert(0, str(Path(__file__).parent))

from bench import OUT, queries
from titles import TITLES

# Common English words, plus question words agents put in front of a query.
STOP = set("""a an the and or of to in on for with by at from as is are was were be been
being it its this that these those there their they them i me my we our you your he she
his her what which who whom whose when where why how can could should would will shall do
does did done doing has have had having not no if then than so such about into over under
up down out off just only also any all each every some more most other own same very too
s t get got make sure let someone""".split())


def split_identifier(ident):
    """Port of codecite's split_identifier (crates/codecite-core/src/terms.rs)."""
    parts, cur = [], ""
    for i, c in enumerate(ident):
        if not c.isalnum():
            if cur:
                parts.append(cur.lower()); cur = ""
            continue
        if c.isupper() and cur:
            after_lower = not ident[i - 1].isupper()
            ends_acronym = i + 1 < len(ident) and ident[i + 1].islower()
            if after_lower or ends_acronym:
                parts.append(cur.lower()); cur = ""
        cur += c
    if cur:
        parts.append(cur.lower())
    return parts


def identifier_terms(ident):
    parts = split_identifier(ident)
    if len(parts) == 1 and parts[0] == ident:
        return ident
    return " ".join(parts + ([ident] if ident else []))


def text_terms(text):
    words = [w for w in re.split(r"[^\w]", text) if any(ch.isalnum() for ch in w)]
    return " ".join(identifier_terms(w) for w in words)


def fts_query(text, stop=False):
    seen = []
    for t in text_terms(text).lower().split():
        if t not in seen and not (stop and t in STOP):
            seen.append(t)
    return " OR ".join('"' + t.replace('"', '""') + '"' for t in seen)


# name -> (tokenizer, title column?, weights for symbol, path, terms, title, stopwords?)
VARIANTS = {
    "baseline": ("unicode61", False, (10, 5, 1), False),
    "stop": ("unicode61", False, (10, 5, 1), True),
    "porter": ("porter unicode61", False, (10, 5, 1), False),
    "porter-stop": ("porter unicode61", False, (10, 5, 1), True),
    "title3": ("unicode61", True, (10, 5, 1, 3), False),
    "title10": ("unicode61", True, (10, 5, 1, 10), False),
    "title3-porter-stop": ("porter unicode61", True, (10, 5, 1, 3), True),
    "title10-porter-stop": ("porter unicode61", True, (10, 5, 1, 10), True),
}


def rows(src):
    """Every chunk as (id, symbol terms, path terms, terms, title terms, is current spec, cite)."""
    out = []
    for (cid, skind, name, rev, cur, path, kind, number, anchor, title, pnum, panchor, ptitle,
         terms, sym) in src.execute("""
            select c.id, s.kind, s.name, substr(s.revision,1,12), s.is_current, d.path, c.kind,
                   c.number, c.anchor, c.title, p.number, p.anchor, p.title, c.terms,
                   sym.display_name
            from chunk c join document d on d.id=c.document_id join source s on s.id=d.source_id
            left join chunk p on p.id=c.parent_id left join symbol sym on sym.id=c.symbol_id"""):
        spec = skind == "document"
        cite = None
        title_terms = ""
        if spec:
            frag = f"section-{pnum}/step-{number}" if kind == "step" else f"section-{number}"
            cite = f"{name}@sha256:{rev}#{frag}"
            title_terms = text_terms(" ".join(t for t in (TITLES.get(name, ""), ptitle, title) if t))
        out.append((cid, identifier_terms(sym) if sym else "", text_terms(path), terms,
                    title_terms, bool(spec and cur), cite))
    return out


def main(db, queries_path, rewrites=None):
    qs = queries(queries_path)
    suffix = ""
    if rewrites:
        rw = json.loads(Path(rewrites).read_text())
        qs = [dict(q, text=rw[q["id"]]) for q in qs if q["id"] in rw]
        suffix = "-rw"
    src = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
    data = rows(src)
    cite = {r[0]: r[6] for r in data if r[5]}
    OUT.mkdir(exist_ok=True)
    for name, (tok, title, weights, stop) in VARIANTS.items():
        con = sqlite3.connect(":memory:")
        cols = "symbol, path, terms" + (", title" if title else "")
        con.execute(f"create virtual table f using fts5({cols}, tokenize='{tok}')")
        n = 4 + title
        con.executemany(f"insert into f(rowid, {cols}) values ({','.join('?' * n)})",
                        [r[:n] for r in data])
        w = ",".join(str(x) for x in weights)
        ranked = {}
        for q in qs:
            m = fts_query(q["text"], stop)
            hits = con.execute(f"select rowid, -bm25(f, {w}) s from f where f match ?", (m,))
            spec = sorted(((s, rid) for rid, s in hits if rid in cite), key=lambda x: (-x[0], x[1]))
            ranked[q["id"]] = [cite[rid] for _, rid in spec[:50]]
        (OUT / f"lex-{name}{suffix}.json").write_text(json.dumps(ranked))
        print("wrote", name)


if __name__ == "__main__":
    if len(sys.argv) not in (3, 4):
        raise SystemExit(__doc__)
    main(*sys.argv[1:])
