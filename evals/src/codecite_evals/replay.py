"""Mine past issues into replay tasks.

A replay task is real past work replayed: the prompt is an issue as it was
asked, the project is its repository at the base of the PR that closed it,
and the hidden tests are the test files that PR added or changed. See
docs/design.md, Evaluation plan.
"""

import argparse
import json
import os
import posixpath
import re
import shutil
import subprocess
import unicodedata
from collections import Counter
from dataclasses import dataclass, replace
from datetime import datetime
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
# What proves a task: copied into each one write_task writes.
CHECK = Path(__file__).resolve().parents[2] / "tasks" / "_replay" / "check.sh"
# The panel's default epochs per tier (inspect/panel.sh).
EPOCHS = {"quick": 5, "full": 10}
PROMPT = """The repository at `/work/project` is {repo}. Make the change this issue
asks for. Stop when you're done.

# {title}

{body}
"""
# What follows the issue when the hidden tests call names the fix adds: the
# agent can't discover a name that doesn't exist yet, so the tests would
# measure guessing.
INTERFACE = """
## Names the tests use

The tests that check this change call these names, which don't exist yet.
Define them as shown.
"""
# Test directories: their non-code files are test data, and so is all of a
# data directory below one. Go's testdata is test data wherever it is.
TEST_DIRS = {"tests", "test", "__tests__"}
DATA_DIRS = {"fixtures", "__fixtures__", "__snapshots__", "snapshots"}
# A top-level definition on an added line, per language: the name is group 1.
DEFINITION = {
    "python": re.compile(r"^(?:\s*(?:async\s+)?def\s+|\s*class\s+)([A-Za-z_]\w*)|^([A-Z_][A-Z0-9_]*)\s*(?::[^=]*)?=[^=]"),
    "go": re.compile(r"^(?:func\s+(?:\([^)]*\)\s*)?|type\s+|const\s+|var\s+)([A-Za-z_]\w*)"),
    "rust": re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(?:fn|struct|enum|trait|const|static|type|mod)\s+([A-Za-z_]\w*)"),
    "node": re.compile(r"^\s*(?:export\s+)?(?:default\s+)?(?:async\s+)?(?:function\*?|class|const|let|var)\s+([A-Za-z_$][\w$]*)"),
}
# An entry of a Go `const (` or `var (` block; anywhere else the same line
# sets a local variable.
GO_BLOCK_START = re.compile(r"^(?:const|var)\s*\(\s*(?://.*)?$")
# The `=` is optional: an iota-continued constant or a typed var has none.
GO_BLOCK_ENTRY = re.compile(r"^\t([A-Za-z_]\w*)(?:\s+[\w.*\[\]]+)?\s*(?:=[^=]|//|$)")
HUNK = re.compile(r"^@@ -\d+(?:,\d+)? \+(\d+)(?:,\d+)? @@")
COMMENT = re.compile(r"^\s*(?://|#|/\*|\*)")
# How much of a definition's doc comment reaches the prompt: the mined PR's
# text is untrusted, so only its last few lines, each cut short.
COMMENT_LINES = 5
LINE_CHARS = 200
BACKTICKS = re.compile(r"`+")
IDENT = re.compile(r"[A-Za-z_]\w*")


def is_test_path(path: str) -> bool:
    """A test file kept apart from source code, so overlaying it after the
    agent exits can't overwrite the agent's fix."""
    p = PurePosixPath(path)
    dirs = p.parts[:-1]
    if is_test_data(path):
        return False
    if p.suffix == ".py":
        return "tests" in dirs or "test" in dirs or p.name.startswith("test_") or p.stem.endswith("_test") or p.name == "conftest.py"
    if p.suffix == ".go":
        return p.name.endswith("_test.go")
    if p.suffix == ".rs":
        # Cargo's integration tests, in a crate's tests/; inline
        # #[cfg(test)] modules and src/tests/ are the library's own.
        if "tests" not in dirs:
            return False
        i = dirs.index("tests")
        return i == 0 or dirs[i - 1] != "src"
    if LANGUAGES.get(p.suffix) == "node":
        return bool(JS_TEST.search(p.name)) or "__tests__" in dirs
    return False


def is_test_data(path: str) -> bool:
    """A file a PR's tests read rather than run (a fixture, golden file or
    snapshot): anything under `testdata` or `__snapshots__` and any `.snap`
    file, and under a test directory a
    non-code file or anything in a data directory below it. It goes in
    `hidden/` with the tests, so they find it whatever the agent does. A
    `fixtures` or `snapshots` directory outside the tests is source."""
    p = PurePosixPath(path)
    dirs = p.parts[:-1]
    # Go's testdata, and Jest's snapshots, which sit beside the test file.
    if "testdata" in dirs or "__snapshots__" in dirs or p.suffix == ".snap":
        return True
    tests = [i for i, d in enumerate(dirs) if d in TEST_DIRS]
    if not tests:
        return False
    return p.suffix not in LANGUAGES or bool(DATA_DIRS.intersection(dirs[tests[0] + 1:]))


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
    data: tuple[str, ...] = ()  # test data the tests read, overlaid with them
    commits: tuple[tuple[str, str], ...] = ()  # the PR's commits: (authoredDate, headline)


def candidates(prs: list[dict], repo: str, issues: dict[int, dict] | None = None) -> list[Candidate]:
    """The replayable PRs among `gh pr list --json number,author,baseRefOid,
    mergeCommit,mergedAt,closingIssuesReferences,files,commits` records,
    newest first.

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
        data = tuple(f["path"] for f in files if is_test_data(f["path"]))
        sources = tuple(f["path"] for f in files if not is_test_path(f["path"]) and not is_test_data(f["path"]))
        lang = language(list(sources))
        tests = tuple(f["path"] for f in files if is_test_path(f["path"]) and language([f["path"]]) == lang)
        if not lang or not tests or (lang == "rust" and not rust_targets(tests)):
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
                data=data,
                commits=tuple((k.get("authoredDate") or "", k.get("messageHeadline") or "") for k in p.get("commits") or []),
            )
        )
    return found


def acceptance_only(text: str) -> bool:
    """Whether every `func Test` in a Go test file is an acceptance test,
    which can't run offline and so is no hidden test."""
    funcs = GO_FUNC.split(text)[1:]
    tests = [f for f in funcs if f.startswith("Test")]
    return bool(tests) and all(ACCEPTANCE.search(f) for f in tests)


def rust_targets(hidden) -> list[str]:
    """Each file directly in a tests/ directory is its own test target; one
    deeper (tests/common/mod.rs) is a module the targets share."""
    return [PurePosixPath(h).stem for h in hidden if PurePosixPath(h).parent.name == "tests"]


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
        targets = rust_targets(hidden)
        if not targets:
            # A bare `cargo test` runs the whole package, not the hidden tests.
            raise ValueError("no rust test target among the hidden tests")
        return ["cargo", "test", "--locked", "--offline", *(a for t in targets for a in ("--test", t))]
    if language == "node":
        return ["npm", "test", "--", "{hidden}"]
    raise ValueError(f"no test command for {language!r}")


def show(clone: Path, commit: str, path: str) -> bytes:
    return subprocess.run(["git", "show", f"{commit}:{path}"], cwd=clone, capture_output=True, check=True).stdout


def exists(clone: Path, commit: str, path: str) -> bool:
    return run("git", "cat-file", "-e", f"{commit}:{path}", cwd=clone, check=False).returncode == 0


def first_parent(clone: Path, merge: str) -> str:
    """The merge commit's first parent: the tree the PR's change was applied
    to. The PR's `baseRefOid` can lag it, and then the project and the fix
    would carry other PRs' edits."""
    return run("git", "rev-parse", f"{merge}^1", cwd=clone).stdout.strip()


def versions(out: Path, repo: str, pr: int) -> list[int]:
    """The versions of `repo`'s PR `pr` under `out`: 1 for the bare
    `replay-<repo>-<pr>`, n for its `-v<n>`."""
    stem = f"replay-{repo.split('/')[1]}-{pr}"
    found = [1] if (out / stem).exists() else []
    for p in out.glob(f"{stem}-v*"):
        m = re.fullmatch(re.escape(stem) + r"-v(\d+)", p.name)
        if m:
            found.append(int(m.group(1)))
    return found


def task_id(c: Candidate, out: Path) -> str:
    """`replay-<repo>-<pr>` when no version of the PR is under `out`, else
    one past the highest version there: a task is never rewritten, so a
    re-mined PR is a new task. An id is unique only among the task
    directories that exist; one deleted can be issued again."""
    stem = f"replay-{c.repo.split('/')[1]}-{c.pr}"
    found = versions(out, c.repo, c.pr)
    return f"{stem}-v{max(found) + 1}" if found else stem


def mined(out: Path, repo: str, pr: int) -> bool:
    """Whether some version of `repo`'s PR `pr` is a task under `out`."""
    return bool(versions(out, repo, pr))


def whole_change(clone: Path, parent: str, c: Candidate) -> bool:
    """Whether the merge commit, against `parent`, changes every file the PR
    did: true of a squash or a merge commit. A multi-commit rebase merge's
    first parent is the PR's own second-to-last commit, already holding
    part of the change, so the project would leak it."""
    out = subprocess.run(["git", "diff", "--name-only", "-z", parent, c.merge], cwd=clone, capture_output=True, check=True).stdout
    changed = set(os.fsdecode(p) for p in out.split(b"\0") if p)
    return set((*c.tests, *c.sources, *c.data)) <= changed


def rebase_merged(clone: Path, c: Candidate) -> bool:
    """Whether a PR of several commits was rebase-merged: its merge commit
    has one parent and is the PR's last commit replayed, keeping its author
    date or subject, where a squash makes a new commit. Then the first
    parent already holds the PR's earlier commits."""
    if len(c.commits) < 2:
        return False
    parents, date, subject = run("git", "log", "-1", "--format=%P%n%aI%n%s", c.merge, cwd=clone).stdout.split("\n", 2)
    if len(parents.split()) != 1:
        return False
    last_date, last_subject = c.commits[-1]
    return same_time(date, last_date) or subject.rstrip("\n") == last_subject


def same_time(a: str, b: str) -> bool:
    try:
        return datetime.fromisoformat(a.replace("Z", "+00:00")) == datetime.fromisoformat(b.replace("Z", "+00:00"))
    except ValueError:
        return False


def visible(text: str) -> str:
    """`text` without Unicode format and control characters (zero-width,
    bidi overrides, tag characters): a mined line reaches the agent's
    prompt, and these would hide words from the person who reviews it."""
    return "".join(ch for ch in text if ch in "\t\n" or unicodedata.category(ch) not in ("Cf", "Cc"))


def go_blocks(text: str) -> set[int]:
    """The 1-based numbers of the lines inside Go `const (`/`var (` blocks."""
    inside, found = False, set()
    for n, line in enumerate(text.splitlines(), 1):
        if inside and line.startswith(")"):
            inside = False
        elif inside:
            found.add(n)
        elif GO_BLOCK_START.match(line):
            inside = True
    return found


def interface(c: Candidate, clone: Path) -> list[tuple[str, str, list[str]]]:
    """The names the hidden tests use that the fix defines and the base
    lacks, each as (name, the file defining it, the definition's added
    lines: its comment, then the line that defines it)."""
    pattern = DEFINITION.get(c.language)
    if pattern is None:
        return []
    used = {name for t in c.tests for name in IDENT.findall(show(clone, c.merge, t).decode(errors="replace"))}
    diff = subprocess.run(["git", "diff", "-U0", c.base, c.merge, "--", *c.sources], cwd=clone, capture_output=True, check=True)
    found, path, comment, lineno, blocks = {}, None, [], 0, {}
    for line in diff.stdout.decode(errors="replace").splitlines():
        if line.startswith("+++ "):
            # git ends the header with a tab when the path has a space.
            path = line[len("+++ b/"):].rstrip("\t") if line.startswith("+++ b/") else None
            comment = []
            continue
        hunk = HUNK.match(line)
        if hunk:
            lineno = int(hunk.group(1))
            comment = []
            continue
        if not line.startswith("+") or path is None:
            comment = []
            continue
        here, lineno = lineno, lineno + 1
        text = line[1:]
        if COMMENT.match(text):
            comment.append(visible(text).strip()[:LINE_CHARS])
            continue
        m = pattern.match(text)
        if not m and c.language == "go":
            if path not in blocks:
                blocks[path] = go_blocks(show(clone, c.merge, path).decode(errors="replace"))
            m = GO_BLOCK_ENTRY.match(text) if here in blocks[path] else None
        name = m and next(g for g in m.groups() if g)
        if name and name in used and name not in found and len(name) > 2:
            found[name] = (visible(path), [*comment[-COMMENT_LINES:], visible(text).strip()[:LINE_CHARS]])
        comment = []
    absent = [n for n in found if run("git", "grep", "-q", "-w", "-e", n, c.base, cwd=clone, check=False).returncode]
    return [(n, *found[n]) for n in absent]


def prompt(c: Candidate, clone: Path) -> str:
    """The issue as asked, then the names its tests call that the agent
    can't discover, if any."""
    text = PROMPT.format(repo=c.repo, title=visible(c.title).strip(), body=visible(c.body).strip())
    names = interface(c, clone)
    if not names:
        return text
    lang = c.language if c.language != "node" else "js"
    parts = [text, INTERFACE]
    for name, path, lines in names:
        # Longer than any backtick run in the lines, so none closes it early.
        fence = "`" * max([3, *(len(r) + 1 for line in lines for r in BACKTICKS.findall(line))])
        parts.append(f"\n- `{name}` in {code(path)}:\n\n  {fence}{lang}\n" + "".join(f"  {line}\n" for line in lines) + f"  {fence}\n")
    return "".join(parts)


def code(text: str) -> str:
    """`text` as an inline code span that a backtick in it can't end."""
    ticks = "`" * max([1, *(len(r) + 1 for r in BACKTICKS.findall(text))])
    pad = " " if text.startswith("`") or text.endswith("`") else ""
    return f"{ticks}{pad}{text}{pad}{ticks}"


def project_dir(clone: Path, commit: str, lock: str, hidden: list[str]) -> str:
    """The directory a project's setup and tests run from: the nearest one
    above the first hidden test that holds `lock` at `commit`, so a project
    in a monorepo's subdirectory finds its dependencies. "." is the root."""
    for parent in PurePosixPath(hidden[0]).parents:
        if exists(clone, commit, str(parent / lock)):
            return str(parent)
    return "."


def write_task(c: Candidate, clone: Path, out: Path) -> Path:
    """Write `c` as a new task directory under `out`, named by `task_id`,
    and return it. `clone` holds both of its commits. A task already there
    is left alone."""
    tid = task_id(c, out)
    task = out / tid
    task.mkdir(parents=True)
    try:
        fill(c, clone, task)
    except BaseException:
        # A half-written directory would read as a task and block re-mining.
        shutil.rmtree(task, ignore_errors=True)
        raise
    return task


def fill(c: Candidate, clone: Path, task: Path) -> None:
    """Write `c`'s files into the empty directory `task`."""
    for path in (*c.tests, *c.data):
        hidden = task / "hidden" / path
        hidden.parent.mkdir(parents=True, exist_ok=True)
        hidden.write_bytes(show(clone, c.merge, path))
    (task / "reference").mkdir()
    fix = subprocess.run(["git", "diff", "--binary", c.base, c.merge, "--", *c.sources], cwd=clone, capture_output=True, check=True)
    (task / "reference" / "fix.patch").write_bytes(fix.stdout)
    (task / "prompt.md").write_text(prompt(c, clone))
    lock, setup = SETUP[c.language]
    workdir = project_dir(clone, c.base, lock, list(c.tests))
    # The test command runs in workdir, so it names the hidden tests from there.
    local = [posixpath.relpath(t, workdir) for t in c.tests]
    meta = {
        "id": task.name,
        "kind": "replay",
        "repo": c.repo,
        "url": f"https://github.com/{c.repo}",
        "base": c.base,
        "merge": c.merge,
        "pr": c.pr,
        "issue": c.issue,
        "language": c.language,
        "hidden": list(c.tests),
        "data": list(c.data),
        "test": command_for(c.language, local),
        "setup": setup if exists(clone, c.base, str(PurePosixPath(workdir) / lock)) else [],
        "workdir": workdir,
        "epochs": EPOCHS,
    }
    (task / "task.json").write_text(json.dumps(meta, indent=2) + "\n")
    shutil.copy2(CHECK, task / "check.sh")


def cargo_test(clone: Path, commit: str, path: str) -> bool:
    """Whether a Rust file is in a crate's integration tests: its `tests/`
    directory sits beside a Cargo.toml. One deeper in a crate's src/ is a
    module of the library, whatever the path above it."""
    dirs = PurePosixPath(path).parts[:-1]
    crate = PurePosixPath(*dirs[: dirs.index("tests")]) if dirs.index("tests") else PurePosixPath(".")
    return exists(clone, commit, str(crate / "Cargo.toml"))


def runnable(c: Candidate, clone: Path) -> Candidate | None:
    """`c` without the hidden tests that can't run offline, or None when
    none is left. A Rust file that turns out to be library code moves to
    the fix, and a Rust candidate needs a test target left."""
    if c.language == "rust":
        library = tuple(t for t in c.tests if not cargo_test(clone, c.merge, t))
        c = replace(c, tests=tuple(t for t in c.tests if t not in library), sources=(*c.sources, *library))
        if not rust_targets(c.tests):
            return None
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
    parser.add_argument("--pr", type=int, action="append", default=[], help="mine this PR again, as a new task id if one is there (repeatable)")
    args = parser.parse_args(argv)
    clone = args.clone
    if (clone / ".git").exists() or (clone / "HEAD").exists():
        run("git", "fetch", "-q", "origin", cwd=clone)
    else:
        clone.parent.mkdir(parents=True, exist_ok=True)
        run("git", "clone", "-q", "--filter=blob:none", f"https://github.com/{args.repo}", str(clone))
    prs = gh_json(
        "pr", "list", "-R", args.repo, "--state", "merged", "--limit", "500",
        "--json", "number,author,baseRefOid,mergeCommit,mergedAt,closingIssuesReferences,files,commits",
    )
    written, seen = 0, set()
    for c in candidates(prs, args.repo):
        if written >= args.max:
            break
        label = f"{args.repo}#{c.pr}"
        if args.pr and c.pr not in args.pr:
            continue
        seen.add(c.pr)
        if not args.pr and mined(args.out, args.repo, c.pr):
            print(f"skip {label}: already a task (--pr {c.pr} mines it again)")
            continue
        if run("git", "cat-file", "-e", f"{c.merge}^{{commit}}", cwd=clone, check=False).returncode:
            print(f"skip {label}: its merge commit is not in the clone")
            continue
        if rebase_merged(clone, c):
            print(f"skip {label}: rebase-merged, so its merge's parent already holds the PR's earlier commits")
            continue
        parent = first_parent(clone, c.merge)
        if not whole_change(clone, parent, c):
            print(f"skip {label}: its merge commit holds only part of the PR (a rebase merge?)")
            continue
        c = replace(c, base=parent)
        viewed = run("gh", "issue", "view", str(c.issue), "-R", args.repo, "--json", "title,body", check=False)
        if viewed.returncode:
            print(f"skip {label}: issue #{c.issue} can't be read")
            continue
        text = json.loads(viewed.stdout)
        c = runnable(replace(c, title=text["title"], body=text.get("body") or ""), clone)
        if c is None:
            print(f"skip {label}: no hidden test can run offline (acceptance tests only, or no cargo test target)")
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
    for n in sorted(set(args.pr) - seen):
        print(f"skip {args.repo}#{n}: not a replayable merged PR among the 500 newest, or --max was reached first")
    print(f"{args.repo}: {written} replay tasks")
