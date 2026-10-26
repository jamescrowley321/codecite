"""pi() records why the agent stopped, without a model key or a live sandbox."""

import asyncio
from contextlib import asynccontextmanager

import pytest
from inspect_ai.agent import AgentState
from inspect_ai.model import ChatMessageUser

import task


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
