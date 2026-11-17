"""A size-dial task (house-rules): its corpus sizes, the codecite note at each,
the staged-size guard and its scorer, without a model key or a live sandbox."""

import asyncio
import json
import types

import pytest
from inspect_ai.util import ExecResult

from codecite_evals import _registry, config, report, task

TASK = "house-rules"


def meta():
    return json.loads((config.task_dir(TASK) / "task.json").read_text())


def test_house_rules_is_registered():
    assert _registry.house_rules is task.house_rules


@pytest.mark.parametrize("given, size", [("", "m"), ("s", "s"), ("m", "m"), ("l", "l")])
def test_the_corpus_size_is_the_given_one_or_the_default(given, size):
    assert task.corpus_size(meta(), given) == size


def test_an_unknown_corpus_size_is_refused():
    with pytest.raises(ValueError, match="corpus must be one of s, m, l"):
        task.corpus_size(meta(), "xl")


def test_a_task_without_sizes_takes_no_corpus():
    plain = json.loads((config.task_dir("backchannel-logout") / "task.json").read_text())
    assert task.corpus_size(plain) == ""
    with pytest.raises(ValueError, match="has no corpus sizes"):
        task.corpus_size(plain, "s")
    with pytest.raises(ValueError):
        task.ab_task("backchannel-logout", "ab", "s")


def test_the_note_names_what_each_size_indexes():
    d = config.task_dir(TASK)
    s, m, l = (task.codecite_note(d, size) for size in "sml")
    for note in (s, m, l):
        assert note.startswith(f"## codecite\n\ncodecite indexes the identity-stack source at `{task.PROJECT}`")
        assert all(tool in note for tool in task.CODECITE_TOOLS)
    assert task.CORPUS_DIR not in s and "requirements eval" not in s and task.SPECS_DIR not in s
    assert f"the identity-model source under `{task.CORPUS_DIR}/<name>`" in m
    assert task.SPECS_DIR in m and "requirements eval" in m
    assert "fastapi-users" in l and "authlib" in l and "fastapi-users" not in m


def test_the_arms_differ_only_by_the_note_at_every_size():
    for size in "sml":
        base, code = task.ab_task(TASK, "ab", size).dataset
        assert base.metadata == {"arm": "baseline", "condition": "baseline", "corpus": size}
        assert code.input.startswith(base.input) and code.input != base.input
        assert code.input[len(base.input):].strip() == task.codecite_note(config.task_dir(TASK), size)


def test_a_size_dial_task_checks_the_staged_size_before_the_agent():
    solvers = task.ab_task(TASK, "baseline", "l").solver
    assert len(solvers) == 3


class StagedSandbox:
    def __init__(self, staged):
        self.staged = staged

    async def read_file(self, path):
        assert path == f"{task.CORPUS_DIR}/.size"
        if self.staged is None:
            raise FileNotFoundError(path)
        return self.staged + "\n"


@pytest.mark.parametrize("staged", ["s", None])
def test_a_sample_on_an_image_of_another_size_errors(monkeypatch, staged):
    monkeypatch.setattr(task, "sandbox", lambda: StagedSandbox(staged))
    with pytest.raises(RuntimeError, match="not 'm'"):
        asyncio.run(task.corpus_staged("m")(object(), None))


def test_a_sample_on_an_image_of_its_size_runs(monkeypatch):
    monkeypatch.setattr(task, "sandbox", lambda: StagedSandbox("m"))
    state = object()
    assert asyncio.run(task.corpus_staged("m")(state, None)) is state


class Store(dict):
    def set(self, key, value):
        self[key] = value


class CorpusSandbox:
    """Records writes and execs; the hidden tests pass and checks.py prints `checks`."""

    def __init__(self, checks, tests_pass=True):
        self.checks, self.tests_pass, self.log = checks, tests_pass, []

    async def write_file(self, path, contents):
        self.log.append(("write", path))

    async def exec(self, cmd, **kwargs):
        self.log.append(("exec", cmd, kwargs))
        if cmd[0] == "python3":
            return ExecResult(success=True, returncode=0, stdout=json.dumps({"checks": self.checks}), stderr="")
        if cmd[0] == "sh":
            return ExecResult(success=True, returncode=0, stdout="", stderr="")
        ok = self.tests_pass
        return ExecResult(success=ok, returncode=0 if ok else 1, stdout="26 passed" if ok else "1 failed", stderr="")


def score(monkeypatch, sb, store=None):
    monkeypatch.setattr(task, "sandbox", lambda: sb)
    monkeypatch.setattr(task, "store", lambda: store or Store())
    state = types.SimpleNamespace(metadata={"corpus": "s"})
    return asyncio.run(task.hidden_tests_and_checks(TASK)(state, None))


def check(name, passed, control=False):
    return {"name": name, "rule": "§1", "level": "MUST", "control": control, "passed": passed, "detail": ""}


def test_the_hidden_tests_run_from_the_workdir_in_the_projects_environment(monkeypatch):
    sb = CorpusSandbox([check("ruff-§1", True)])
    s = score(monkeypatch, sb)
    assert s.value == 1 and s.metadata["corpus"] == "s" and s.metadata["violations"] == 0
    execs = [e for e in sb.log if e[0] == "exec"]
    (_, tests, kw), (_, start, _), (_, checks, _) = execs
    assert tests == [".venv/bin/python", "-m", "pytest", "-q", "-p", "no:cacheprovider", "/score/hidden"]
    assert kw["cwd"] == f"{task.PROJECT}/backend" and kw["env"]["UV_OFFLINE"] == "1"
    assert task.PROJECT_START in start[-1]
    assert checks == ["python3", "-I", "/score/checks.py", "/score/start", task.PROJECT]
    writes = {e[1] for e in sb.log if e[0] == "write"}
    assert {"/score/hidden/test_hidden.py", "/score/hidden/conftest.py", "/score/checks.py"} <= writes
    # Nothing reaches the sandbox before scoring writes it: the writes come first.
    assert [e[0] for e in sb.log][:3] == ["write"] * 3


def test_a_failed_control_is_reported_and_not_gated(monkeypatch):
    s = score(monkeypatch, CorpusSandbox([check("ruff-§1", True), check("control-print", False, control=True)]))
    assert s.value == 1
    assert "failed checks: none; failed controls (reported, not gated): control-print" in s.explanation


def test_a_rule_violation_fails_the_sample(monkeypatch):
    s = score(monkeypatch, CorpusSandbox([check("ruff-§1", False), check("control-print", True, control=True)]))
    assert s.value == 0 and s.metadata["violations"] == 1
    assert "failed checks: ruff-§1" in s.explanation


def test_crashed_checks_fail_the_sample(monkeypatch):
    s = score(monkeypatch, CorpusSandbox([]))
    assert s.value == 0 and "checks-crashed" in s.explanation


def test_failing_hidden_tests_fail_the_sample(monkeypatch):
    s = score(monkeypatch, CorpusSandbox([check("ruff-§1", True)], tests_pass=False))
    assert s.value == 0 and s.metadata["tests_tail"] == "1 failed"


def test_a_stopped_agent_cannot_pass(monkeypatch):
    s = score(monkeypatch, CorpusSandbox([check("ruff-§1", True)]), Store(stopped="agent_timeout"))
    assert s.value == 0 and s.explanation.startswith("agent timed out; ")


def log(task_name, args):
    return types.SimpleNamespace(eval=types.SimpleNamespace(task=task_name, task_args=args))


def test_the_scorecard_gives_each_corpus_size_its_own_table():
    assert report.task_name(log("codecite_evals/house_rules", {"arms": "ab", "corpus": "l"})) == "house_rules@l"
    assert report.task_name(log("codecite_evals/ab", {"task_id": "house-rules", "corpus": "s"})) == "house-rules@s"
    assert report.task_name(log("codecite_evals/backchannel_logout", {"arms": "ab"})) == "backchannel_logout"
