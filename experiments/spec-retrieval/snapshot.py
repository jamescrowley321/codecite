"""Keep every measured result in results/, which is committed, so the analysis
survives a clean checkout and can be re-scored or extended later.

Usage: uv run --locked python -I snapshot.py <queries.toml>

Copies each ranked list in out/ (codecite's own, every variant's spec list:
lex-*, vec-*, rr-*, rw-*) to results/lists/, and writes:
- results/table.md: options.py's table;
- results/per-query.csv: for every query and every option, the rank of the
  first right answer in the code-safe top 10 (blank when none), so a later,
  larger benchmark can be compared query by query.
The embedding caches (out/*.npy, out/emb-*.json) are not copied: they are large
and rebuild from the pinned models; see the README.
"""
import contextlib, csv, io, shutil, sys
from pathlib import Path

# -I leaves the script's directory off sys.path; the spike's modules live there.
sys.path.insert(0, str(Path(__file__).parent))

from bench import DEPTH, HERE, OUT, hit, queries
import options

RESULTS = HERE / "results"


def main(queries_path):
    qs = queries(queries_path)
    lists = RESULTS / "lists"
    lists.mkdir(parents=True, exist_ok=True)
    for p in sorted(OUT.glob("*.json")):
        if not p.name.startswith("emb-"):
            shutil.copy2(p, lists / p.name)
    table = io.StringIO()
    with contextlib.redirect_stdout(table):
        options.main(queries_path, "--markdown")
    (RESULTS / "table.md").write_text(table.getvalue())
    rows = options.rows()
    with open(RESULTS / "per-query.csv", "w", newline="") as f:
        w = csv.writer(f)
        w.writerow(["query", "class", *[name for name, _ in rows]])
        for q in qs:
            ranks = []
            for _, ranked in rows:
                r = next((i + 1 for i, c in enumerate(ranked[q["id"]][:DEPTH])
                          if any(hit(a, c) for a in q["answers"])), "")
                ranks.append(r)
            w.writerow([q["id"], q["class"], *ranks])
    print(f"{len(list(lists.iterdir()))} lists, {len(rows)} options, {len(qs)} queries -> {RESULTS}")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(__doc__)
    main(sys.argv[1])
