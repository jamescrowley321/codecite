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


def history_repo(tmp_path, version=1):
    git(tmp_path, "init", "-q")
    mined = tmp_path / "evals" / "history" / "r"
    mined.mkdir(parents=True)
    (tmp_path / "evals" / "tasks").mkdir()
    (mined / "queries.toml").write_text(f"# version: {version}\ncorpus = \"history-r\"\n")
    (mined / "manifest.toml").write_text("name = \"history-r\"\n")
    return tmp_path, commit(tmp_path, "mine r")


def test_mined_history_unchanged_passes(tmp_path):
    repo, base = history_repo(tmp_path)
    assert problems(repo / "evals", base) == []


def test_editing_mined_history_fails(tmp_path):
    repo, base = history_repo(tmp_path)
    (repo / "evals/history/r/queries.toml").write_text("# version: 1\ncorpus = \"history-r\"\n# tuned\n")
    (repo / "evals/history/r/manifest.toml").write_text("name = \"history-r\"\n# moved\n")
    assert problems(repo / "evals", base) == [
        "history r: evals/history/r/manifest.toml changed without a new version in queries.toml",
        "history r: evals/history/r/queries.toml changed without a new version in queries.toml",
    ]


def test_remining_with_a_new_version_passes(tmp_path):
    repo, base = history_repo(tmp_path)
    (repo / "evals/history/r/queries.toml").write_text("# version: 2\ncorpus = \"history-r\"\n# remined\n")
    (repo / "evals/history/r/manifest.toml").write_text("name = \"history-r\"\n# moved\n")
    assert problems(repo / "evals", base) == []


def test_newly_mined_history_passes(tmp_path):
    repo, base = history_repo(tmp_path)
    (repo / "evals/history/s").mkdir()
    (repo / "evals/history/s/queries.toml").write_text("# version: 1\n")
    assert problems(repo / "evals", base) == []


def test_deleting_a_mined_repository_fails(tmp_path):
    import shutil

    repo, base = history_repo(tmp_path)
    shutil.rmtree(repo / "evals/history/r")
    assert problems(repo / "evals", base) == ["history r: evals/history/r was mined at the base and is gone"]


def test_a_malformed_version_line_is_a_problem_not_a_crash(tmp_path):
    repo, base = history_repo(tmp_path)
    (repo / "evals/history/r/queries.toml").write_text("# version: two\ncorpus = \"history-r\"\n")
    assert problems(repo / "evals", base) == ["history r: evals/history/r/queries.toml has a `# version:` line that isn't a number"]
