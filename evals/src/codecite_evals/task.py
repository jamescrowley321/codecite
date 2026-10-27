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
"""

import json
from pathlib import Path

from inspect_ai import Task, task
from inspect_ai.agent import Agent, AgentState, agent, sandbox_agent_bridge
from inspect_ai.dataset import Sample
from inspect_ai.scorer import Score, Target, accuracy, scorer, stderr
from inspect_ai.solver import Generate, TaskState, solver
from inspect_ai.util import OutputLimitExceededError, sandbox, store

from codecite_evals import config

ARMS = {"both": ("baseline", "codecite"), "baseline": ("baseline",)}
PROJECT = "/work/project"
IDENTITY_MODEL = "/opt/identity-model"
CODECITE = "/opt/codecite/bin/codecite"
CODECITE_TOOLS = ["mcp__codecite__search", "mcp__codecite__read"]
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


def codecite_note(task_dir: Path) -> str:
    """The codecite arm's note, naming what the task's `eval` corpus indexes."""
    corpus = json.loads((task_dir / "task.json").read_text())["corpus"]
    indexes = f"the identity-model source at `{IDENTITY_MODEL}`"
    if corpus.get("documents"):
        indexes += " and the OAuth 2.0, OpenID Connect and FAPI specifications it implements"
    return (
        f"codecite indexes {indexes} as its corpus `eval`. Its tools `mcp__codecite__search` (ranked "
        "results with citations) and `mcp__codecite__read` (the text a citation points at) search and "
        'read it. The `codecite` command on your PATH does the same: `codecite search eval "<query>"` and '
        '`codecite read eval "<citation>"`.'
    )


def prompt(task_dir: Path, arm: str) -> str:
    text = (task_dir / "prompt.md").read_text().replace("{identity_model}", IDENTITY_MODEL)
    return f"{text}\n\n{codecite_note(task_dir)}\n" if arm == "codecite" else text


def ab_task(task_id: str, arms: str = "both") -> Task:
    """One task's A/B eval: a sample per arm, scored by its hidden tests and
    checks. arms="baseline" keeps only the baseline arm, a calibration."""
    if arms not in ARMS:
        raise ValueError(f"arms must be one of {', '.join(ARMS)}, not {arms!r}")
    task_dir = config.task_dir(task_id)
    return Task(
        dataset=[Sample(id=arm, input=prompt(task_dir, arm), metadata={"arm": arm}) for arm in ARMS[arms]],
        solver=[arm_to_store(), pi()],
        scorer=hidden_tests_and_checks(task_id),
        sandbox=("docker", str(config.COMPOSE)),
    )


def pi_command(arm: str, prompt: str) -> list[str]:
    """pi's command line for an arm. --no-extensions drops the built-in
    extensions too, so the codecite arm loads only the MCP one back, and only
    it lists codecite's tools."""
    if arm == "codecite":
        flags, tools = [*PI_FLAGS, "-e", "builtin:mcp"], [*PI_TOOLS, *CODECITE_TOOLS]
    else:
        flags, tools = [*PI_FLAGS, "--no-mcp"], PI_TOOLS
    return ["pi", "-p", "--mode", "json", "--provider", "inspect", "--model", "inspect", *flags, "--tools", ",".join(tools), "--", prompt]


@task
def ab(task_id: str = "fastapi-middleware", arms: str = "both") -> Task:
    """Any task under tasks/ by its id; the eval workflow and panel.sh run this."""
    return ab_task(task_id, arms)


@task
def fastapi_middleware(arms: str = "both") -> Task:
    return ab_task("fastapi-middleware", arms)


@task
def client_auth_uri(arms: str = "both") -> Task:
    return ab_task("client-auth-uri", arms)


@task
def backchannel_logout(arms: str = "both") -> Task:
    return ab_task("backchannel-logout", arms)


@solver
def arm_to_store():
    """Hand the sample's arm to the agent, which sees only its messages."""

    async def solve(state: TaskState, generate: Generate) -> TaskState:
        store().set("arm", state.metadata["arm"])
        return state

    return solve


@agent
def pi() -> Agent:
    async def execute(state: AgentState) -> AgentState:
        sb = sandbox()
        arm = store().get("arm")
        if arm == "baseline":
            await sb.exec(["rm", "-rf", "/opt/codecite"])
        else:
            await sb.write_file(PI_MCP, json.dumps(MCP_CONFIG, indent=2))
        path = ("/opt/codecite/bin:" if arm == "codecite" else "") + "/usr/local/bin:/usr/bin:/bin"
        async with sandbox_agent_bridge(state) as bridge:
            try:
                result = await sb.exec(
                    pi_command(arm, state.messages[0].text),
                    cwd=PROJECT,
                    env={"PATH": path, "HOME": "/root"},
                    timeout=AGENT_TIMEOUT,
                )
            except TimeoutError:
                # Running out of time is the agent's failure, not infra: score
                # whatever it left behind. Erroring would retry the sample and
                # keep only the runs fast enough to finish.
                store().set("stopped", "agent_timeout")
                return bridge.state
            except (OutputLimitExceededError, UnicodeDecodeError):
                # pi's output overflowing the stream limit or failing to
                # decode is the agent's doing too, not the sandbox's.
                store().set("stopped", "agent_output")
                return bridge.state
            except Exception as e:
                # A sandbox failure (connection loss, process kill) is scored
                # the same way, so retrying doesn't keep only the runs that
                # survive it, but labelled apart so the scorecard shows it.
                store().set("stopped", "sandbox_error")
                store().set("sandbox_error", f"{type(e).__name__}: {e}"[:2000])
                return bridge.state
            if not result.success:
                raise RuntimeError(f"pi exited {result.returncode}: {result.stderr[-2000:]}")
            if error := last_turn_error(result.stdout):
                # A provider error is infra, not a wrong answer: error the
                # sample (Inspect can retry it) rather than score it.
                raise RuntimeError(f"pi's last turn failed: {error[:2000]}")
        return bridge.state

    return execute


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
