"""pi() records why the agent stopped, without a model key or a live sandbox."""

import asyncio
from contextlib import asynccontextmanager

import pytest
from inspect_ai.agent import AgentState
from inspect_ai.model import ChatMessageUser
from inspect_ai.util import ExecResult, OutputLimitExceededError

from codecite_evals import _registry, config, task

TASKS = ["fastapi-middleware", "client-auth-uri"]


class Store(dict):
    def set(self, key, value):
        self[key] = value


class Sandbox:
    def __init__(self, error):
        self.error = error

    async def exec(self, cmd, **kwargs):
        if cmd[0] == "pi":
            raise self.error
        raise AssertionError(f"unexpected exec: {cmd}")


def run_pi(monkeypatch, error):
    state = AgentState(messages=[ChatMessageUser(content="do the task")])
    store = Store(arm="codecite")

    @asynccontextmanager
    async def bridge(s):
        yield type("Bridge", (), {"state": s})()

    monkeypatch.setattr(task, "sandbox", lambda: Sandbox(error))
    monkeypatch.setattr(task, "store", lambda: store)
    monkeypatch.setattr(task, "sandbox_agent_bridge", bridge)
    assert asyncio.run(task.pi()(state)) is state
    return store


def test_timeout_is_an_agent_timeout(monkeypatch):
    store = run_pi(monkeypatch, TimeoutError())
    assert store.get("stopped") == "agent_timeout"
    assert "sandbox_error" not in store


@pytest.mark.parametrize(
    "error, recorded",
    [
        (ConnectionResetError("connection lost"), "ConnectionResetError: connection lost"),
        (RuntimeError("container killed"), "RuntimeError: container killed"),
    ],
)
def test_other_exception_is_a_sandbox_error(monkeypatch, error, recorded):
    store = run_pi(monkeypatch, error)
    assert store.get("stopped") == "sandbox_error"
    assert store.get("sandbox_error") == recorded


@pytest.mark.parametrize(
    "error",
    [OutputLimitExceededError("10 MiB", None), UnicodeDecodeError("utf-8", b"\xff", 0, 1, "invalid start byte")],
)
def test_agent_output_error_is_not_a_sandbox_error(monkeypatch, error):
    # pi's own output overflowing or failing to decode is the agent's doing.
    store = run_pi(monkeypatch, error)
    assert store.get("stopped") == "agent_output"
    assert "sandbox_error" not in store


class ScoringSandbox:
    def __init__(self):
        self.files = {}

    async def write_file(self, path, contents):
        self.files[path] = contents

    async def exec(self, cmd, **kwargs):
        if cmd[0] == "uv":
            return ExecResult(success=True, returncode=0, stdout="1 passed", stderr="")
        return ExecResult(success=True, returncode=0, stdout='{"checks": [{"name": "c", "passed": true}]}', stderr="")


def score_with(monkeypatch, store, task_id="fastapi-middleware", sb=None):
    sb = sb or ScoringSandbox()
    monkeypatch.setattr(task, "sandbox", lambda: sb)
    monkeypatch.setattr(task, "store", lambda: store)
    return asyncio.run(task.hidden_tests_and_checks(task_id)(None, None))


def test_scorer_records_a_sandbox_error(monkeypatch):
    score = score_with(monkeypatch, Store(stopped="sandbox_error", sandbox_error="RuntimeError: container killed"))
    assert score.value == 0
    assert score.metadata["stopped"] == "sandbox_error"
    assert score.metadata["sandbox_error"] == "RuntimeError: container killed"
    assert "sandbox failed (RuntimeError: container killed)" in score.explanation


def test_scorer_passes_an_agent_that_finished(monkeypatch):
    score = score_with(monkeypatch, Store())
    assert score.value == 1
    assert score.metadata["stopped"] is None


def test_every_task_is_registered():
    # Inspect finds tasks through _registry: codecite_evals/<name>.
    assert callable(_registry.fastapi_middleware)
    assert callable(_registry.client_auth_uri)


@pytest.mark.parametrize("task_id", TASKS)
def test_each_arm_gets_its_own_tasks_prompt(task_id):
    d = config.task_dir(task_id)
    baseline, codecite = task.prompt(d, "baseline"), task.prompt(d, "codecite")
    assert "{identity_model}" not in baseline
    assert baseline.startswith((d / "prompt.md").read_text().splitlines()[0])
    assert "codecite" not in baseline.lower()
    assert codecite.startswith(baseline) and "`codecite search eval" in codecite


def test_the_codecite_note_names_the_specs_only_when_the_corpus_has_them():
    # client-auth-uri's corpus adds the spec documents (task.json corpus.documents).
    assert "specifications" in task.prompt(config.task_dir("client-auth-uri"), "codecite")
    assert "specifications" not in task.prompt(config.task_dir("fastapi-middleware"), "codecite")


@pytest.mark.parametrize("task_id", TASKS)
def test_scorer_copies_in_its_own_tasks_tests_and_checks(monkeypatch, task_id):
    d, sb = config.task_dir(task_id), ScoringSandbox()
    score_with(monkeypatch, Store(), task_id, sb)
    assert sb.files["/score/checks.py"] == (d / "checks.py").read_text()
    assert sb.files["/score/hidden/test_hidden.py"] == (d / "hidden" / "test_hidden.py").read_text()
    assert sb.files["/score/start/app/main.py"] == (d / "project" / "app" / "main.py").read_text()
