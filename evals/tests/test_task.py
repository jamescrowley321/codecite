"""pi() records why the agent stopped, without a model key or a live sandbox."""

import asyncio
from contextlib import asynccontextmanager

import pytest
from inspect_ai.agent import AgentState
from inspect_ai.model import ChatMessageUser
from inspect_ai.util import ExecResult, OutputLimitExceededError

from codecite_evals import task


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
    store = Store(condition="codecite")

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
    async def write_file(self, path, contents):
        pass

    async def exec(self, cmd, **kwargs):
        if cmd[0] == "uv":
            return ExecResult(success=True, returncode=0, stdout="1 passed", stderr="")
        return ExecResult(success=True, returncode=0, stdout='{"checks": [{"name": "c", "passed": true}]}', stderr="")


def score_with(monkeypatch, store):
    monkeypatch.setattr(task, "sandbox", lambda: ScoringSandbox())
    monkeypatch.setattr(task, "store", lambda: store)
    return asyncio.run(task.hidden_tests_and_checks()(None, None))


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


@pytest.mark.parametrize("arms, expected", [
    ("ab", [("baseline", "baseline"), ("codecite", "codecite")]),
    ("aa", [("baseline-a", "baseline"), ("baseline-b", "baseline")]),
])
def test_arms_select_the_samples(arms, expected):
    samples = list(task.fastapi_middleware(arms=arms).dataset)
    assert [(s.metadata["arm"], s.metadata["condition"]) for s in samples] == expected
    assert [s.id for s in samples] == [label for label, _ in expected]


def test_default_arms_are_baseline_against_codecite():
    assert [s.id for s in task.fastapi_middleware().dataset] == ["baseline", "codecite"]


def test_aa_arms_get_identical_prompts():
    first, second = task.fastapi_middleware(arms="aa").dataset
    assert first.input == second.input == task.prompt("baseline")
    assert task.CODECITE_NOTE not in first.input


def test_unknown_arms_are_refused():
    with pytest.raises(ValueError, match="arms must be one of"):
        task.fastapi_middleware(arms="abc")
