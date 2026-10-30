"""Mine past issues into replay tasks.

A replay task is real past work replayed: the prompt is an issue as it was
asked, the project is its repository at the base of the PR that closed it,
and the hidden tests are the test files that PR added or changed. See
docs/design.md, Evaluation plan.
"""

import argparse
import json
import os
import re
import shutil
import subprocess
from collections import Counter
from dataclasses import dataclass, replace
from pathlib import Path, PurePosixPath

from codecite_evals.history import closing_issue, gh_json, is_bot, run

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
# What installs a project's dependencies, run only when the base has the
# lock file that pins them.
SETUP = {
    "python": ("uv.lock", ["uv", "sync"]),
    "go": ("go.sum", ["go", "mod", "download"]),
    "rust": ("Cargo.lock", ["cargo", "fetch", "--locked"]),
    "node": ("package-lock.json", ["npm", "ci"]),
}
# The panel's default epochs per tier (inspect/panel.sh).
EPOCHS = {"quick": 5, "full": 10}
PROMPT = """The repository at `/work/project` is {repo}. Make the change this issue
asks for. Stop when you're done.

# {title}

{body}
"""


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


def command_for(language: str, hidden: list[str]) -> list[str]:
    """The command that runs only the hidden tests. `{hidden}` stands for
    their paths where the runner takes files."""
    if language == "python":
        return ["uv", "run", "--with", "pytest==9.1.1", "pytest", "-q", "-p", "no:cacheprovider", "{hidden}"]
    if language == "go":
        # Go tests run by package, so name the packages that hold them.
        packages = sorted({"./" + str(PurePosixPath(h).parent) if "/" in h else "." for h in hidden})
        return ["go", "test", "-count=1", *packages]
    if language == "rust":
        # Each file under a tests/ directory is its own test target.
        return ["cargo", "test", "--locked", "--offline", *(a for h in hidden for a in ("--test", PurePosixPath(h).stem))]
    if language == "node":
        return ["npm", "test", "--", "{hidden}"]
    raise ValueError(f"no test command for {language!r}")


def show(clone: Path, commit: str, path: str) -> bytes:
    return subprocess.run(["git", "show", f"{commit}:{path}"], cwd=clone, capture_output=True, check=True).stdout


def exists(clone: Path, commit: str, path: str) -> bool:
    return run("git", "cat-file", "-e", f"{commit}:{path}", cwd=clone, check=False).returncode == 0


def write_task(c: Candidate, clone: Path, out: Path) -> Path:
    """Write `c` as the task directory `out/replay-<repo>-<pr>/` and return
    it. `clone` holds both of its commits."""
    name = c.repo.split("/")[1]
    task_id = f"replay-{name}-{c.pr}"
    task = out / task_id
    if task.exists():
        shutil.rmtree(task)
    for path in c.tests:
        hidden = task / "hidden" / path
        hidden.parent.mkdir(parents=True, exist_ok=True)
        hidden.write_bytes(show(clone, c.merge, path))
    (task / "reference").mkdir(parents=True)
    fix = subprocess.run(["git", "diff", "--binary", c.base, c.merge, "--", *c.sources], cwd=clone, capture_output=True, check=True)
    (task / "reference" / "fix.patch").write_bytes(fix.stdout)
    (task / "prompt.md").write_text(PROMPT.format(repo=c.repo, title=c.title.strip(), body=c.body.strip()))
    lock, setup = SETUP[c.language]
    meta = {
        "id": task_id,
        "kind": "replay",
        "gate": "replay",
        "repo": c.repo,
        "url": f"https://github.com/{c.repo}",
        "base": c.base,
        "merge": c.merge,
        "pr": c.pr,
        "issue": c.issue,
        "language": c.language,
        "hidden": list(c.tests),
        "test": command_for(c.language, list(c.tests)),
        "setup": setup if exists(clone, c.base, lock) else [],
        "epochs": EPOCHS,
    }
    (task / "task.json").write_text(json.dumps(meta, indent=2) + "\n")
    return task


def runnable(c: Candidate, clone: Path) -> Candidate | None:
    """`c` without the hidden tests that can't run offline, or None when
    none is left."""
    tests = tuple(t for t in c.tests if c.language != "go" or not acceptance_only(show(clone, c.merge, t).decode(errors="replace")))
    return replace(c, tests=tests) if tests else None


def accepted(task: Path, clone: Path) -> subprocess.CompletedProcess:
    """Run the task's check.sh: it exits 0 only when the hidden tests fail
    at the base and pass with the reference fix."""
    return subprocess.run(["bash", str(task / "check.sh")], env={**os.environ, "REPLAY_REPO": str(clone)}, capture_output=True, text=True, check=False)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("repo", help="OWNER/REPO")
    parser.add_argument("--clone", type=Path, required=True, help="where to clone or fetch the repository")
    parser.add_argument("--max", type=int, default=3, help="most tasks to write")
    parser.add_argument("--out", type=Path, default=Path(__file__).resolve().parents[2] / "tasks")
    args = parser.parse_args(argv)
    clone = args.clone
    if (clone / ".git").exists() or (clone / "HEAD").exists():
        run("git", "fetch", "-q", "origin", cwd=clone)
    else:
        clone.parent.mkdir(parents=True, exist_ok=True)
        run("git", "clone", "-q", "--filter=blob:none", f"https://github.com/{args.repo}", str(clone))
    prs = gh_json(
        "pr", "list", "-R", args.repo, "--state", "merged", "--limit", "500",
        "--json", "number,author,baseRefOid,mergeCommit,mergedAt,closingIssuesReferences,files",
    )
    written = 0
    for c in candidates(prs, args.repo):
        if written >= args.max:
            break
        label = f"{args.repo}#{c.pr}"
        if any(run("git", "cat-file", "-e", f"{sha}^{{commit}}", cwd=clone, check=False).returncode for sha in (c.base, c.merge)):
            print(f"skip {label}: its commits are not in the clone")
            continue
        viewed = run("gh", "issue", "view", str(c.issue), "-R", args.repo, "--json", "title,body", check=False)
        if viewed.returncode:
            print(f"skip {label}: issue #{c.issue} can't be read")
            continue
        text = json.loads(viewed.stdout)
        c = runnable(replace(c, title=text["title"], body=text.get("body") or ""), clone)
        if c is None:
            print(f"skip {label}: its only tests are acceptance tests")
            continue
        task = write_task(c, clone, args.out)
        check = accepted(task, clone)
        if check.returncode:
            shutil.rmtree(task)
            why = (check.stderr or check.stdout).strip().splitlines()
            print(f"skip {label}: check.sh rejected it ({why[-1] if why else f'exit {check.returncode}'})")
            continue
        written += 1
        print(f"wrote {task.name}: issue #{c.issue}, {len(c.tests)} hidden test files")
    print(f"{args.repo}: {written} replay tasks")
