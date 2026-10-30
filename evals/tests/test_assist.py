"""The assist arm: codecite reaches the agent only through its grep and glob
hook. The hook runs here as the sandbox runs it, against a stand-in codecite."""

import asyncio
import json
import os
import stat
import subprocess
import sys

import pytest

from codecite_evals import assist, claude_code, config, task

GREP = {
    "tool_name": "Grep",
    "tool_input": {"pattern": "validate_logout_token"},
    "tool_response": {"mode": "files_with_matches", "filenames": ["src/a.py"], "numFiles": 1},
}


@pytest.fixture
def hook(tmp_path):
    """The hook script, its binary and log pointed at tmp_path, and a fake
    codecite that records its arguments and stdin and prints $BLOCK."""
    fake = tmp_path / "codecite"
    fake.write_text(
        "#!/bin/sh\n"
        f'printf "%s\\n" "$@" > {tmp_path}/args\n'
        f"cat > {tmp_path}/stdin\n"
        'printf "%s" "$BLOCK"\n'
        'exit "${CODE:-0}"\n'
    )
    fake.chmod(fake.stat().st_mode | stat.S_IEXEC)
    script = tmp_path / "hook.py"
    script.write_text(assist.HOOK.replace(assist.CODECITE, str(fake)).replace(assist.LOG, str(tmp_path / "log.jsonl")))

    def run(payload, block="", code=0):
        env = {**os.environ, "BLOCK": block, "CODE": str(code)}
        data = payload if isinstance(payload, str) else json.dumps(payload)
        return subprocess.run([sys.executable, "-I", str(script)], input=data, capture_output=True, text=True, env=env)

    return run


def test_the_hook_passes_the_call_to_assist_and_returns_its_block(hook, tmp_path):
    out = hook(GREP, block="codecite (eval) also found, not in this result:\nx.py#L1-L2 · function f · def f():\n")
    assert out.returncode == 0
    args = (tmp_path / "args").read_text().splitlines()
    assert args[:5] == ["assist", assist.CORPUS, "--tool", "grep", "--input"]
    assert json.loads(args[5]) == GREP["tool_input"]
    assert json.loads((tmp_path / "stdin").read_text()) == GREP["tool_response"]
    context = json.loads(out.stdout)["hookSpecificOutput"]
    assert context["hookEventName"] == "PostToolUse"
    assert context["additionalContext"].startswith(assist.HEADER) and "def f():" in context["additionalContext"]
    assert assist.measures((tmp_path / "log.jsonl").read_text()) == {"assist_calls": 1, "assist_blocks": 1, "assist_bytes": 85}


@pytest.mark.parametrize("name, tool", [("Grep", "grep"), ("Glob", "glob"), ("grep", "grep"), ("find", "glob")])
def test_the_hook_maps_both_agents_tools(hook, tmp_path, name, tool):
    hook({**GREP, "tool_name": name})
    assert (tmp_path / "args").read_text().splitlines()[3] == tool


def test_a_text_response_goes_to_stdin_as_it_is(hook, tmp_path):
    hook({**GREP, "tool_name": "grep", "tool_response": "src/a.py:3: def validate_logout_token"})
    assert (tmp_path / "stdin").read_text() == "src/a.py:3: def validate_logout_token"


@pytest.mark.parametrize("payload", ["not json", "[]", {**GREP, "tool_name": "Bash"}, {"tool_name": "Grep"}])
def test_the_hook_never_fails_on_a_call_it_cant_read(hook, payload):
    out = hook(payload)
    assert (out.returncode, out.stdout) == (0, "")


def test_an_empty_block_prints_nothing_but_counts_the_call(hook, tmp_path):
    out = hook(GREP)
    assert (out.returncode, out.stdout) == (0, "")
    assert assist.measures((tmp_path / "log.jsonl").read_text()) == {"assist_calls": 1, "assist_blocks": 0, "assist_bytes": 0}


def test_codecite_failing_prints_nothing(hook):
    out = hook(GREP, block="partial", code=3)
    assert (out.returncode, out.stdout) == (0, "")


def test_measures_skip_lines_they_cant_read():
    log = '{"tool": "grep", "bytes": 120}\n{"tool": "glob", "bytes": 0}\nnot json\n[]\n{"tool": "grep", "bytes": "x"}\n'
    assert assist.measures(log) == {"assist_calls": 2, "assist_blocks": 1, "assist_bytes": 120}
    assert assist.measures("") == {"assist_calls": 0, "assist_blocks": 0, "assist_bytes": 0}


def test_the_assist_arm_is_baseline_against_assist():
    assert config.ARMS["assist"] == (("baseline", "baseline"), ("assist", "assist"))
    first, second = task.fastapi_middleware(arms="assist").dataset
    # No note: the hook is the only way codecite reaches the agent.
    assert first.input == second.input == task.prompt(config.task_dir("fastapi-middleware"), "assist")
    assert "codecite" not in second.input.lower()


def test_pi_loads_only_the_assist_extension():
    cmd = task.pi_command("assist", "do it")
    assert cmd[cmd.index("-e") + 1] == assist.PI_EXTENSION_PATH and cmd.count("-e") == 1
    assert "--no-mcp" in cmd and "--no-extensions" in cmd
    assert cmd[cmd.index("--tools") + 1].split(",") == task.PI_TOOLS


def test_claude_code_gets_the_hook_settings_and_no_mcp():
    cmd = claude_code.command("assist", "do it", "claude-sonnet-5-5", 600)
    assert cmd[cmd.index("--settings") + 1] == assist.SETTINGS_PATH
    assert "--mcp-config" not in cmd and "--strict-mcp-config" in cmd
    base = claude_code.command("baseline", "do it", "claude-sonnet-5-5", 600)
    assert [a for a in cmd if a not in ("--settings", assist.SETTINGS_PATH)] == base


def test_the_settings_run_the_hook_on_grep_and_glob():
    (entry,) = assist.SETTINGS["hooks"]["PostToolUse"]
    assert entry["matcher"] == "Grep|Glob"
    assert entry["hooks"] == [{"type": "command", "command": f"python3 -I {assist.HOOK_PATH}", "timeout": 10}]


def test_the_extension_runs_the_same_hook():
    assert assist.HOOK_PATH in assist.PI_EXTENSION and assist.LOG not in assist.PI_EXTENSION
    assert 'grep: "grep"' in assist.PI_EXTENSION and 'find: "find"' in assist.PI_EXTENSION


class Sandbox:
    def __init__(self):
        self.files, self.commands = {}, []

    async def write_file(self, path, contents):
        self.files[path] = contents

    async def exec(self, cmd, **kwargs):
        self.commands.append(cmd)


def test_prepare_arm_keeps_codecite_off_path_and_writes_the_hook():
    sb = Sandbox()
    env = asyncio.run(task.prepare_arm(sb, "assist", claude_code.MCP, claude_code.MCP_CONFIG))
    assert sb.commands == [] and "/opt/codecite" not in env["PATH"]
    assert claude_code.MCP not in sb.files
    assert sb.files[assist.HOOK_PATH] == assist.HOOK
    assert json.loads(sb.files[assist.SETTINGS_PATH]) == assist.SETTINGS
    assert sb.files[assist.PI_EXTENSION_PATH] == assist.PI_EXTENSION
    # The hook calls codecite where the image put it.
    assert assist.CODECITE == task.CODECITE
