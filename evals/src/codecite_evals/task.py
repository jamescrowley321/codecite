"""The A/B eval on Inspect: pi in a Docker sandbox, with and without
codecite, on any model Inspect can reach. Any task in evals/tasks/ runs as
codecite_evals/ab -T task_id=<id>, and each registered here also runs as
codecite_evals/<name>:

    uv run inspect eval codecite_evals/fastapi_middleware --model openrouter/openai/gpt-6.1-sol -M strict_tools=false --epochs 2

`-T arms=baseline` runs only the baseline arm, to calibrate a task's
difficulty without watching codecite's (evals/README.md).

The sandbox image holds one task's project and corpus: stage it with
`inspect/prepare.sh <task-id>` before building. Each sample is one arm; epochs
repeat it. pi's model calls go through Inspect's agent bridge, so Inspect
records every call, its tool list and its tokens. The codecite arm gives pi
codecite's `search` and `read` as tools of their own, from `codecite mcp`
through pi's built-in MCP support, and keeps the CLI on PATH; the baseline
arm has no MCP and no codecite.

`-T arms=aa` runs an A/A round instead: two baseline arms with the same prompt
and sandbox, labelled apart, so the scorecard shows what noise alone does.
`-T arms=baseline` runs only the baseline arm, to calibrate a task's
difficulty without watching codecite's (evals/README.md).
"""

import json
import os
import re
from pathlib import Path
from time import monotonic

from inspect_ai import Task, task
from inspect_ai.agent import Agent, AgentState, agent, sandbox_agent_bridge
from inspect_ai.dataset import Sample
from inspect_ai.model import ChatMessageAssistant, get_model
from inspect_ai.scorer import Score, Target, accuracy, scorer, stderr
from inspect_ai.solver import Generate, TaskState, solver
from inspect_ai.util import OutputLimitExceededError, sandbox, store

from codecite_evals import claude_code, config, scrub

ARMS = {"both": ("baseline", "codecite"), "baseline": ("baseline",)}
PROJECT = "/work/project"
IDENTITY_MODEL = "/opt/identity-model"
CODECITE = "/opt/codecite/bin/codecite"
CODECITE_TOOLS = ["mcp__codecite__search", "mcp__codecite__read", "mcp__codecite__requirements"]
PI_TOOLS = ["read", "bash", "edit", "write", "grep", "find", "ls"]
PI_FLAGS = ["--no-session", "--no-extensions", "--no-skills", "--no-prompt-templates", "--no-themes", "--no-approve"]
# pi's user-level MCP config. `direct` exposure declares the tools to the
# model next to the built-in ones, with the server's descriptions, rather
# than behind codemode or tool_search.
PI_MCP = "/root/.pi/agent/mcp.json"
MCP_CONFIG = {
    "mcpServers": {
        "codecite": {
            "command": CODECITE,
            "args": ["mcp"],
            "exposure": "direct",
        }
    },
    "autoEnableCodemode": False,
}
AGENT_TIMEOUT = 600


def snippet(corpus: str) -> str:
    """The AGENTS.md snippet from docs/usage.md, for `corpus`, without its
    heading's fence. Fails loudly if the markers or the fence are missing."""
    text = (config.ROOT / "docs" / "usage.md").read_text()
    m = re.search(r"<!-- agent-snippet -->\s*```markdown\n(.*?)```\s*<!-- /agent-snippet -->", text, re.S)
    if not m:
        raise ValueError("docs/usage.md has no <!-- agent-snippet --> block holding a markdown fence")
    return m.group(1).strip().replace("<CORPUS>", corpus)


def codecite_note(task_dir: Path) -> str:
    """The codecite arm's note, naming what the task's `eval` corpus indexes."""
    corpus = json.loads((task_dir / "task.json").read_text())["corpus"]
    indexes = f"the identity-model source at `{IDENTITY_MODEL}`"
    if corpus.get("documents"):
        indexes += " and the OAuth 2.0, OpenID Connect and FAPI specifications it implements"
    mcp = ", ".join(f"`{tool}`" for tool in CODECITE_TOOLS)
    return (
        f"codecite indexes {indexes} as its corpus `eval`.\n\n{snippet('eval')}\n\n"
        f"The same commands are the MCP tools {mcp}."
    )


def prompt(task_dir: Path, condition: str) -> str:
    text = (task_dir / "prompt.md").read_text().replace("{identity_model}", IDENTITY_MODEL)
    return f"{text}\n\n{codecite_note(task_dir)}\n" if condition == "codecite" else text


def ab_task(task_id: str, arms: str = "ab") -> Task:
    """One task's eval: a sample per arm (config.ARMS), scored by its hidden
    tests and checks. arms="baseline" keeps only the baseline arm, a
    calibration; arms="aa" runs two baseline arms."""
    if arms not in config.ARMS:
        raise ValueError(f"arms must be one of {', '.join(config.ARMS)}, not {arms!r}")
    task_dir = config.task_dir(task_id)
    return Task(
        dataset=[
            Sample(id=label, input=prompt(task_dir, condition), metadata={"arm": label, "condition": condition})
            for label, condition in config.ARMS[arms]
        ],
        solver=[condition_to_store(), coding_agent()],
        scorer=hidden_tests_and_checks(task_id),
        sandbox=("docker", str(config.COMPOSE)),
    )


def pi_command(condition: str, prompt: str) -> list[str]:
    """pi's command line for an arm. --no-extensions drops the built-in
    extensions too, so the codecite arm loads only the MCP one back, and only
    it lists codecite's tools."""
    if condition == "codecite":
        flags, tools = [*PI_FLAGS, "-e", "builtin:mcp"], [*PI_TOOLS, *CODECITE_TOOLS]
    else:
        flags, tools = [*PI_FLAGS, "--no-mcp"], PI_TOOLS
    return ["pi", "-p", "--mode", "json", "--provider", "inspect", "--model", "inspect", *flags, "--tools", ",".join(tools), "--", prompt]


@task
def ab(task_id: str = "fastapi-middleware", arms: str = "ab") -> Task:
    """Any task under tasks/ by its id; the eval workflow and panel.sh run this."""
    return ab_task(task_id, arms)


@task
def fastapi_middleware(arms: str = "ab") -> Task:
    return ab_task("fastapi-middleware", arms)


@task
def client_auth_uri(arms: str = "ab") -> Task:
    return ab_task("client-auth-uri", arms)


@task
def backchannel_logout(arms: str = "ab") -> Task:
    return ab_task("backchannel-logout", arms)


@solver
def condition_to_store():
    """Hand the sample's condition to the agent, which sees only its messages."""

    async def solve(state: TaskState, generate: Generate) -> TaskState:
        store().set("condition", state.metadata["condition"])
        return state

    return solve


@agent
def coding_agent() -> Agent:
    """The sample's agent: Claude Code for a claude-code/ model
    (claude_code.py), else pi through Inspect's agent bridge."""

    async def execute(state: AgentState) -> AgentState:
        model = get_model()
        if claude_code.runs(model):
            return await claude(model.name)(state)
        return await pi()(state)

    return execute


async def prepare_arm(sb, condition: str, mcp_path: str, mcp_config: dict) -> dict[str, str]:
    """Take codecite out of the baseline arm's sandbox, or give the codecite
    arm the agent's MCP config, and return the arm's base env."""
    if condition == "baseline":
        await sb.exec(["rm", "-rf", "/opt/codecite"])
    else:
        await sb.write_file(mcp_path, json.dumps(mcp_config, indent=2))
    path = ("/opt/codecite/bin:" if condition == "codecite" else "") + "/usr/local/bin:/usr/bin:/bin"
    return {"PATH": path, "HOME": "/root"}


async def run_agent(sb, cmd: list[str], env: dict[str, str], timeout: int):
    """Run the agent's command; None when it stopped early, with the reason
    in the store."""
    try:
        return await sb.exec(cmd, cwd=PROJECT, env=env, timeout=timeout)
    except TimeoutError:
        # Running out of time is the agent's failure, not infra: score
        # whatever it left behind. Erroring would retry the sample and keep
        # only the runs fast enough to finish.
        store().set("stopped", "agent_timeout")
    except (OutputLimitExceededError, UnicodeDecodeError):
        # The agent's output overflowing the stream limit or failing to
        # decode is the agent's doing too, not the sandbox's.
        store().set("stopped", "agent_output")
    except Exception as e:
        # A sandbox failure (connection loss, process kill) is scored the
        # same way, so retrying doesn't keep only the runs that survive it,
        # but labelled apart so the scorecard shows it.
        store().set("stopped", "sandbox_error")
        store().set("sandbox_error", withheld(f"{type(e).__name__}: {e}"[:2000]))
    return None


@agent
def pi() -> Agent:
    async def execute(state: AgentState) -> AgentState:
        sb = sandbox()
        condition = store().get("condition")
        env = await prepare_arm(sb, condition, PI_MCP, MCP_CONFIG)
        async with sandbox_agent_bridge(state) as bridge:
            result = await run_agent(sb, pi_command(condition, state.messages[0].text), env, AGENT_TIMEOUT)
            if result is None:
                return bridge.state
            if not result.success:
                raise RuntimeError(f"pi exited {result.returncode}: {result.stderr[-2000:]}")
            if error := last_turn_error(result.stdout):
                # A provider error is infra, not a wrong answer: error the
                # sample (Inspect can retry it) rather than score it.
                raise RuntimeError(f"pi's last turn failed: {error[:2000]}")
        return bridge.state

    return execute


@agent
def claude(model: str) -> Agent:
    """Claude Code on `model`. Its measures reach the store even when it
    stopped early, from the output file it left."""

    async def execute(state: AgentState) -> AgentState:
        sb = sandbox()
        condition = store().get("condition")
        env = {**await prepare_arm(sb, condition, claude_code.MCP, claude_code.MCP_CONFIG), **claude_code.ENV}
        cmd = claude_code.command(condition, state.messages[0].text, model, AGENT_TIMEOUT)
        started = monotonic()
        result = await run_agent(sb, cmd, env, AGENT_TIMEOUT + 30)
        ran = monotonic() - started
        try:
            stdout = await sb.read_file(claude_code.OUTPUT)
        except Exception:
            stdout = ""
        for key, value in claude_code.measures(stdout).items():
            store().set(key, value)
        if result is None:
            return state
        if result.returncode == 124 or (result.returncode == 137 and ran >= AGENT_TIMEOUT):
            # `timeout` stopped it (SIGTERM, then SIGKILL if that didn't).
            store().set("stopped", "agent_timeout")
            return state
        if result.returncode == 137:
            # Killed before its time: the sandbox's doing (an OOM kill), as in run_agent.
            store().set("stopped", "sandbox_error")
            store().set("sandbox_error", f"claude was killed after {ran:.0f}s")
            return state
        if error := claude_code.failure(stdout):
            # An API error or a usage limit is infra, as pi's provider errors are.
            raise RuntimeError(f"Claude Code failed: {withheld(error[:2000])}")
        if not result.success:
            raise RuntimeError(f"claude exited {result.returncode}: {withheld(result.stderr[-2000:])}")
        state.messages.append(ChatMessageAssistant(content=claude_code.answer(stdout)))
        return state

    return execute


def withheld(text: str) -> str:
    """The text, unless it holds the Claude token: an error's text reaches
    the job's public console, which CI's masking covers only verbatim."""
    token = os.environ.get(claude_code.TOKEN)
    if token and scrub.found(text.encode(), scrub.needles(token)):
        return "(withheld: it held the Claude token)"
    return text


def last_turn_error(stdout: str) -> str | None:
    """pi's errorMessage when its last assistant message ended in an error."""
    last = None
    for line in stdout.splitlines():
        try:
            event = json.loads(line)
        except ValueError:
            continue
        if not isinstance(event, dict):
            continue
        message = event.get("message") if event.get("type") == "message_end" else None
        if isinstance(message, dict) and message.get("role") == "assistant":
            last = message
    if last and last.get("stopReason") == "error":
        return str(last.get("errorMessage") or "unknown error")
    return None


def why_stopped(stopped: str | None, sandbox_error: str | None) -> str:
    """The start of a score's explanation when the agent didn't finish: such
    a sample is scored on what it left behind, but can't pass."""
    return {
        "agent_timeout": "agent timed out; ",
        "agent_output": "agent's output overflowed or didn't decode; ",
        "sandbox_error": f"sandbox failed ({sandbox_error}); ",
    }.get(stopped, "")


@scorer(metrics=[accuracy(), stderr()])
def hidden_tests_and_checks(task_id: str):
    """Correct when the hidden tests and every deterministic check pass. The
    tests and checks reach the sandbox only now, after the agent has exited.
    Takes the task's id, not its path, so the eval log records the id."""
    task_dir = config.task_dir(task_id)

    async def score(state: TaskState, target: Target) -> Score:
        sb = sandbox()
        for f in (task_dir / "hidden").glob("*.py"):
            await sb.write_file(f"/score/hidden/{f.name}", f.read_text())
        await sb.write_file("/score/checks.py", (task_dir / "checks.py").read_text())
        start = task_dir / "project"
        for f in start.rglob("*"):
            if f.is_file():
                text = f.read_text().replace("@IDENTITY_MODEL@", IDENTITY_MODEL)
                await sb.write_file(f"/score/start/{f.relative_to(start)}", text)

        # Both run the agent's code, so a hang is the agent's failure, scored
        # like any other; letting the timeout raise would retry the sample.
        try:
            tests = await sb.exec(
                ["uv", "run", "--with", "pytest==9.1.1", "--with", "httpx==0.28.1", "pytest", "-q", "-p", "no:cacheprovider", "/score/hidden"],
                cwd=PROJECT,
                env={"UV_OFFLINE": "1"},
                timeout=300,
            )
            tests_passed, tests_tail = tests.success, tests.stdout[-1500:]
        except TimeoutError:
            tests_passed, tests_tail = False, "hidden tests timed out"
        try:
            checks = await sb.exec(["python3", "-I", "/score/checks.py", "/score/start", PROJECT], timeout=60)
            results = json.loads(checks.stdout)["checks"] if checks.success else []
        except (TimeoutError, ValueError, KeyError, TypeError):
            results = []
        failed = [c["name"] for c in results if not c["passed"]] if results else ["checks-crashed"]
        stopped, sandbox_error = store().get("stopped"), store().get("sandbox_error")
        passed = tests_passed and not failed and not stopped
        why = why_stopped(stopped, sandbox_error)
        return Score(
            value=1 if passed else 0,
            explanation=f"{why}hidden tests {'passed' if tests_passed else 'failed'}; failed checks: {', '.join(failed) or 'none'}",
            metadata={"stopped": stopped, "sandbox_error": sandbox_error, "tests_passed": tests_passed, "tests_tail": tests_tail, "checks": results},
        )

    return score
