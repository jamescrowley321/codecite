#!/usr/bin/env python3
"""Rework on real PRs, read from their blind peer review comments (#179).

Observational only, never a gate. See README.md for what is counted and how.
Stdlib only, so it runs with a bare `python3`.
"""

import argparse
import json
import os
import re
import subprocess
import sys
import tempfile
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


# A heading on a line of its own. A review posted when the PR opens starts with
# it; a re-review is claude[bot]'s progress comment, edited in place, with the
# heading below the "Claude finished" line.
_HEADING = re.compile(
    r"^[ \t]*#{1,4}[ \t]*blind peer review\b(?P<rest>.*)$",
    re.IGNORECASE | re.MULTILINE,
)
_VERDICT = re.compile(r"verdict\W*(PASS|BLOCK)\b", re.IGNORECASE)
_PASS_OR_BLOCK = re.compile(r"\b(PASS|BLOCK)\b", re.IGNORECASE)
_TOTAL = re.compile(r"(\d+) findings?(?:\(s\))?", re.IGNORECASE)
_MUST = re.compile(r"(\d+) MUST FIX", re.IGNORECASE)
_LISTING = re.compile(
    r"^[ \t]*(?P<lens>[A-Z][\w&]*(?: [\w&]+)*)[ \t]{2,}(?P<verdict>PASS|BLOCK)\s+"
    r"(?P<total>\d+) finding\(s\), (?P<must>\d+) MUST FIX",
    re.MULTILINE,
)
_JSON_BLOCK = re.compile(r"```json\s*(\{.*?\})\s*```", re.DOTALL)


def lens_name(text):
    """The canonical name of the lens `text` spells, or None."""
    key = re.sub(r"[*`]", "", text).strip().lower()
    return _ALIASES.get(key)


def is_review(body):
    """True when a line of the comment is a blind peer review heading."""
    return _HEADING.search(body) is not None


def parse_review(body):
    """The lens results in one review comment: a list of LensResult."""
    heading = _HEADING.search(body)
    if heading is None:
        return []
    rest = body[heading.end() :]
    lens = _heading_lens(heading["rest"])
    if lens is not None:
        result = _per_lens(lens, rest)
        return [result] if result else []
    results = _table(rest) or _listing(rest)
    if results:
        return results
    verdict = _VERDICT.search(body, heading.start())
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
            findings = [f for f in findings if isinstance(f, dict)]
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
    """The plan's text at commit `ref`, or None when it isn't there (404).

    Any other error, such as a rate limit, is raised: it says nothing about
    the plan.
    """
    try:
        return gh(
            [
                "api",
                "-H",
                "Accept: application/vnd.github.raw",
                f"repos/{repo}/contents/{path}?ref={ref}",
            ]
        )
    except GhError as err:
        if re.search(r"\b404\b", str(err)):
            return None
        raise


def _read_json(path):
    """The JSON in `path`, or None when it is missing or cut short."""
    try:
        return json.loads(Path(path).read_text())
    except (FileNotFoundError, json.JSONDecodeError):
        return None


def _write_json(path, value):
    """Write `value` to `path` whole or not at all: a temp file, then rename."""
    path = Path(path)
    path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    fd, tmp = tempfile.mkstemp(dir=path.parent, prefix=path.name + ".")
    try:
        with os.fdopen(fd, "w") as f:
            json.dump(value, f)
        os.replace(tmp, path)
    except BaseException:
        os.unlink(tmp)
        raise


def default_cache():
    """The per-user cache directory: $XDG_CACHE_HOME or ~/.cache."""
    base = os.environ.get("XDG_CACHE_HOME") or Path.home() / ".cache"
    return Path(base) / "codecite-rework"


# Merged PRs, 25 a page: one `gh pr list` query for every PR's comments,
# reviews and files times out on the larger repositories. Each connection
# below takes its first 100 nodes; totalCount says when a PR has more, and
# main warns about it. Authors let pr_row drop comments from outsiders.
QUERY = """
query($owner: String!, $name: String!, $endCursor: String) {
  repository(owner: $owner, name: $name) {
    pullRequests(states: MERGED, first: 25, after: $endCursor) {
      nodes {
        number
        title
        headRefOid
        files(first: 100) { totalCount nodes { path } }
        comments(first: 100) {
          totalCount
          nodes { createdAt body author { __typename login } authorAssociation }
        }
        reviews(first: 100) {
          totalCount
          nodes { submittedAt body author { __typename login } authorAssociation }
        }
      }
      pageInfo { hasNextPage endCursor }
    }
  }
}
"""


def _prs(pages):
    """The PRs in `gh api graphql --paginate --slurp` output, nodes unwrapped.

    `truncated` names the connections that had more nodes than were fetched.
    """
    prs = []
    for page in pages:
        for pr in page["data"]["repository"]["pullRequests"]["nodes"]:
            pr["truncated"] = []
            for key in ("files", "comments", "reviews"):
                connection = pr.get(key) or {}
                pr[key] = connection.get("nodes") or []
                if connection.get("totalCount", 0) > len(pr[key]):
                    pr["truncated"].append(key)
            prs.append(pr)
    return prs


def fetch(repo, cache, refresh=False, gh=run_gh, wait=10):
    """The repository's merged PRs, cached as JSON in `cache`.

    A GraphQL page can still time out (HTTP 502/504); the fetch is retried
    twice. Any other error is raised and nothing is cached.
    """
    path = Path(cache) / (repo.replace("/", "__") + ".json")
    cached = None if refresh else _read_json(path)
    if cached is not None:
        return cached
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
    _write_json(path, prs)
    return prs


CITED = "cited plan"
UNCITED = "uncited plan"
NO_PLAN = "no plan"
GROUPS = [CITED, UNCITED, NO_PLAN]


# Anyone can comment on a public PR. A review counts only from a GitHub App
# (the review bot) or from someone the repository itself trusts.
TRUSTED = {"OWNER", "MEMBER", "COLLABORATOR"}


def trusted(node):
    """True when a comment or review comes from a bot or the repository's people."""
    author = node.get("author") or {}
    return author.get("__typename") == "Bot" or node.get("authorAssociation") in TRUSTED


def pr_row(repo, pr, plans):
    """One PR's rework: its group, rounds to pass and violation counts.

    `plans(repo, path, ref)` reads a plan's text at the head, or None.
    `unparsed` counts review headings that gave no result (a run that failed).
    """
    nodes = [(c["createdAt"], c) for c in pr.get("comments") or []]
    nodes += [(r["submittedAt"], r) for r in pr.get("reviews") or []]
    reviews = [
        (at, parse_review(n["body"]))
        for at, n in nodes
        if trusted(n) and is_review(n.get("body") or "")
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
        "unparsed": sum(not results for _, results in reviews),
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
    """A `plans(repo, path, ref)` reader that keeps every answer in `cache`.

    A plan at a commit never changes, and plan_text raises on anything but a
    404, so every cached answer, None included, stays right.
    """
    path = Path(cache) / "plans.json"
    known = _read_json(path) or {}

    def read(repo, plan, ref):
        key = f"{repo}@{ref}:{plan}"
        if key not in known:
            known[key] = plan_text(repo, plan, ref, gh=gh)
            _write_json(path, known)
        return known[key]

    return read


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("repos", nargs="*", default=REPOS, metavar="owner/repo")
    parser.add_argument("--cache", default=default_cache())
    parser.add_argument("--refresh", action="store_true", help="refetch the PRs")
    args = parser.parse_args(argv)
    plans = cached_plans(args.cache)
    rows = []
    for repo in args.repos:
        try:
            prs = fetch(repo, args.cache, refresh=args.refresh)
            rows += [
                pr_row(repo, pr, plans) for pr in sorted(prs, key=lambda p: p["number"])
            ]
        except GhError as err:
            print(f"rework: {repo}: {err}", file=sys.stderr)
            return 1
        for pr in prs:
            if pr.get("truncated"):
                cut = ", ".join(pr["truncated"])
                print(
                    f"rework: {repo}#{pr['number']}: only the first 100 {cut}",
                    file=sys.stderr,
                )
    unparsed = sum(r["unparsed"] for r in rows)
    if unparsed:
        print(f"rework: {unparsed} review comments gave no result", file=sys.stderr)
    sys.stdout.write(report(rows))
    return 0


if __name__ == "__main__":
    sys.exit(main())
