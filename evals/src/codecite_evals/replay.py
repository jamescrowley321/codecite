"""Mine past issues into replay tasks.

A replay task is real past work replayed: the prompt is an issue as it was
asked, the project is its repository at the base of the PR that closed it,
and the hidden tests are the test files that PR added or changed. See
docs/design.md, Evaluation plan.
"""

import re
from collections import Counter
from dataclasses import dataclass
from pathlib import PurePosixPath

from codecite_evals.history import closing_issue, is_bot

LANGUAGES = {
    ".py": "python",
    ".go": "go",
    ".rs": "rust",
    ".js": "node",
    ".jsx": "node",
    ".mjs": "node",
    ".cjs": "node",
    ".ts": "node",
    ".tsx": "node",
}
JS_TEST = re.compile(r"\.(test|spec)\.[cm]?[jt]sx?$")
# A top-level Go function, and the terraform-plugin-testing calls that make
# one an acceptance test: those need TF_ACC and a live tenant.
GO_FUNC = re.compile(r"^func ", re.MULTILINE)
ACCEPTANCE = re.compile(r"\bresource\.(Parallel)?Test\(")


def is_test_path(path: str) -> bool:
    """A test file kept apart from source code, so overlaying it after the
    agent exits can't overwrite the agent's fix."""
    p = PurePosixPath(path)
    dirs = p.parts[:-1]
    if p.suffix == ".py":
        return "tests" in dirs or "test" in dirs or p.name.startswith("test_") or p.stem.endswith("_test") or p.name == "conftest.py"
    if p.suffix == ".go":
        return p.name.endswith("_test.go")
    if p.suffix == ".rs":
        # Cargo's integration tests; inline #[cfg(test)] modules live in src/.
        return "tests" in dirs
    if LANGUAGES.get(p.suffix) == "node":
        return bool(JS_TEST.search(p.name)) or "__tests__" in dirs
    return False


def language(paths: list[str]) -> str:
    """The language most of `paths` are written in, or "" when none is one
    the sandbox can build."""
    counts = Counter(LANGUAGES[s] for s in (PurePosixPath(p).suffix for p in paths) if s in LANGUAGES)
    if not counts:
        return ""
    return counts.most_common(1)[0][0]


@dataclass(frozen=True)
class Candidate:
    """A merged PR that closes an issue and adds or changes its tests."""

    repo: str
    pr: int
    issue: int
    title: str  # the issue's, as asked
    body: str
    base: str
    merge: str
    tests: tuple[str, ...]  # the hidden tests: the PR's test files in its language
    sources: tuple[str, ...]  # everything else it changed: the reference fix
    language: str


def candidates(prs: list[dict], repo: str, issues: dict[int, dict] | None = None) -> list[Candidate]:
    """The replayable PRs among `gh pr list --json number,author,baseRefOid,
    mergeCommit,mergedAt,closingIssuesReferences,files` records, newest first.

    `issues` maps an issue number to its `title` and `body`; one missing
    leaves them empty for the caller to fetch.
    """
    issues = issues or {}
    found = []
    for p in sorted(prs, key=lambda p: p.get("mergedAt") or "", reverse=True):
        author = p.get("author") or {}
        if author.get("is_bot") or is_bot(author.get("login", "")):
            continue
        merge = (p.get("mergeCommit") or {}).get("oid")
        issue = closing_issue(p.get("closingIssuesReferences") or [], repo)
        if not merge or issue is None:
            continue
        files = [f for f in p.get("files") or [] if f.get("changeType") != "DELETED"]
        sources = tuple(f["path"] for f in files if not is_test_path(f["path"]))
        lang = language(list(sources))
        tests = tuple(f["path"] for f in files if is_test_path(f["path"]) and language([f["path"]]) == lang)
        if not lang or not tests:
            continue
        text = issues.get(issue) or {}
        found.append(
            Candidate(
                repo=repo,
                pr=p["number"],
                issue=issue,
                title=text.get("title") or "",
                body=text.get("body") or "",
                base=p["baseRefOid"],
                merge=merge,
                tests=tests,
                sources=sources,
                language=lang,
            )
        )
    return found


def acceptance_only(text: str) -> bool:
    """Whether every `func Test` in a Go test file is an acceptance test,
    which can't run offline and so is no hidden test."""
    funcs = GO_FUNC.split(text)[1:]
    tests = [f for f in funcs if f.startswith("Test")]
    return bool(tests) and all(ACCEPTANCE.search(f) for f in tests)
