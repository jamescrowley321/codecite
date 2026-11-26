"""The plugin arm: Claude Code with the codecite plugin loaded from
plugins/codecite, installed in the sandbox with --plugin-dir."""

import asyncio
import json

import pytest

from codecite_evals import claude_code, config, plugin, report, task


def test_plugin_arm_is_registered():
    assert config.ARMS["plugin"] == (("baseline", "baseline"), ("plugin", "plugin"))


def test_plugin_files_cover_the_plugin():
    files = plugin.files()
    assert f"{plugin.PLUGIN_DIR}/.claude-plugin/plugin.json" in files
    assert f"{plugin.PLUGIN_DIR}/skills/codecite/SKILL.md" in files
    assert f"{plugin.PLUGIN_DIR}/.mcp.json" in files
    assert f"{plugin.PLUGIN_DIR}/hooks/hooks.json" in files
    source = config.ROOT / "plugins/codecite/skills/codecite/SKILL.md"
    assert files[f"{plugin.PLUGIN_DIR}/skills/codecite/SKILL.md"] == source.read_text()


def test_plugin_arm_gets_the_tool_hooks():
    hooks = json.loads(plugin.files()[f"{plugin.PLUGIN_DIR}/hooks/hooks.json"])["hooks"]
    post, pre = hooks["PostToolUse"][0], hooks["PreToolUse"][0]
    assert post["hooks"][0]["command"] == "codecite hook claude post-tool-use"
    assert pre["hooks"][0]["command"] == "codecite hook claude pre-tool-use"
    # The arm's tools include every call the hooks match.
    for matcher in (post["matcher"], pre["matcher"]):
        assert set(matcher.split("|")) - {"MultiEdit"} <= set(claude_code.TOOLS)


def test_claude_command_loads_the_plugin_without_strict_mcp():
    cmd = claude_code.command("plugin", "p", "m", 60)
    assert cmd[cmd.index("--plugin-dir") + 1] == plugin.PLUGIN_DIR
    assert "--strict-mcp-config" not in cmd and "--mcp-config" not in cmd and "--settings" not in cmd


def test_only_the_plugin_arm_gets_the_skill_tool():
    def tools(condition):
        cmd = claude_code.command(condition, "p", "m", 60)
        return cmd[cmd.index("--tools") + 1].split(",")

    assert "Skill" in tools("plugin")
    for condition in ("baseline", "codecite", "assist"):
        assert "Skill" not in tools(condition)


def test_plugin_mcp_tools_count_as_codecite_calls():
    def call(name):
        block = {"type": "tool_use", "name": name, "input": {}}
        return json.dumps({"type": "assistant", "message": {"id": name, "content": [block]}})

    plugin_tool = "mcp__plugin_codecite_codecite__search"
    assert claude_code.measures(call(plugin_tool))["codecite_calls"] == 1
    assert claude_code.measures(call("mcp__codecite__search"))["codecite_calls"] == 1
    assert claude_code.measures(call("mcp__other__search"))["codecite_calls"] == 0
    assert report.is_codecite_tool(plugin_tool) and not report.is_codecite_tool("Bash")


def test_other_arms_keep_strict_mcp_and_no_plugin():
    for condition in ("baseline", "codecite", "assist"):
        cmd = claude_code.command(condition, "p", "m", 60)
        assert "--strict-mcp-config" in cmd and "--plugin-dir" not in cmd


def test_plugin_prompt_has_no_note(tmp_path):
    (tmp_path / "prompt.md").write_text("Fix it.\n")
    assert task.prompt(tmp_path, "plugin") == "Fix it.\n"


def test_pi_has_no_plugin_arm_yet():
    with pytest.raises(ValueError, match="pi package"):
        task.pi_command("plugin", "p")


class Sandbox:
    def __init__(self):
        self.files, self.commands = {}, []

    async def write_file(self, path, contents):
        self.files[path] = contents

    async def exec(self, cmd, **kwargs):
        self.commands.append(cmd)


def test_prepare_arm_writes_the_plugin_and_keeps_codecite_on_path():
    sb = Sandbox()
    env = asyncio.run(task.prepare_arm(sb, "plugin", claude_code.MCP, claude_code.MCP_CONFIG))
    # Only CodeGraph is stripped; codecite stays.
    assert all("/opt/codegraph" in " ".join(c) and "/opt/codecite" not in " ".join(c) for c in sb.commands)
    assert claude_code.MCP not in sb.files
    assert sb.files == plugin.files()
    assert env["PATH"].startswith("/opt/codecite/bin:") and env["HOME"] == "/root"
    assert env[plugin.CORPUS_ENV] == "eval"
    # No --strict-mcp-config, so keep the account's claude.ai connectors out.
    assert env["ENABLE_CLAUDEAI_MCP_SERVERS"] == "false"


def test_other_arms_keep_claudeai_connectors_setting_untouched():
    sb = Sandbox()
    env = asyncio.run(task.prepare_arm(sb, "baseline", claude_code.MCP, claude_code.MCP_CONFIG))
    assert "ENABLE_CLAUDEAI_MCP_SERVERS" not in env
