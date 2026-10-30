"""Mine merged PRs into the history benchmark's frozen queries.

Each merged PR is a labelled query whose answer nobody wrote by hand: the
question is the PR's title (`history-title`) and, when it closes an issue,
the issue as it was asked (`history-issue`); the answer is the files the PR
modified that exist at the repository's pinned commit, the parent of the
oldest sampled PR. See docs/design.md, Evaluation plan.

    uv run codecite-history-mine OWNER/REPO --clone DIR [--max 40] [--out DIR]

writes `<out>/<repo>/queries.toml` and `<out>/<repo>/manifest.toml`. The
files are frozen once committed: remining writes a new version.
"""

import argparse
import json
import re
import subprocess
from dataclasses import dataclass, field
from datetime import date
from pathlib import Path
from typing import Callable

# Bump when the mining rules change, so a remined file says it is new.
# 2: release PRs and hub files left out.
VERSION = 3

LOCKFILES = {"Cargo.lock", "uv.lock", "poetry.lock", "package-lock.json", "pnpm-lock.yaml", "yarn.lock", "go.sum"}
BOTS = {"dependabot", "renovate", "github-actions"}
ISSUE_TEXT_LIMIT = 400
# A PR touching more files says little about where one change belongs, and
# almost any search would hit one of them.
MAX_ANSWERS = 20
# A file more than this share of the sampled PRs changed (a design doc, a
# manifest) answers so many queries that naming it beats any search.
MAX_ANSWER_SHARE = 0.25
# ... and at least this many, so a small sample's repeats aren't hubs.
MIN_HUB_PRS = 3
# release-please and similar: a version bump, not a change anyone asked for.
RELEASE = re.compile(r"^chore(\([^)]*\))?!?: release\b", re.IGNORECASE)

# A Markdown heading line (`## Problem`, not `#123`), and an HTML comment,
# as issue templates leave them.
HEADING = re.compile(r"^#+\s")
COMMENT = re.compile(r"<!--.*?-->", re.DOTALL)


@dataclass
class Change:
    """One file a PR touched, as `git diff --name-status -M` lists it."""

    status: str  # M, A, D, R (rename), C (copy), T (type change)
    path: str
    old_path: str | None = None


@dataclass
class PullRequest:
    number: int
    title: str
    url: str
    author: str
    parent: str  # the merge commit's first parent: the base before the PR
    changes: list[Change] = field(default_factory=list)
    issue: str | None = None  # issue_text() of the first issue it closes


@dataclass
class Mined:
    number: int
    url: str
    title: str
    issue: str | None
    answers: list[str]


def is_bot(login: str) -> bool:
    # gh names a GitHub App's PRs `app/<name>`: `app/dependabot`.
    return login.endswith("[bot]") or login.startswith("app/") or login in BOTS


def candidate_paths(changes: list[Change]) -> list[str]:
    """The paths a PR's changes could answer with, before the existence check."""
    paths = []
    for change in changes:
        if change.status == "M" or change.status == "T":
            path = change.path
        elif change.status == "R" and change.old_path:
            path = change.old_path
        else:
            continue
        name = path.rsplit("/", 1)[-1]
        if name in LOCKFILES or name.startswith("CHANGELOG"):
            continue
        # codecite-bench reads a bare label without `/` or `.` as a spec
        # label missing its section, not as a file.
        if "/" not in path and "." not in path:
            continue
        # It splits a label at `#`, so such a path can't be labelled.
        if "#" in path:
            continue
        if path not in paths:
            paths.append(path)
    return paths


def eligible(p: PullRequest) -> bool:
    """A PR by a person with between 1 and MAX_ANSWERS candidate answers."""
    return (
        not is_bot(p.author)
        and not RELEASE.match(p.title)
        and 0 < len(candidate_paths(p.changes)) <= MAX_ANSWERS
    )


def select_prs(prs: list[PullRequest], exists: Callable[[str, str], bool], max_prs: int) -> tuple[str, list[Mined]]:
    """The pinned commit and the mined PRs, newest first, from `prs` newest first.

    The newest `max_prs` eligible PRs are sampled, the commit pinned is the
    oldest one's parent, and each keeps only the answers that exist there
    and that no more than MAX_ANSWER_SHARE of the sample changed; a PR left
    with none is dropped.
    """
    sampled = [p for p in prs if eligible(p)][:max_prs]
    if not sampled:
        raise SystemExit("no eligible merged PRs")
    commit = sampled[-1].parent
    changed_by: dict[str, int] = {}
    for p in sampled:
        for path in candidate_paths(p.changes):
            changed_by[path] = changed_by.get(path, 0) + 1
    hubs = {path for path, n in changed_by.items() if n >= MIN_HUB_PRS and n > MAX_ANSWER_SHARE * len(sampled)}
    mined = []
    for p in sampled:
        answers = [path for path in candidate_paths(p.changes) if path not in hubs and exists(commit, path)]
        if answers:
            mined.append(Mined(p.number, p.url, p.title, p.issue, answers))
    return commit, mined


def issue_text(title: str, body: str) -> str:
    """An issue as asked: its title, a blank line, and the first paragraph of
    its body that isn't a heading or a comment, at most 400 characters and
    cut at a word."""
    body = COMMENT.sub("", (body or "").replace("\r\n", "\n"))
    for paragraph in body.split("\n\n"):
        lines = [line for line in paragraph.split("\n") if not HEADING.match(line)]
        text = " ".join(" ".join(lines).split())
        if text:
            break
    else:
        return title
    full = f"{title}\n\n{text}"
    if len(full) <= ISSUE_TEXT_LIMIT:
        return full
    window = full[: ISSUE_TEXT_LIMIT + 1]
    if " " not in window:
        return full[:ISSUE_TEXT_LIMIT]
    return window.rsplit(" ", 1)[0].rstrip()


def basic(value: str) -> str:
    """A TOML basic string: JSON's escapes are all valid TOML."""
    return json.dumps(value, ensure_ascii=False)


def queries_toml(repo: str, commit: str, mined: list[Mined], mined_on: str) -> str:
    name = repo.split("/")[1]
    lines = [
        f"# History benchmark queries for {repo}, mined from merged PRs by",
        "# `codecite-history-mine` (evals/src/codecite_evals/history.py). Frozen:",
        "# never edit by hand; remining writes a new version.",
        "#",
        f"# version: {VERSION}",
        f"# commit: {commit}",
        f"# mined: {mined_on}",
        "",
        f"corpus = {basic('history-' + name)}",
    ]
    for m in mined:
        for cls, text in (("title", m.title), ("issue", m.issue)):
            if text is None:
                continue
            answers = ", ".join(basic(a) for a in m.answers)
            lines += [
                "",
                "[[query]]",
                f"id = {basic(f'{name}-pr{m.number}-{cls}')}",
                f"class = {basic('history-' + cls)}",
                f"text = {basic(text)}",
                f"answers = [{answers}]",
                f"source = {basic(m.url)}",
            ]
    return "\n".join(lines) + "\n"


def manifest_toml(repo: str, commit: str, license: str | None) -> str:
    name = repo.split("/")[1]
    lines = [
        f"# The history benchmark's corpus for {repo}: the repository at the parent",
        "# of the oldest PR its queries were mined from. Frozen with queries.toml.",
        "",
        f"name = {basic('history-' + name)}",
        "",
        "[[source]]",
        'kind = "git"',
        f"name = {basic(name)}",
        f"url = {basic('https://github.com/' + repo)}",
        f"commit = {basic(commit)}",
    ]
    if license:
        lines.append(f"license = {basic(license)}")
    lines.append('use = "adapt"')
    return "\n".join(lines) + "\n"


def run(*args: str, cwd: Path | None = None, check: bool = True) -> subprocess.CompletedProcess:
    return subprocess.run(list(args), cwd=cwd, capture_output=True, text=True, check=check)


def gh_json(*args: str):
    return json.loads(run("gh", *args).stdout)


def parse_name_status(out: str) -> list[Change]:
    """`git diff --name-status -z` output: NUL-separated, with raw paths, so
    a tab, quote or non-ASCII byte in a path survives."""
    fields = out.split("\0")
    found = []
    i = 0
    while i < len(fields) and fields[i]:
        status = fields[i][0]
        if status in "RC":
            found.append(Change(status, fields[i + 2], old_path=fields[i + 1]))
            i += 3
        else:
            found.append(Change(status, fields[i + 1]))
            i += 2
    return found


def merged_span(parents: int, subject: str, number: int, commits) -> int:
    """How many first-parent commits the PR spans on the base branch: one for
    a true merge or a squash (whose subject GitHub ends with `(#<number>)`),
    else, as a rebase merge, the PR's own commit count (`commits()`)."""
    if parents > 1 or subject.rstrip().endswith(f"(#{number})"):
        return 1
    return commits()


def closing_issue(refs: list[dict], repo: str) -> int | None:
    """The first issue a PR closes in `repo` itself (owner and name)."""
    owner, name = repo.split("/")
    for ref in refs:
        where = ref.get("repository") or {}
        if where.get("name") == name and (where.get("owner") or {}).get("login") == owner:
            return ref["number"]
    return None


def changes(clone: Path, merge: str, span: int = 1) -> tuple[str, list[Change]]:
    """The base the PR's `span` commits sit on, and what they changed
    against it."""
    parent = run("git", "rev-parse", f"{merge}~{span}", cwd=clone).stdout.strip()
    out = run("git", "-c", "core.quotePath=false", "diff", "--name-status", "-z", "-M", parent, merge, cwd=clone).stdout
    return parent, parse_name_status(out)


def mine(repo: str, clone: Path, max_prs: int) -> tuple[str, list[Mined]]:
    url = f"https://github.com/{repo}"
    if (clone / ".git").exists() or (clone / "HEAD").exists():
        run("git", "fetch", "-q", "origin", cwd=clone)
    else:
        clone.parent.mkdir(parents=True, exist_ok=True)
        run("git", "clone", "-q", "--filter=blob:none", url, str(clone))
    listed = gh_json(
        "pr", "list", "-R", repo, "--state", "merged", "--limit", "500",
        "--json", "number,title,url,author,mergeCommit,mergedAt,closingIssuesReferences",
    )
    listed.sort(key=lambda p: p["mergedAt"], reverse=True)
    prs = []
    for p in listed:
        merge = (p.get("mergeCommit") or {}).get("oid")
        if not merge or run("git", "cat-file", "-e", f"{merge}^{{commit}}", cwd=clone, check=False).returncode:
            continue
        if is_bot((p.get("author") or {}).get("login", "")):
            continue
        parents, subject = run("git", "log", "-1", "--format=%P%x00%s", merge, cwd=clone).stdout.strip().split("\0", 1)
        span = merged_span(
            len(parents.split()), subject, p["number"],
            lambda: len(gh_json("pr", "view", str(p["number"]), "-R", repo, "--json", "commits")["commits"]),
        )
        parent, found = changes(clone, merge, span)
        issue = None
        number = closing_issue(p.get("closingIssuesReferences") or [], repo)
        viewed = number and run("gh", "issue", "view", str(number), "-R", repo, "--json", "title,body", check=False)
        # A deleted or transferred issue leaves the PR with its title only.
        if viewed and viewed.returncode == 0:
            data = json.loads(viewed.stdout)
            issue = issue_text(data["title"], data.get("body") or "")
        prs.append(PullRequest(p["number"], p["title"], p["url"], (p.get("author") or {}).get("login", ""), parent, found, issue))
        # Enough to sample from: later PRs could only be older than the sample.
        if sum(eligible(q) for q in prs) >= max_prs:
            break

    def exists(commit: str, path: str) -> bool:
        return run("git", "cat-file", "-e", f"{commit}:{path}", cwd=clone, check=False).returncode == 0

    return select_prs(prs, exists, max_prs)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("repo", help="OWNER/REPO")
    parser.add_argument("--clone", type=Path, required=True, help="where to clone or fetch the repository")
    parser.add_argument("--max", type=int, default=40, help="most PRs to sample, newest first")
    parser.add_argument("--out", type=Path, default=Path(__file__).resolve().parents[2] / "history")
    args = parser.parse_args(argv)
    commit, mined = mine(args.repo, args.clone, args.max)
    license = (gh_json("repo", "view", args.repo, "--json", "licenseInfo").get("licenseInfo") or {}).get("spdxId")
    out = args.out / args.repo.split("/")[1]
    out.mkdir(parents=True, exist_ok=True)
    (out / "queries.toml").write_text(queries_toml(args.repo, commit, mined, date.today().isoformat()))
    (out / "manifest.toml").write_text(manifest_toml(args.repo, commit, license))
    queries = sum(1 + (m.issue is not None) for m in mined)
    print(f"{args.repo}: {len(mined)} PRs, {queries} queries, pinned at {commit}")


ENGINES = ("codecite", "grep", "popular")
# Not an engine's ranking: what a query costs (`<repo>-cost.json`).
COST = "cost"


def scorecard(runs: dict) -> str:
    """The scorecard: a row per repository, class and engine, then a pooled
    `all` row per engine, from each repository's bench reports (`codecite`,
    `grep`, `popular`: the query-blind floor), its index summary line
    (`index`), or why it failed (`failed`)."""
    lines = [
        "| Repository | Class | Engine | Queries | Recall@1 | Recall@10 | MRR |",
        "| --- | --- | --- | ---: | ---: | ---: | ---: |",
    ]
    notes = []

    def row(repo, cls, engine, s):
        lines.append(
            f"| {repo} | {cls} | {engine} | {s['queries']} | {s['recall_at_1']:.2f}"
            f" | {s['recall_at_10']:.2f} | {s['mrr']:.2f} |"
        )

    for repo in sorted(runs):
        run = runs[repo]
        if "failed" in run:
            lines.append(f"| {repo} | failed: {run['failed']} |")
            continue
        engines = [e for e in ENGINES if e in run]
        classes = sorted({c for e in engines for c in run[e]["classes"]}, reverse=True)
        for cls in classes:
            for engine in engines:
                if s := run[engine]["classes"].get(cls):
                    row(repo, cls, engine, s)
        for engine in engines:
            if s := run[engine].get("all"):
                row(repo, "all", engine, s)
        if run.get("index"):
            notes.append(f"- {repo}: {run['index']}")
    text = "\n".join(lines + ([""] + notes if notes else [])) + "\n"
    return text + cost_table(runs)


def cost_table(runs: dict) -> str:
    """`## Cost per query`: a row per repository with a cost report, the
    median output bytes and time of `codecite search` and of `rg -n` and
    `rg -l`, with the 90th percentile of the time in parentheses. Empty
    when no repository has one."""
    rows = []
    for repo in sorted(runs):
        report = runs[repo].get(COST)
        if not report:
            continue
        cells = [repo, str(report["queries"])]
        for engine in ("codecite", "rg_lines", "rg_files"):
            c = report[engine]
            cells += [str(c["median_bytes"]), f"{c['median_ms']:.1f} ({c['p90_ms']:.1f})"]
        rows.append("| " + " | ".join(cells) + " |")
    if not rows:
        return ""
    header = [
        "",
        "## Cost per query",
        "",
        "| Repository | Queries | codecite bytes | codecite ms | rg -n bytes | rg -n ms | rg -l bytes | rg -l ms |",
        "| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |",
    ]
    return "\n".join(header + rows) + "\n"


def scorecard_main(argv=None):
    """`codecite-history-scorecard RESULTS_DIR`: rebuild scorecard.md from
    `<repo>-<engine>.json`, `<repo>-index.txt` and `<repo>-failed.txt`."""
    parser = argparse.ArgumentParser(description="Rebuild the history benchmark's scorecard.")
    parser.add_argument("results", type=Path)
    args = parser.parse_args(argv)
    runs: dict = {}
    for path in sorted(args.results.iterdir()):
        repo, _, kind = path.stem.rpartition("-")
        if path.suffix == ".json" and kind in (*ENGINES, COST):
            # An empty or partial report is a failed run, which `failed`
            # explains; it must not hide the other repositories.
            try:
                runs.setdefault(repo, {})[kind] = json.loads(path.read_text())
            except json.JSONDecodeError:
                runs.setdefault(repo, {})
        elif path.suffix == ".txt" and kind in ("index", "failed"):
            runs.setdefault(repo, {})[kind] = path.read_text().strip()
    text = scorecard(runs)
    (args.results / "scorecard.md").write_text("# History benchmark scorecard\n\n" + text)
    print(text, end="")


if __name__ == "__main__":
    main()
