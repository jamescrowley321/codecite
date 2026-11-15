"""Claude Code as the eval's agent, on a Claude subscription.

    uv run inspect eval codecite_evals/fastapi_middleware --model claude-code/claude-sonnet-5-5

`claude-code/<model>` is an Inspect model that is never called: it names the
Claude model, and task.py's agent runs Claude Code (`claude -p`) in the
sandbox with it instead of pi. Claude Code calls the Anthropic API itself,
authenticated by CLAUDE_CODE_OAUTH_TOKEN (`claude setup-token`, a Pro or Max
subscription), so a run is billed to the subscription, not per token. The
token reaches the sandbox through compose's environment, never through an
exec's env or a written file, both of which Inspect writes to the log; and
only Claude Code samples' sandboxes get it, since panel.sh runs pi's panel in
a process without it. The agent's own commands can still read it: Claude
Code's subprocess scrubbing (CLAUDE_CODE_SUBPROCESS_ENV_SCRUB) brings its
sandbox and permission prompts with it, which would change the agent under
test. So the sandbox's only way out is the compose file's proxy, which lets
through api.anthropic.com alone, and CI scrubs the logs (scrub.py) before
uploading them.

Inspect sees none of Claude Code's model calls, so the agent reads the turns,
tokens, model time and codecite calls from its stream-json output and keeps
them in the sample's store, where report.py reads them.
"""

import json
import os

from inspect_ai.model import GenerateConfig, ModelAPI, ModelOutput, modelapi

from codecite_evals import assist
from codecite_evals.report import CLI, NATIVE

PROVIDER = "claude-code"
TOKEN = "CLAUDE_CODE_OAUTH_TOKEN"
# The same built-in tools as pi's: no WebFetch or WebSearch, which would
# reach the specs without codecite, and no subagents.
TOOLS = ["Bash", "Read", "Edit", "Write", "Glob", "Grep"]
MCP = "/root/.claude/codecite-mcp.json"
MCP_CONFIG = {"mcpServers": {"codecite": {"command": "/opt/codecite/bin/codecite", "args": ["mcp"]}}}
PROXY = "http://proxy:8888"
# Claude Code's stream-json goes to a file, not the exec's stdout, so a run
# that outgrows Inspect's output limit or loses its exec still leaves it.
OUTPUT = "/tmp/claude-code.jsonl"
ENV = {
    "HTTPS_PROXY": PROXY,
    # Claude Code refuses to skip permissions as root unless told it is in a sandbox.
    "IS_SANDBOX": "1",
    "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC": "1",
    "DISABLE_AUTOUPDATER": "1",
}


@modelapi(name=PROVIDER)
class ClaudeCode(ModelAPI):
    """Names the model for Claude Code; Inspect never generates with it."""

    claude_code = True

    def __init__(self, model_name: str, base_url=None, api_key=None, config: GenerateConfig = GenerateConfig(), **model_args):
        super().__init__(model_name, base_url, api_key, [TOKEN], config)
        if not os.environ.get(TOKEN):
            raise RuntimeError(f"{PROVIDER} models need {TOKEN} (`claude setup-token`)")

    async def generate(self, input, tools, tool_choice, config) -> ModelOutput:
        raise RuntimeError(f"{PROVIDER} models are run by Claude Code in the sandbox, not by Inspect")


def runs(model) -> bool:
    """Whether Claude Code runs this Inspect model. @modelapi wraps the class
    in a factory, so the instance is told by its marker, not isinstance."""
    return getattr(model.api, "claude_code", False) is True


def command(condition: str, prompt: str, model: str, timeout: int) -> list[str]:
    """Claude Code's command line for an arm, its stream-json written to
    OUTPUT. Only the codecite arm gets an MCP config; --strict-mcp-config
    keeps any other out of every arm. Only the assist arm gets the settings
    holding its grep and glob hook. `timeout` stops it a little before
    Inspect would."""
    mcp = ["--mcp-config", MCP] if condition == "codecite" else []
    settings = ["--settings", assist.SETTINGS_PATH] if condition == "assist" else []
    return [
        "sh", "-c", f'"$@" >{OUTPUT}', "sh",
        "timeout", "--kill-after=10", str(timeout),
        "claude", "-p", "--output-format", "stream-json", "--verbose", "--model", model,
        "--tools", ",".join(TOOLS), "--strict-mcp-config", *mcp, *settings,
        "--dangerously-skip-permissions", "--no-session-persistence", "--", prompt,
    ]  # fmt: skip


def events(stdout: str):
    for line in stdout.splitlines():
        try:
            event = json.loads(line)
        except ValueError:
            continue
        if isinstance(event, dict):
            yield event


# The most characters of one tool call's input kept in the store.
MAX_CALL_CHARS = 300


def tool_calls(stdout: str) -> list[dict]:
    """Every tool call in Claude Code's stream-json, in order, as {"name",
    "input"} with the input as JSON cut at MAX_CALL_CHARS. Inspect cuts a
    sandbox read's output at 10,000 characters in the log, so this is what
    shows which tools a run searched with."""
    calls = []
    for event in events(stdout):
        message = event.get("message") if event.get("type") == "assistant" else None
        if not isinstance(message, dict):
            continue
        for block in message.get("content") or []:
            if isinstance(block, dict) and block.get("type") == "tool_use":
                text = json.dumps(block.get("input"), ensure_ascii=False)
                calls.append({"name": str(block.get("name")), "input": text[:MAX_CALL_CHARS]})
    return calls


def measures(stdout: str) -> dict:
    """Turns, tokens, model time and codecite calls from Claude Code's
    stream-json output, which may stop short when it timed out. A turn is
    one API message, counted once though it streams a block at a time. Each
    message's usage is only what was known when it started streaming, so the
    tokens are the result's totals; a run cut off before its result has only
    those partial counts, a lower bound."""
    usage, calls, result = {}, 0, None
    for event in events(stdout):
        if event.get("type") == "result":
            result = event
        message = event.get("message") if event.get("type") == "assistant" else None
        if not isinstance(message, dict):
            continue
        usage[message.get("id") or len(usage)] = message.get("usage") or {}
        for block in message.get("content") or []:
            if not isinstance(block, dict) or block.get("type") != "tool_use":
                continue
            name, args = str(block.get("name")), block.get("input") or {}
            if name.startswith(NATIVE):
                calls += 1
            elif name == "Bash" and isinstance(args.get("command"), str):
                calls += bool(CLI.search(args["command"]))
    totals = [result["usage"]] if result and isinstance(result.get("usage"), dict) else list(usage.values())
    count = lambda *keys: sum(int(u.get(k) or 0) for u in totals for k in keys)
    api_ms = result.get("duration_api_ms") if result else None
    return {
        "agent_turns": len(usage),
        "agent_tokens": count("input_tokens", "output_tokens", "cache_creation_input_tokens", "cache_read_input_tokens"),
        "agent_output_tokens": count("output_tokens"),
        "agent_model_seconds": api_ms / 1000 if isinstance(api_ms, (int, float)) else None,
        "codecite_calls": calls,
    }


def failure(stdout: str) -> str | None:
    """Why Claude Code's run failed, when it ended in an error (an API error,
    a usage limit, a crash), or None when it finished or was cut off."""
    result = next((e for e in events(stdout) if e.get("type") == "result"), None)
    if result and (result.get("is_error") or result.get("subtype") != "success"):
        return str(result.get("result") or result.get("subtype") or "unknown error")
    return None


def answer(stdout: str) -> str:
    """Claude Code's final message, for the sample's transcript."""
    result = next((e for e in events(stdout) if e.get("type") == "result"), None)
    return str((result or {}).get("result") or "")
