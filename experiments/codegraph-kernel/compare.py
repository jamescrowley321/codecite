#!/usr/bin/env python3
"""Compare the kernel's definitions with codecite's, per repository and
language, and measure the kernel's header fields. See README, "Method".

    python3 -I compare.py <run dir> <src dir> <results dir> [repo...]

Reads <run dir>/<repo>/{codecite.jsonl, kernel.jsonl, kernel-files.jsonl,
*-stats.json, *.time} from run.sh and writes <results dir>/parity.json,
headers.json, cost.json, coverage.json, summary.md and samples.md (the
sampled disagreements and headers with their source lines, for judging).
"""

import collections
import json
import random
import sys
from pathlib import Path

REPOS = ["ripgrep", "django", "excalidraw", "keycloak", "linux"]
SEED = 20261125
# Kernel node kinds that are definitions codecite could also make.
TYPES = {"function", "method", "class", "struct", "interface", "trait", "protocol",
         "enum", "type_alias", "union", "component"}
VARS = {"variable", "constant"}
COMPARABLE = TYPES | VARS
# codecite symbol kinds by bucket
CC_TYPES = {"function", "method", "class", "struct", "enum", "trait", "interface", "type_alias"}
# Kernel language names folded onto codecite's (jsx is JavaScript there).
FOLD = {"jsx": "javascript"}


CODECITE_EXTS = {"rs", "py", "pyi", "ts", "mts", "cts", "tsx", "js", "mjs", "cjs", "jsx",
                 "go", "c", "h", "cs", "java"}


def routed(path):
    """Whether kernel-extract routes the file: codecite also detects a
    shebang file with no extension, which the kernel run skips."""
    name = path.rsplit("/", 1)[-1]
    stem, dot, ext = name.rpartition(".")
    return bool(dot and stem and ext.lower() in CODECITE_EXTS)


def bucket_cc(kind):
    return "types" if kind in CC_TYPES else "vars"


def bucket_k(kind):
    return "types" if kind in TYPES else "vars"


def load_jsonl(path):
    with open(path) as f:
        return [json.loads(line) for line in f]


def match(cc_defs, k_nodes):
    """One-to-one pairs within one (path, name): the kernel node's start line
    must fall inside codecite's chunk (which starts at its doc comment);
    cheapest start-plus-end distance first."""
    cands = []
    for i, c in enumerate(cc_defs):
        for j, k in enumerate(k_nodes):
            if c["s"] <= k["s"] <= c["e"]:
                cands.append((abs(k["s"] - c["s"]) + abs(k["e"] - c["e"]), i, j))
    cands.sort()
    used_c, used_k, pairs = set(), set(), []
    for _, i, j in cands:
        if i in used_c or j in used_k:
            continue
        used_c.add(i)
        used_k.add(j)
        pairs.append((i, j))
    return pairs, used_c, used_k


def header_of(k):
    sig = k.get("sig") or ""
    ret = k.get("ret") or ""
    # Java's signature field starts with the return type ("void (int x)").
    sep = "" if not sig or sig.startswith(("(", "<")) else " "
    head = f'{k["k"]} {k["n"]}{sep}{sig}' + (f" -> {ret}" if ret and ret not in sig else "")
    return head, k.get("doc") or ""


def source_lines(src, repo, path, s, e, limit=6):
    try:
        lines = (Path(src) / repo / path).read_text(errors="replace").splitlines()
    except OSError:
        return "(unreadable)"
    e = min(e, s + limit - 1)
    return "\n".join(f"{n:>6}  {lines[n - 1][:200]}" for n in range(max(1, s), min(e, len(lines)) + 1))


def read_time(path):
    try:
        wall, rss = Path(path).read_text().split()
        return {"wall_seconds": float(wall), "peak_rss_kib": int(rss)}
    except (OSError, ValueError):
        return None


def main():
    run, src, out = Path(sys.argv[1]), sys.argv[2], Path(sys.argv[3])
    repos = sys.argv[4:] or REPOS
    out.mkdir(parents=True, exist_ok=True)
    rng = random.Random(SEED)
    parity, headers, cost, coverage = {}, {}, {}, {}
    pools = collections.defaultdict(list)  # (lang, kind of disagreement) -> examples
    header_pool = collections.defaultdict(list)
    for repo in repos:
        d = run / repo
        cc = load_jsonl(d / "codecite.jsonl")
        kn = [k for k in load_jsonl(d / "kernel.jsonl")]
        bad = {b["p"]: b["why"] for b in load_jsonl(d / "kernel-files.jsonl")}
        kstats = json.loads((d / "kernel-stats.json").read_text())
        cstats = json.loads((d / "codecite-stats.json").read_text())
        mt = json.loads((d / "mt" / "kernel-stats.json").read_text())
        for k in kn:
            k["lang"] = FOLD.get(k["lang"], k["lang"])
        shared_langs = {k for k in kstats["per_language"] if not k.endswith("*")}
        coverage[repo] = {k.rstrip("*"): v["files"] for k, v in kstats["per_language"].items() if k.endswith("*")}

        # Parity, keyed (path, name).
        by_c = collections.defaultdict(list)
        for c in cc:
            by_c[(c["p"], c["n"])].append(c)
        by_k = collections.defaultdict(list)
        other_kinds = collections.Counter()
        for k in kn:
            if k["k"] in COMPARABLE:
                by_k[(k["p"], k["n"])].append(k)
            else:
                other_kinds[(k["lang"], k["k"])] += 1
        rows = collections.defaultdict(lambda: collections.Counter())
        for key in sorted(set(by_c) | set(by_k)):
            cs, ks = by_c.get(key, []), by_k.get(key, [])
            pairs, used_c, used_k = match(cs, ks)
            for i, j in pairs:
                c, k = cs[i], ks[j]
                r = rows[(c["lang"], bucket_cc(c["k"]))]
                r["matched"] += 1
                r["same_end"] += abs(c["e"] - k["e"]) <= 1
            for i, c in enumerate(cs):
                if i in used_c:
                    continue
                r = rows[(c["lang"], bucket_cc(c["k"]))]
                if c["p"] in bad:
                    r["codecite_only_file_deferred"] += 1
                elif not routed(c["p"]):
                    r["codecite_only_file_unrouted"] += 1
                else:
                    r["codecite_only"] += 1
                    pools[(c["lang"], "codecite_only")].append((repo, c))
            for j, k in enumerate(ks):
                if j in used_k:
                    continue
                rows[(k["lang"], bucket_k(k["k"]))]["kernel_only"] += 1
                pools[(k["lang"], "kernel_only")].append((repo, k))
        deferred = collections.Counter()
        for p, why in bad.items():
            deferred[why] += 1
        parity[repo] = {
            "rows": {f"{lang}/{b}": dict(v) for (lang, b), v in sorted(rows.items())},
            "kernel_files": {k: {x: v[x] for x in ("files", "ok", "defer", "timeout", "other", "non_utf8")}
                             for k, v in kstats["per_language"].items()},
            "kernel_other_kinds": {f"{l}/{k}": n for (l, k), n in sorted(other_kinds.items())},
        }

        # Headers: kernel definitions in shared languages, by bucket "types".
        hc = collections.defaultdict(collections.Counter)
        for k in kn:
            if k["k"] not in TYPES or k["lang"] not in shared_langs | {"javascript"}:
                continue
            h = hc[k["lang"]]
            h["definitions"] += 1
            sig = bool(k.get("sig"))
            doc = bool(k.get("doc"))
            h["signature"] += sig
            h["doc"] += doc
            h["both"] += sig and doc
            h["neither"] += not sig and not doc
            header_pool[k["lang"]].append((repo, k))
        headers[repo] = {lang: dict(v) for lang, v in sorted(hc.items())}

        cost[repo] = {
            "codecite_extract": read_time(d / "codecite-extract.time"),
            "codecite_chunk_seconds": cstats["chunk_seconds"],
            "codecite_index": read_time(d / "codecite-index.time"),
            "kernel_1_thread": read_time(d / "kernel-extract.time"),
            "kernel_extract_seconds_shared_langs": sum(v["extract_seconds"] for k, v in kstats["per_language"].items() if not k.endswith("*")),
            "kernel_extract_seconds_all": sum(v["extract_seconds"] for v in kstats["per_language"].values()),
            "kernel_n_threads": read_time(d / "kernel-extract-mt.time") | {"threads": mt["threads"]},
        }

    # Samples for judging.
    lines = ["# Sampled disagreements and headers", "",
             f"Seed {SEED}. Lines are from the pinned checkouts. Verdicts are in judgements.json.", ""]
    picked = {}
    for (lang, kind), pool in sorted(pools.items()):
        sample = rng.sample(pool, min(10, len(pool)))
        picked[f"{lang}/{kind}"] = len(pool)
        lines += [f"## {lang}: {kind} ({len(pool)} in all)", ""]
        for n, (repo, x) in enumerate(sample, 1):
            lines += [f"### {lang}/{kind}/{n}: {repo} `{x['p']}` {x['s']}-{x['e']} {x['k']} `{x['n']}`"
                      + (f" (qn `{x.get('qn')}`)" if x.get("qn") else ""), "", "```",
                      source_lines(src, repo, x["p"], x["s"], x["e"]), "```", ""]
    for lang, pool in sorted(header_pool.items()):
        sample = rng.sample(pool, min(10, len(pool)))
        lines += [f"## {lang}: headers ({len(pool)} in all)", ""]
        for n, (repo, k) in enumerate(sample, 1):
            head, doc = header_of(k)
            lines += [f"### {lang}/header/{n}: {repo} `{k['p']}` {k['s']}", "",
                      f"- header: `{head}`", f"- doc: {json.dumps(doc[:300])}", "", "```",
                      source_lines(src, repo, k["p"], max(1, k["s"] - 3), k["s"] + 2), "```", ""]
    (out / "samples.md").write_text("\n".join(lines))
    for name, obj in [("parity", parity), ("headers", headers), ("cost", cost), ("coverage", coverage)]:
        (out / f"{name}.json").write_text(json.dumps(obj, indent=1, sort_keys=True) + "\n")
    (out / "summary.md").write_text(summary(parity, headers, cost, coverage))
    print(json.dumps({"pools": picked}, indent=1))


def summary(parity, headers, cost, coverage):
    md = ["# Summary tables", "", "Generated by compare.py from the run's outputs.", "",
          "## Definition parity", "",
          "`types` is functions, methods and types; `vars` is variables and constants. "
          "Matched share is of codecite's definitions; the bracket leaves out files the kernel deferred and shebang files it never saw.", "",
          "| Repository | Language | Bucket | Matched | Codecite-only | Codecite-only, file deferred | Codecite-only, shebang file | Kernel-only | Matched share |",
          "|---|---|---|--:|--:|--:|--:|--:|--:|"]
    for r, v in parity.items():
        for k, row in v["rows"].items():
            lang, b = k.split("/")
            m, co = row.get("matched", 0), row.get("codecite_only", 0)
            d, u, ko = row.get("codecite_only_file_deferred", 0), row.get("codecite_only_file_unrouted", 0), row.get("kernel_only", 0)
            tot = m + co + d + u
            share = f"{100 * m / tot:.1f}%" if tot else "-"
            inner = f" ({100 * m / (m + co):.2f}%)" if m + co and m + co != tot else ""
            md.append(f"| {r} | {lang} | {b} | {m} | {co} | {d} | {u} | {ko} | {share}{inner} |")
    md += ["", "## Kernel files", "", "| Repository | Language | Files | Extracted | Deferred | Timed out |", "|---|---|--:|--:|--:|--:|"]
    for r, v in parity.items():
        for lang, f in v["kernel_files"].items():
            md.append(f"| {r} | {lang} | {f['files']} | {f['ok']} | {f['defer']} | {f['timeout']} |")
    md += ["", "A language ending in `*` is one codecite has no grammar for.", "",
           "## Header fields (kernel functions, methods and types)", "",
           "| Repository | Language | Definitions | Signature | Doc | Both | Neither |", "|---|---|--:|--:|--:|--:|--:|"]
    pct = lambda a, b: f"{a} ({100 * a / b:.0f}%)" if b else "0"
    for r, v in headers.items():
        for lang, h in v.items():
            n = h["definitions"]
            md.append(f"| {r} | {lang} | {n} | {pct(h['signature'], n)} | {pct(h['doc'], n)} | {pct(h['both'], n)} | {pct(h['neither'], n)} |")
    md += ["", "## Cost", "",
           "Wall seconds and peak RSS (MiB). codecite-extract walks and chunks with codecite's Chunker, no store; "
           "the kernel runs over the same file list, one thread unless named.", "",
           "| Repository | codecite index | codecite-extract | codecite chunk time | kernel, 1 thread | kernel, all threads |", "|---|--:|--:|--:|--:|--:|"]
    f = lambda t: f"{t['wall_seconds']:.2f} s, {t['peak_rss_kib'] / 1024:.0f} MiB"
    for r, v in cost.items():
        md.append(f"| {r} | {f(v['codecite_index'])} | {f(v['codecite_extract'])} | {v['codecite_chunk_seconds']:.2f} s "
                  f"| {f(v['kernel_1_thread'])} | {f(v['kernel_n_threads'])} ({v['kernel_n_threads']['threads']}) |")
    md += ["", "## Kernel-only languages (files)", ""]
    for r, v in coverage.items():
        md.append(f"- {r}: " + (", ".join(f"{k} {n}" for k, n in sorted(v.items())) or "none"))
    return "\n".join(md) + "\n"


if __name__ == "__main__":
    main()
