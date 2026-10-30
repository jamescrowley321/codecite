#!/usr/bin/env python3
"""Rework on real PRs, read from their blind peer review comments (#179).

Observational only, never a gate. See README.md for what is counted and how.
Stdlib only, so it runs with a bare `python3`.
"""

import argparse
import json
import re
import subprocess
import sys
import time
from pathlib import Path
from typing import NamedTuple

# The history benchmark's repositories, all public and the owner's own.
REPOS = [
    "jamescrowley321/blind-peer-review",
    "jamescrowley321/codecite",
    "jamescrowley321/identity-model",
    "jamescrowley321/identity-stack",
    "jamescrowley321/terraform-provider-descope",
]

# The overall result of a review that names no lens.
ALL = "All lenses"

# Canonical lens name -> the other spellings review comments use for it.
LENSES = {
    "Cold Read": ["cold_read"],
    "Edge Cases": ["edge_case", "edge case", "edge_cases"],
    "Acceptance Criteria": ["acceptance"],
    "Security Review": ["security"],
    "Red Team": ["red_team"],
    "Policy & Provenance": ["policy", "policy and provenance"],
    "OWASP Web Top 10": ["owasp_web", "owasp web"],
    "OWASP LLM Top 10": ["owasp_llm", "owasp llm"],
}
_ALIASES = {
    alias: name
    for name, aliases in LENSES.items()
    for alias in [name.lower(), *aliases]
}


class LensResult(NamedTuple):
    """One lens's result in one review. A count the comment doesn't give is None."""

    lens: str
    verdict: str  # "PASS" or "BLOCK"
    must: int | None
    should: int | None
    nit: int | None
    total: int | None  # findings of every severity, when the comment says


_HEADING = re.compile(r"\s*#{1,3}\s*blind peer review\b(?P<rest>.*)", re.IGNORECASE)
_VERDICT = re.compile(r"verdict\W*(PASS|BLOCK)\b", re.IGNORECASE)
_PASS_OR_BLOCK = re.compile(r"\b(PASS|BLOCK)\b", re.IGNORECASE)
_TOTAL = re.compile(r"(\d+) findings?(?:\(s\))?", re.IGNORECASE)
_MUST = re.compile(r"(\d+) MUST FIX", re.IGNORECASE)
_LISTING = re.compile(
    r"^\s*(?P<lens>[A-Z][\w &]+?)\s{2,}(?P<verdict>PASS|BLOCK)\s+"
    r"(?P<total>\d+) finding\(s\), (?P<must>\d+) MUST FIX",
    re.MULTILINE,
)
_JSON_BLOCK = re.compile(r"```json\s*(\{.*?\})\s*```", re.DOTALL)


def lens_name(text):
    """The canonical name of the lens `text` spells, or None."""
    key = re.sub(r"[*`]", "", text).strip().lower()
    return _ALIASES.get(key)


def is_review(body):
    """True when the comment's first line is a blind peer review heading."""
    return _HEADING.match(body.lstrip().split("\n", 1)[0]) is not None


def parse_review(body):
    """The lens results in one review comment: a list of LensResult."""
    first, _, rest = body.lstrip().partition("\n")
    heading = _HEADING.match(first)
    if heading is None:
        return []
    lens = _heading_lens(heading["rest"])
    if lens is not None:
        result = _per_lens(lens, rest)
        return [result] if result else []
    results = _table(rest) or _listing(rest)
    if results:
        return results
    verdict = _VERDICT.search(body)
    if verdict:
        return [LensResult(ALL, verdict[1].upper(), None, None, None, None)]
    return []


def _heading_lens(rest):
    # "— Cold Read", ": Cold Read (#860, round 3)", "— acceptance (codex, …)"
    m = re.match(r"\s*[—:-]\s*([^(]+)", rest)
    return lens_name(m[1]) if m else None


def _per_lens(lens, text):
    block = _JSON_BLOCK.search(text)
    if block:
        try:
            findings = json.loads(block[1]).get("findings")
        except json.JSONDecodeError:
            findings = None
        if isinstance(findings, list):
            severities = [str(f.get("severity", "")).upper() for f in findings]
            must = sum("MUST" in s for s in severities)
            should = sum("SHOULD" in s for s in severities)
            nit = sum("NIT" in s for s in severities)
            verdict = "BLOCK" if must else "PASS"
            return LensResult(lens, verdict, must, should, nit, len(findings))
    for line in text.splitlines():
        verdict = _VERDICT.search(line)
        if verdict:
            total = _TOTAL.search(line)
            must = _MUST.search(line)
            return LensResult(
                lens,
                verdict[1].upper(),
                int(must[1]) if must else None,
                None,
                None,
                int(total[1]) if total else None,
            )
    # No verdict line: "No findings. **PASS**"
    bare = re.findall(r"\*\*(PASS|BLOCK)\*\*", text)
    if bare:
        total = 0 if re.search(r"\bno findings\b", text, re.IGNORECASE) else None
        return LensResult(lens, bare[-1], None, None, None, total)
    return None


def _cells(line):
    return [c.strip() for c in line.strip().strip("|").split("|")]


def _count(cell):
    m = re.search(r"\d+", cell)
    return int(m[0]) if m else None


def _table(text):
    """Rows of a Markdown table whose header has a Lens column."""
    columns = None
    results = []
    for line in text.splitlines():
        if not line.lstrip().startswith("|"):
            if results:
                break
            columns = None
            continue
        cells = _cells(line)
        if columns is None:
            header = [c.lower() for c in cells]
            if header and header[0] == "lens":
                columns = header
            continue
        lens = lens_name(cells[0])
        if lens is None:
            continue
        row = dict(zip(columns, cells))
        result = next(
            (row[c] for c in columns if c in ("result", "verdict", "status")), ""
        )
        verdict = _PASS_OR_BLOCK.search(result)
        if verdict is None:
            continue

        def severity(prefix):
            cell = next((row[c] for c in columns if c.startswith(prefix)), None)
            return None if cell is None else _count(cell)

        total = _TOTAL.search(result)
        results.append(
            LensResult(
                lens,
                verdict[1].upper(),
                severity("must"),
                severity("should"),
                severity("nit"),
                int(total[1]) if total else None,
            )
        )
    return results


def _listing(text):
    """Lines like `  Cold Read   PASS   1 finding(s), 0 MUST FIX`."""
    results = []
    for m in _LISTING.finditer(text):
        lens = lens_name(m["lens"])
        if lens is not None:
            results.append(
                LensResult(
                    lens, m["verdict"], int(m["must"]), None, None, int(m["total"])
                )
            )
    return results


SPEC = "Acceptance Criteria"
CONVENTION = "Policy & Provenance"


def rounds(reviews):
    """Group `(createdAt, [LensResult])` reviews into rounds, oldest first.

    A combined comment is a round of its own. Per-lens comments in a row form
    one round, which ends when a lens repeats.
    """
    out = []
    run = None  # the open per-lens round, if any
    for _, results in sorted(reviews, key=lambda r: r[0]):
        if not results:
            continue
        per_lens = len(results) == 1 and results[0].lens != ALL
        if not per_lens:
            out.append(list(results))
            run = None
        elif run is not None and results[0].lens not in {r.lens for r in run}:
            run.append(results[0])
        else:
            run = [results[0]]
            out.append(run)
    return out


def rounds_to_pass(rounds):
    """The 1-based index of the first round where every lens passed, or None."""
    for i, results in enumerate(rounds, 1):
        if all(r.verdict == "PASS" for r in results):
            return i
    return None


def _findings(result):
    """MUST + SHOULD findings, or the lens's total when neither is given."""
    if result.must is None and result.should is None:
        return result.total or 0
    return (result.must or 0) + (result.should or 0)


def violations(rounds):
    """Spec, convention and all MUST + SHOULD findings across every round."""
    counts = {"spec": 0, "convention": 0, "all": 0}
    for results in rounds:
        for r in results:
            n = _findings(r)
            counts["all"] += n
            if r.lens == SPEC:
                counts["spec"] += n
            elif r.lens == CONVENTION:
                counts["convention"] += n
    return counts


def plan_paths(files):
    """The Markdown files under a `plans/` directory, in the order given."""
    return [f for f in files if f.endswith(".md") and "plans" in f.split("/")[:-1]]


# The citation forms in docs/usage.md: a repository's lines
# `name@<sha>:path#L10-L42`, a document's lines `name@sha256:<hex>#L3-L9`,
# and a spec's section `name@sha256:<hex>#section-4.3`.
_CITATION = re.compile(
    r"(?<![\w.-])[\w.-]+@"
    r"(?:[0-9a-f]{7,64}:[^\s#`()]+#L\d+(?:-L\d+)?"
    r"|sha256:[0-9a-f]{7,64}#(?:L\d+(?:-L\d+)?|section-[\w.-]+))"
)


def has_citation(text):
    """True when `text` holds at least one codecite citation."""
    return _CITATION.search(text) is not None


class GhError(Exception):
    """A `gh` command that failed."""


def run_gh(args):
    """Run `gh` with `args` and return its stdout; raise GhError on failure."""
    proc = subprocess.run(["gh", *args], capture_output=True, text=True)
    if proc.returncode != 0:
        raise GhError(proc.stderr.strip() or f"gh exited {proc.returncode}")
    return proc.stdout


def plan_text(repo, path, ref, gh=run_gh):
    """The plan's text at commit `ref`, or None when it can't be read there."""
    try:
        return gh(
            [
                "api",
                "-H",
                "Accept: application/vnd.github.raw",
                f"repos/{repo}/contents/{path}?ref={ref}",
            ]
        )
    except GhError:
        return None


# Merged PRs, 25 a page: one `gh pr list` query for every PR's comments,
# reviews and files times out on the larger repositories. Each connection
# below takes its first 100 nodes, enough for every PR in these repositories.
QUERY = """
query($owner: String!, $name: String!, $endCursor: String) {
  repository(owner: $owner, name: $name) {
    pullRequests(states: MERGED, first: 25, after: $endCursor) {
      nodes {
        number
        title
        headRefOid
        files(first: 100) { nodes { path } }
        comments(first: 100) { nodes { createdAt body } }
        reviews(first: 100) { nodes { submittedAt body } }
      }
      pageInfo { hasNextPage endCursor }
    }
  }
}
"""


def _prs(pages):
    """The PRs in `gh api graphql --paginate --slurp` output, nodes unwrapped."""
    prs = []
    for page in pages:
        for pr in page["data"]["repository"]["pullRequests"]["nodes"]:
            for key in ("files", "comments", "reviews"):
                pr[key] = (pr.get(key) or {}).get("nodes") or []
            prs.append(pr)
    return prs


def fetch(repo, cache, refresh=False, gh=run_gh, wait=10):
    """The repository's merged PRs, cached as JSON in `cache`.

    A GraphQL page can still time out (HTTP 502/504); the fetch is retried
    twice. Any other error is raised and nothing is cached.
    """
    path = Path(cache) / (repo.replace("/", "__") + ".json")
    if path.exists() and not refresh:
        return json.loads(path.read_text())
    owner, _, name = repo.partition("/")
    args = ["api", "graphql", "--paginate", "--slurp", "-f", f"query={QUERY}"]
    args += ["-F", f"owner={owner}", "-F", f"name={name}"]
    for attempt in range(3):
        try:
            out = gh(args)
            break
        except GhError as err:
            if attempt == 2 or not re.search(r"\b50[234]\b", str(err)):
                raise
            time.sleep(wait)
    prs = _prs(json.loads(out))
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(prs))
    return prs


CITED = "cited plan"
UNCITED = "uncited plan"
NO_PLAN = "no plan"
GROUPS = [CITED, UNCITED, NO_PLAN]


def pr_row(repo, pr, plans):
    """One PR's rework: its group, rounds to pass and violation counts.

    `plans(repo, path, ref)` reads a plan's text at the head, or None.
    """
    reviews = [
        (c["createdAt"], parse_review(c["body"]))
        for c in pr.get("comments") or []
        if is_review(c["body"])
    ]
    reviews += [
        (r["submittedAt"], parse_review(r["body"]))
        for r in pr.get("reviews") or []
        if is_review(r.get("body") or "")
    ]
    grouped = rounds(reviews)
    paths = plan_paths([f["path"] for f in pr.get("files") or []])
    texts = [plans(repo, path, pr["headRefOid"]) for path in paths]
    if not paths:
        group = NO_PLAN
    elif any(t is not None and has_citation(t) for t in texts):
        group = CITED
    else:
        group = UNCITED
    return {
        "repo": repo,
        "number": pr["number"],
        "group": group,
        "reviewed": bool(grouped),
        "unreadable": sum(t is None for t in texts),
        "rounds_to_pass": rounds_to_pass(grouped),
        **violations(grouped),
    }


def _mean(values):
    return f"{sum(values) / len(values):.2f}" if values else "–"


def _line(repo, group, rows):
    reviewed = [r for r in rows if r["reviewed"]]
    passed = [r["rounds_to_pass"] for r in reviewed if r["rounds_to_pass"]]
    cells = [
        repo,
        group,
        str(len(rows)),
        str(len(reviewed)),
        _mean(passed),
        str(len(reviewed) - len(passed)),
        *(_mean([r[k] for r in reviewed]) for k in ("spec", "convention", "all")),
        str(sum(r["unreadable"] for r in rows)),
    ]
    return "| " + " | ".join(cells) + " |"


def report(rows):
    """A Markdown table per repository and group, then a pooled row per group."""
    lines = [
        "| repository | group | PRs | reviewed | mean rounds to pass"
        " | never passed | spec / reviewed PR | convention / reviewed PR"
        " | all MUST+SHOULD / reviewed PR | unreadable plans |",
        "| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |",
    ]
    repos = list(dict.fromkeys(r["repo"] for r in rows))
    for repo in repos:
        for group in GROUPS:
            some = [r for r in rows if r["repo"] == repo and r["group"] == group]
            if some:
                lines.append(_line(repo, group, some))
    for group in GROUPS:
        some = [r for r in rows if r["group"] == group]
        if some:
            lines.append(_line("**all**", group, some))
    return "\n".join(lines) + "\n"


def cached_plans(cache, gh=run_gh):
    """A `plans(repo, path, ref)` reader that keeps every answer in `cache`."""
    path = Path(cache) / "plans.json"
    known = json.loads(path.read_text()) if path.exists() else {}

    def read(repo, plan, ref):
        key = f"{repo}@{ref}:{plan}"
        if key not in known:
            known[key] = plan_text(repo, plan, ref, gh=gh)
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(json.dumps(known))
        return known[key]

    return read


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("repos", nargs="*", default=REPOS, metavar="owner/repo")
    parser.add_argument("--cache", default="/var/tmp/codecite-rework")
    parser.add_argument("--refresh", action="store_true", help="refetch the PRs")
    args = parser.parse_args(argv)
    plans = cached_plans(args.cache)
    rows = []
    for repo in args.repos:
        try:
            prs = fetch(repo, args.cache, refresh=args.refresh)
        except GhError as err:
            print(f"rework: {repo}: {err}", file=sys.stderr)
            return 1
        rows += [
            pr_row(repo, pr, plans) for pr in sorted(prs, key=lambda p: p["number"])
        ]
    sys.stdout.write(report(rows))
    return 0


if __name__ == "__main__":
    sys.exit(main())
