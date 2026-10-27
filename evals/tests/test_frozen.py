"""The frozen-task check, on a throwaway git repository built here."""

import json
import subprocess

import pytest

from codecite_evals import config
from codecite_evals.frozen import problems


def git(repo, *args):
    return subprocess.run(["git", "-C", str(repo), *args], capture_output=True, text=True, check=True).stdout.strip()


def commit(repo, message="change"):
    git(repo, "add", "-A")
    git(repo, "-c", "user.name=t", "-c", "user.email=t@localhost", "commit", "-qm", message)
    return git(repo, "rev-parse", "HEAD")


@pytest.fixture
def repo(tmp_path):
    """A repo whose evals/tasks/t froze at its first commit."""
    git(tmp_path, "init", "-q")
    task = tmp_path / "evals" / "tasks" / "t"
    (task / "project").mkdir(parents=True)
    (task / "prompt.md").write_text("do it\n")
    (task / "project" / "main.py").write_text("pass\n")
    (task / "task.json").write_text(json.dumps({"id": "t"}))
    first = commit(tmp_path, "add t")
    (task / "task.json").write_text(json.dumps({"id": "t", "frozen_at": first}))
    commit(tmp_path, "freeze t")
    return tmp_path


def test_a_task_frozen_as_it_stands_passes(repo):
    assert problems(repo / "evals") == []


def test_a_changed_file_fails(repo):
    (repo / "evals/tasks/t/prompt.md").write_text("do it differently\n")
    assert problems(repo / "evals") == [f"t: evals/tasks/t/prompt.md changed since it froze at {git(repo, 'rev-list', '--max-parents=0', 'HEAD')}"]


def test_an_added_file_fails_committed_or_not(repo):
    (repo / "evals/tasks/t/project/extra.py").write_text("x = 1\n")
    assert [p.split(": ")[1].split()[0] for p in problems(repo / "evals")] == ["evals/tasks/t/project/extra.py"]
    commit(repo)
    assert len(problems(repo / "evals")) == 1


def test_a_changed_task_json_fails(repo):
    spec = repo / "evals/tasks/t/task.json"
    data = json.loads(spec.read_text())
    spec.write_text(json.dumps({**data, "setup": ["uv", "sync"]}))
    assert "evals/tasks/t/task.json changed" in problems(repo / "evals")[0]


def test_moving_frozen_at_fails_against_the_base(repo):
    base = git(repo, "rev-parse", "HEAD")
    (repo / "evals/tasks/t/prompt.md").write_text("do it differently\n")
    later = commit(repo)
    spec = repo / "evals/tasks/t/task.json"
    spec.write_text(json.dumps({"id": "t", "frozen_at": later}))
    commit(repo)
    assert problems(repo / "evals") == []  # matches its own frozen_at...
    assert "frozen_at moved" in problems(repo / "evals", base)[0]  # ...but not the base's


def test_an_unfrozen_task_may_change(repo):
    other = repo / "evals/tasks/u"
    other.mkdir()
    (other / "task.json").write_text(json.dumps({"id": "u"}))
    (other / "prompt.md").write_text("anything\n")
    assert problems(repo / "evals") == []


def test_an_unknown_frozen_at_fails(repo):
    (repo / "evals/tasks/t/task.json").write_text(json.dumps({"id": "t", "frozen_at": "0" * 40}))
    assert "is not a commit here" in problems(repo / "evals")[0]


def test_fastapi_middleware_is_frozen_at_its_first_ab_run():
    # A task freezes when its first A/B run starts; one still calibrating has no frozen_at.
    spec = config.task_dir("fastapi-middleware") / "task.json"
    assert json.loads(spec.read_text())["frozen_at"] == "a304f6a8e52f48a2991e6640e644f927873c5bce"


def test_an_unknown_base_fails_rather_than_skipping_the_check(repo):
    assert problems(repo / "evals", "origin/no-such-branch") == ["base ref origin/no-such-branch is not a commit here (CI needs the full history)"]
