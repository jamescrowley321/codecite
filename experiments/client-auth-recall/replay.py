"""Replays the agents' codecite searches from client-auth-uri runs (#170) and
reports where the sections holding RFC 6749's repeated-parameter rule rank.

Usage: python3 -I replay.py <codecite binary> <corpus> [calls.json]
"""
import json, subprocess, sys

# The rule the failing hidden test needs, and where RFC 6749 states it.
TARGETS = {
    "3.1": "MUST NOT be included more than once (authorization endpoint)",
    "3.2": "MUST NOT be included more than once (token endpoint)",
    "5.2": "invalid_request: repeats a parameter",
}
DEPTH = 20


def section(citation):
    if not citation.startswith("rfc6749@"):
        return None
    return citation.rsplit("#section-", 1)[-1]


def search(binary, corpus, args, limit):
    cmd = [binary, "search", corpus, args["query"], "--json", "--limit", str(limit)]
    if args.get("only"):
        cmd += ["--only", args["only"]]
    out = subprocess.run(cmd, capture_output=True, text=True, check=True).stdout
    return [r["citation"] for r in json.loads(out)["results"]]


def main():
    binary, corpus = sys.argv[1], sys.argv[2]
    calls = json.load(open(sys.argv[3] if len(sys.argv) > 3 else "calls.json"))
    seen = set()
    print("| model | passed | limit | query | top results | rank of 3.1 / 3.2 / 5.2 (top 20) |")
    print("|---|---|---|---|---|---|")
    for c in calls:
        if c["tool"] != "search":
            continue
        a = c["args"]
        key = (c["model"], c["passed"], json.dumps(a, sort_keys=True))
        if key in seen:
            continue
        seen.add(key)
        limit = a.get("limit", 5)
        deep = search(binary, corpus, a, DEPTH)
        top = [section(x) or x.split("@")[0] + "…" for x in deep[:limit]]
        ranks = []
        for t in TARGETS:
            hit = [i + 1 for i, x in enumerate(deep) if section(x) == t]
            r = hit[0] if hit else None
            ranks.append(f"**{r}**" if r and r <= limit else str(r or "–"))
        print(f"| {c['model']} | {c['passed']} | {limit} | {a['query'][:70]} | {', '.join(top)} | {' / '.join(ranks)} |")


main()
