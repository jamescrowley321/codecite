"""Claude Code as the agent, without a token, a sandbox or a model call."""

import asyncio
import json

import pytest
from inspect_ai.agent import AgentState
from inspect_ai.model import ChatMessageAssistant, ChatMessageUser
from inspect_ai.util import ExecResult

from codecite_evals import claude_code, task


def assistant(id, usage, *blocks):
    return {"type": "assistant", "message": {"id": id, "usage": usage, "content": list(blocks)}}


def tool(name, **args):
    return {"type": "tool_use", "name": name, "input": args}


def stream(*events):
    return "\n".join(json.dumps(e) for e in events) + "\n"


USAGE = {"input_tokens": 10, "output_tokens": 5, "cache_creation_input_tokens": 100, "cache_read_input_tokens": 1000}
DONE = {"type": "result", "subtype": "success", "is_error": False, "duration_api_ms": 4500, "result": "Fixed it."}


def test_only_the_codecite_arm_gets_an_mcp_config():
    code = claude_code.command("codecite", "do it", "claude-sonnet-5-5", 600)
    base = claude_code.command("baseline", "do it", "claude-sonnet-5-5", 600)
    assert code[code.index("--mcp-config") + 1] == claude_code.MCP
    assert "--mcp-config" not in base
    assert "--strict-mcp-config" in code and "--strict-mcp-config" in base
    assert [a for a in code if a not in ("--mcp-config", claude_code.MCP)] == base


def test_both_arms_get_pis_tools_and_no_web():
    cmd = claude_code.command("baseline", "do it", "claude-sonnet-5-5", 600)
    tools = cmd[cmd.index("--tools") + 1].split(",")
    assert tools == claude_code.TOOLS
    assert not {"WebFetch", "WebSearch", "Task", "Agent"} & set(tools)


def test_the_prompt_comes_last_and_timeout_stops_it_first():
    cmd = claude_code.command("baseline", "--not-a-flag", "claude-haiku-5-5", 600)
    assert cmd[-2:] == ["--", "--not-a-flag"]
    assert cmd[:4] == ["timeout", "--kill-after=10", "600", "claude"]
    assert cmd[cmd.index("--model") + 1] == "claude-haiku-5-5"


def test_measures_count_each_message_once():
    out = stream(
        assistant("m1", USAGE, tool("mcp__codecite__search", query="client auth")),
        # The same message again, streaming its next block.
        assistant("m1", USAGE, tool("Bash", command="codecite read eval 'rfc6749#section-2.3'")),
        assistant("m2", USAGE, tool("Bash", command="grep -r codecite ."), tool("Read", file_path="/x")),
        DONE,
    )
    assert claude_code.measures(out) == {
        "agent_turns": 2,
        "agent_tokens": 2 * 1115,
        "agent_output_tokens": 10,
        "agent_model_seconds": 4.5,
        "codecite_calls": 2,
    }


def test_a_cut_off_stream_still_has_measures_but_no_model_time():
    out = stream(assistant("m1", USAGE)) + '{"type": "assist'
    m = claude_code.measures(out)
    assert (m["agent_turns"], m["agent_model_seconds"]) == (1, None)
    assert claude_code.failure(out) is None


@pytest.mark.parametrize(
    "result, expected",
    [
        (DONE, None),
        ({**DONE, "is_error": True, "result": "Claude AI usage limit reached"}, "Claude AI usage limit reached"),
        ({"type": "result", "subtype": "error_during_execution", "is_error": True}, "error_during_execution"),
    ],
)
def test_failure(result, expected):
    assert claude_code.failure(stream(result)) == expected


def test_the_provider_needs_the_token(monkeypatch):
    monkeypatch.delenv(claude_code.TOKEN, raising=False)
    with pytest.raises(RuntimeError, match=claude_code.TOKEN):
        claude_code.ClaudeCode("claude-sonnet-5-5")
    monkeypatch.setenv(claude_code.TOKEN, "t")
    assert claude_code.ClaudeCode("claude-sonnet-5-5", strict_tools=False).model_name == "claude-sonnet-5-5"


class Store(dict):
    def set(self, key, value):
        self[key] = value


class Sandbox:
    def __init__(self, result):
        self.result = result
        self.files, self.commands, self.envs = {}, [], []

    async def write_file(self, path, contents):
        self.files[path] = contents

    async def exec(self, cmd, **kwargs):
        self.commands.append(cmd)
        if cmd[0] == "timeout":
            self.envs.append(kwargs["env"])
            if isinstance(self.result, Exception):
                raise self.result
            return self.result
        return ExecResult(success=True, returncode=0, stdout="", stderr="")


def run_claude(monkeypatch, result, arm="codecite"):
    state = AgentState(messages=[ChatMessageUser(content="do the task")])
    store, sb = Store(condition=arm), Sandbox(result)
    monkeypatch.setattr(task, "sandbox", lambda: sb)
    monkeypatch.setattr(task, "store", lambda: store)
    return asyncio.run(task.claude("claude-sonnet-5-5")(state)), store, sb


def finished(stdout, returncode=0):
    return ExecResult(success=returncode == 0, returncode=returncode, stdout=stdout, stderr="")


def test_a_finished_run_keeps_its_answer_and_measures(monkeypatch):
    state, store, sb = run_claude(monkeypatch, finished(stream(assistant("m1", USAGE), DONE)))
    assert isinstance(state.messages[-1], ChatMessageAssistant) and state.messages[-1].text == "Fixed it."
    assert store["agent_turns"] == 1 and "stopped" not in store
    assert json.loads(sb.files[claude_code.MCP]) == claude_code.MCP_CONFIG


def test_the_token_never_goes_in_an_execs_env(monkeypatch):
    # Inspect logs an exec's env; the token reaches the sandbox through compose.
    monkeypatch.setenv(claude_code.TOKEN, "secret")
    _, _, sb = run_claude(monkeypatch, finished(stream(DONE)))
    assert all(claude_code.TOKEN not in env and "secret" not in env.values() for env in sb.envs)


def test_the_baseline_arm_loses_codecite(monkeypatch):
    _, _, sb = run_claude(monkeypatch, finished(stream(DONE)), arm="baseline")
    assert ["rm", "-rf", "/opt/codecite"] in sb.commands and sb.files == {}
    assert not sb.envs[0]["PATH"].startswith("/opt/codecite")


@pytest.mark.parametrize("returncode", [124, 137])
def test_timeout_stopping_it_is_an_agent_timeout_with_measures(monkeypatch, returncode):
    _, store, _ = run_claude(monkeypatch, finished(stream(assistant("m1", USAGE)), returncode))
    assert store["stopped"] == "agent_timeout" and store["agent_turns"] == 1


def test_an_inspect_timeout_is_an_agent_timeout(monkeypatch):
    _, store, _ = run_claude(monkeypatch, TimeoutError())
    assert store["stopped"] == "agent_timeout"


def test_an_api_error_errors_the_sample(monkeypatch):
    out = stream({**DONE, "is_error": True, "result": "API Error: 429"})
    with pytest.raises(RuntimeError, match="API Error: 429"):
        run_claude(monkeypatch, finished(out, 1))


def test_the_agent_picks_claude_code_for_a_claude_code_model(monkeypatch):
    monkeypatch.setenv(claude_code.TOKEN, "t")
    seen = []
    model = type("Model", (), {"api": claude_code.ClaudeCode(model_name="claude-haiku-5-5"), "name": "claude-haiku-5-5"})()
    monkeypatch.setattr(task, "get_model", lambda: model)
    monkeypatch.setattr(task, "claude", lambda m: lambda s: seen.append(("claude", m)) or asyncio.sleep(0, s))
    monkeypatch.setattr(task, "pi", lambda: lambda s: seen.append(("pi",)) or asyncio.sleep(0, s))
    state = AgentState(messages=[])
    asyncio.run(task.coding_agent()(state))
    model.api = object()
    asyncio.run(task.coding_agent()(state))
    assert seen == [("claude", "claude-haiku-5-5"), ("pi",)]
