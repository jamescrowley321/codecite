"""The replay miner's choice of past PRs, without the network."""

import json
import subprocess

import pytest

from codecite_evals.replay import Candidate, acceptance_only, candidates, command_for, is_test_path, language, write_task

REPO = "o/r"


@pytest.mark.parametrize(
    "path",
    [
        "tests/test_a.py",
        "src/pkg/test/helpers.py",
        "src/pkg/test_a.py",
        "src/pkg/a_test.py",
        "conftest.py",
        "internal/client/client_test.go",
        "crates/x/tests/a.rs",
        "tests/cli.rs",
        "src/a.test.ts",
        "web/a.spec.jsx",
        "web/__tests__/a.js",
    ],
)
def test_test_paths(path):
    assert is_test_path(path)


@pytest.mark.parametrize(
    "path",
    [
        "src/pkg/a.py",
        "crates/x/src/a.rs",
        "internal/client/client.go",
        "src/a.ts",
        "tests/fixtures/data.json",
        "docs/testing.md",
        "src/contest.py",
    ],
)
def test_source_paths(path):
    assert not is_test_path(path)


def test_language_is_the_most_common_source_extension():
    assert language(["a.py", "b.py", "c.go", "README.md"]) == "python"
    assert language(["main.go", "go.mod"]) == "go"
    assert language(["crates/x/src/lib.rs"]) == "rust"
    assert language(["src/a.ts", "src/b.tsx", "x.py"]) == "node"
    assert language(["README.md", "Cargo.toml"]) == ""


def file(path, change="MODIFIED"):
    return {"path": path, "additions": 1, "deletions": 0, "changeType": change}


def record(number, *files, author="jamescrowley321", is_bot=False, issue=10, merged="2026-01-01T00:00:00Z"):
    refs = []
    if issue is not None:
        refs.append({"number": issue, "repository": {"name": "r", "owner": {"login": "o"}}})
    return {
        "number": number,
        "author": {"login": author, "is_bot": is_bot},
        "baseRefOid": f"base{number}",
        "mergeCommit": {"oid": f"merge{number}"},
        "mergedAt": merged,
        "closingIssuesReferences": refs,
        "files": list(files),
    }


ISSUES = {10: {"title": "Fix the thing", "body": "It breaks."}}


def test_a_candidate_carries_its_issue_tests_and_sources():
    [c] = candidates([record(1, file("src/a.py"), file("tests/test_a.py", "ADDED"), file("README.md"))], REPO, ISSUES)
    assert c == Candidate(
        repo=REPO,
        pr=1,
        issue=10,
        title="Fix the thing",
        body="It breaks.",
        base="base1",
        merge="merge1",
        tests=("tests/test_a.py",),
        sources=("src/a.py", "README.md"),
        language="python",
    )


def test_prs_that_cannot_be_replayed_are_skipped():
    prs = [
        record(1, file("src/a.py"), file("tests/test_a.py"), author="dependabot[bot]", is_bot=True),
        record(2, file("src/a.py"), file("tests/test_a.py"), author="app/renovate"),
        record(3, file("src/a.py"), file("tests/test_a.py"), issue=None),
        record(4, file("tests/test_a.py")),
        # Inline #[cfg(test)] modules only: no test file kept apart from source.
        record(5, file("crates/x/src/a.rs")),
        # A deleted test file can't be a hidden test.
        record(6, file("src/a.py"), file("tests/test_a.py", "DELETED")),
        # No source language to build.
        record(7, file("README.md"), file("tests/test_a.py")),
        # Closes an issue in another repository.
        {**record(8, file("src/a.py"), file("tests/test_a.py")),
         "closingIssuesReferences": [{"number": 10, "repository": {"name": "other", "owner": {"login": "o"}}}]},
        # Unmerged into this history.
        {**record(9, file("src/a.py"), file("tests/test_a.py")), "mergeCommit": None},
    ]
    assert candidates(prs, REPO, ISSUES) == []


def test_hidden_tests_are_the_language_of_the_fix():
    [c] = candidates([record(1, file("a.go"), file("a_test.go"), file("evals/tests/test_x.py"))], REPO, ISSUES)
    assert c.language == "go"
    assert c.tests == ("a_test.go",)


def test_candidates_are_newest_first():
    prs = [
        record(1, file("src/a.py"), file("tests/test_a.py"), merged="2026-01-01T00:00:00Z"),
        record(2, file("src/a.py"), file("tests/test_a.py"), merged="2026-03-01T00:00:00Z"),
        record(3, file("src/a.py"), file("tests/test_a.py"), merged="2026-02-01T00:00:00Z"),
    ]
    assert [c.pr for c in candidates(prs, REPO, ISSUES)] == [2, 3, 1]


def test_an_issue_not_fetched_leaves_the_title_and_body_empty():
    [c] = candidates([record(1, file("src/a.py"), file("tests/test_a.py"), issue=11)], REPO, ISSUES)
    assert (c.issue, c.title, c.body) == (11, "", "")


ACCEPTANCE = """package p

func TestAccThing(t *testing.T) {
\tresource.Test(t, resource.TestCase{})
}

func TestAccOther(t *testing.T) {
\tresource.ParallelTest(t, resource.TestCase{})
}
"""

UNIT = """package p

func TestAccThing(t *testing.T) {
\tresource.Test(t, resource.TestCase{})
}

func TestParse(t *testing.T) {
\tif parse("x") != 1 {
\t\tt.Fatal("bad")
\t}
}
"""


def test_acceptance_only():
    assert acceptance_only(ACCEPTANCE)
    assert not acceptance_only(UNIT)
    assert not acceptance_only("package p\n\nfunc helper() {}\n")


def git(repo, *args):
    return subprocess.run(["git", *args], cwd=repo, capture_output=True, text=True, check=True).stdout.strip()


@pytest.fixture
def clone(tmp_path):
    """A throwaway repository: a base commit, then a PR's merge that fixes
    `src/a.py`, adds `tests/test_new.py` and changes `tests/test_old.py`."""
    repo = tmp_path / "clone"
    repo.mkdir()
    git(repo, "init", "-q", "-b", "main")
    git(repo, "config", "user.email", "t@example.com")
    git(repo, "config", "user.name", "t")
    (repo / "src").mkdir()
    (repo / "tests").mkdir()
    (repo / "src/a.py").write_text("def f():\n    return 1\n")
    (repo / "tests/test_old.py").write_text("old\n")
    git(repo, "add", ".")
    git(repo, "commit", "-q", "-m", "base")
    base = git(repo, "rev-parse", "HEAD")
    (repo / "src/a.py").write_text("def f():\n    return 2\n")
    (repo / "tests/test_old.py").write_text("changed\n")
    (repo / "tests/test_new.py").write_text("new\n")
    git(repo, "add", ".")
    git(repo, "commit", "-q", "-m", "fix (#7)")
    return repo, base, git(repo, "rev-parse", "HEAD")


def replay(base, merge):
    return Candidate(
        repo="o/r",
        pr=7,
        issue=10,
        title="f returns the wrong number",
        body="It should return 2.",
        base=base,
        merge=merge,
        tests=("tests/test_new.py", "tests/test_old.py"),
        sources=("src/a.py",),
        language="python",
    )


def test_write_task_lays_out_the_task(clone, tmp_path):
    repo, base, merge = clone
    task = write_task(replay(base, merge), repo, tmp_path / "tasks")
    assert task == tmp_path / "tasks" / "replay-r-7"
    for name in ("task.json", "prompt.md", "reference/fix.patch", "hidden/tests/test_new.py", "hidden/tests/test_old.py"):
        assert (task / name).is_file(), name


def test_hidden_tests_are_the_merge_versions(clone, tmp_path):
    repo, base, merge = clone
    task = write_task(replay(base, merge), repo, tmp_path / "tasks")
    assert (task / "hidden/tests/test_new.py").read_text() == "new\n"
    assert (task / "hidden/tests/test_old.py").read_text() == "changed\n"
    # What the agent sees: the added test is absent at the base, the changed
    # one is its base version.
    assert subprocess.run(["git", "cat-file", "-e", f"{base}:tests/test_new.py"], cwd=repo).returncode != 0
    assert git(repo, "show", f"{base}:tests/test_old.py") == "old"


def test_fix_patch_holds_the_sources_and_no_test(clone, tmp_path):
    repo, base, merge = clone
    patch = (write_task(replay(base, merge), repo, tmp_path / "tasks") / "reference/fix.patch").read_text()
    assert "src/a.py" in patch and "+    return 2" in patch
    assert "tests/" not in patch


def test_prompt_is_the_issue_and_no_pr_text(clone, tmp_path):
    repo, base, merge = clone
    prompt = (write_task(replay(base, merge), repo, tmp_path / "tasks") / "prompt.md").read_text()
    assert "# f returns the wrong number" in prompt
    assert "It should return 2." in prompt
    assert "/work/project" in prompt
    for leak in ("fix (#7)", "test_new", "test_old", "#7"):
        assert leak not in prompt


def test_task_json_round_trips(clone, tmp_path):
    repo, base, merge = clone
    task = write_task(replay(base, merge), repo, tmp_path / "tasks")
    data = json.loads((task / "task.json").read_text())
    assert data == {
        "id": "replay-r-7",
        "kind": "replay",
        "gate": "replay",
        "repo": "o/r",
        "url": "https://github.com/o/r",
        "base": base,
        "merge": merge,
        "pr": 7,
        "issue": 10,
        "language": "python",
        "hidden": ["tests/test_new.py", "tests/test_old.py"],
        "test": command_for("python", ["tests/test_new.py", "tests/test_old.py"]),
        "setup": [],
        "epochs": {"quick": 5, "full": 10},
    }


def test_commands_by_language():
    assert command_for("python", ["tests/test_a.py"]) == ["uv", "run", "--with", "pytest==9.1.1", "pytest", "-q", "-p", "no:cacheprovider", "{hidden}"]
    assert command_for("go", ["internal/a/a_test.go", "internal/a/b_test.go", "c_test.go"]) == ["go", "test", "-count=1", ".", "./internal/a"]
    assert command_for("rust", ["crates/x/tests/cli.rs", "tests/a.rs"]) == ["cargo", "test", "--locked", "--offline", "--test", "cli", "--test", "a"]
    assert command_for("node", ["src/a.test.ts"]) == ["npm", "test", "--", "{hidden}"]
