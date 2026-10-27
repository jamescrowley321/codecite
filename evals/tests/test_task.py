"""pi() records why the agent stopped, without a model key or a live sandbox."""

import asyncio
import json
from contextlib import asynccontextmanager

import pytest
from inspect_ai.agent import AgentState
from inspect_ai.model import ChatMessageUser
from inspect_ai.util import ExecResult, OutputLimitExceededError

from codecite_evals import _registry, config, task

TASKS = ["fastapi-middleware", "client-auth-uri", "backchannel-logout"]


class Store(dict):
    def set(self, key, value):
        self[key] = value


class Sandbox:
    def __init__(self, error):
        self.error = error
        self.files, self.commands = {}, []

    async def write_file(self, path, contents):
        self.files[path] = contents

    async def exec(self, cmd, **kwargs):
        self.commands.append(cmd)
        if cmd[0] == "pi":
            raise self.error
        if cmd == ["rm", "-rf", "/opt/codecite"]:
            return ExecResult(success=True, returncode=0, stdout="", stderr="")
        raise AssertionError(f"unexpected exec: {cmd}")


def run_pi(monkeypatch, error, arm="codecite", sb=None):
    state = AgentState(messages=[ChatMessageUser(content="do the task")])
    store = Store(condition=arm)
    sb = sb or Sandbox(error)

    @asynccontextmanager
    async def bridge(s):
        yield type("Bridge", (), {"state": s})()

    monkeypatch.setattr(task, "sandbox", lambda: sb)
    monkeypatch.setattr(task, "store", lambda: store)
    monkeypatch.setattr(task, "sandbox_agent_bridge", bridge)
    assert asyncio.run(task.pi()(state)) is state
    return store


def test_only_the_codecite_arm_has_codecite_tools(monkeypatch):
    code, base = Sandbox(TimeoutError()), Sandbox(TimeoutError())
    run_pi(monkeypatch, TimeoutError(), "codecite", code)
    run_pi(monkeypatch, TimeoutError(), "baseline", base)
    config = json.loads(code.files[task.PI_MCP])["mcpServers"]["codecite"]
    assert (config["command"], config["args"], config["exposure"]) == (task.CODECITE, ["mcp"], "direct")
    assert base.files == {} and ["rm", "-rf", "/opt/codecite"] in base.commands
    (code_pi,), (base_pi,) = ([c for c in sb.commands if c[0] == "pi"] for sb in (code, base))
    tools = lambda cmd: cmd[cmd.index("--tools") + 1].split(",")
    assert tools(code_pi) == task.PI_TOOLS + ["mcp__codecite__search", "mcp__codecite__read"]
    assert tools(base_pi) == task.PI_TOOLS
    assert "--no-mcp" in base_pi and "builtin:mcp" not in base_pi
    assert "--no-mcp" not in code_pi and code_pi[code_pi.index("-e") + 1] == "builtin:mcp"
    # Apart from the tools and the note, the arms run the same command.
    strip = lambda cmd: [a for a in cmd if a not in ("--no-mcp", "-e", "builtin:mcp") and a != cmd[cmd.index("--tools") + 1]]
    assert strip(code_pi) == strip(base_pi)


@pytest.mark.parametrize("task_id", TASKS)
def test_prompts_differ_only_by_the_note(task_id):
    d = config.task_dir(task_id)
    note = task.codecite_note(d)
    assert task.prompt(d, "codecite") == f"{task.prompt(d, 'baseline')}\n\n{note}\n"
    assert "mcp__codecite__search" in note and "mcp__codecite__read" in note


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


def test_arms_both_gives_both_samples():
    t = task.ab("fastapi-middleware")
    assert [s.metadata["arm"] for s in t.dataset] == ["baseline", "codecite"]


def test_arms_baseline_gives_only_the_baseline_sample():
    t = task.ab("fastapi-middleware", arms="baseline")
    assert [s.metadata["arm"] for s in t.dataset] == ["baseline"]
    assert "codecite" not in t.dataset[0].input


def test_unknown_arms_are_refused():
    with pytest.raises(ValueError, match="arms must be one of ab, aa, baseline"):
        task.ab("fastapi-middleware", arms="codecite")


def test_every_task_is_registered():
    # Inspect finds tasks through _registry: codecite_evals/<name>.
    assert callable(_registry.fastapi_middleware)
    assert callable(_registry.client_auth_uri)
    assert callable(_registry.ab)
    assert callable(_registry.backchannel_logout)


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
    assert "specifications" in task.prompt(config.task_dir("backchannel-logout"), "codecite")
    assert "specifications" not in task.prompt(config.task_dir("fastapi-middleware"), "codecite")


@pytest.mark.parametrize("task_id", TASKS)
def test_scorer_copies_in_its_own_tasks_tests_and_checks(monkeypatch, task_id):
    d, sb = config.task_dir(task_id), ScoringSandbox()
    score_with(monkeypatch, Store(), task_id, sb)
    assert sb.files["/score/checks.py"] == (d / "checks.py").read_text()
    assert sb.files["/score/hidden/test_hidden.py"] == (d / "hidden" / "test_hidden.py").read_text()
    assert sb.files["/score/start/app/main.py"] == (d / "project" / "app" / "main.py").read_text()


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
    assert first.input == second.input == task.prompt(config.task_dir("fastapi-middleware"), "baseline")
    assert "codecite" not in first.input.lower()


@pytest.mark.parametrize("name", ["fastapi_middleware", "client_auth_uri", "backchannel_logout"])
def test_every_task_takes_the_arms(name):
    assert [s.id for s in getattr(task, name)(arms="aa").dataset] == ["baseline-a", "baseline-b"]


def test_unknown_arms_are_refused():
    with pytest.raises(ValueError, match="arms must be one of"):
        task.fastapi_middleware(arms="abc")
