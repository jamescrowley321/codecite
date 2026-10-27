"""The A/B eval tasks on Inspect: pi in a Docker sandbox, with and without
codecite, on any model Inspect can reach.

    uv run inspect eval codecite_evals/fastapi_middleware --model openrouter/openai/gpt-6.1-sol -M strict_tools=false --epochs 2

Each sample is one arm; epochs repeat it. pi's model calls go through
Inspect's agent bridge, so Inspect records every call and its tokens. Every
task is built the same way from its directory under evals/tasks/.
"""

import json

from inspect_ai import Task, task
from inspect_ai.agent import Agent, AgentState, agent, sandbox_agent_bridge
from inspect_ai.dataset import Sample
from inspect_ai.scorer import Score, Target, accuracy, scorer, stderr
from inspect_ai.solver import Generate, TaskState, solver
from inspect_ai.util import OutputLimitExceededError, sandbox, store

from codecite_evals import config

PROJECT = "/work/project"
IDENTITY_MODEL = "/opt/identity-model"
CODECITE_NOTE = (
    "The `codecite` command is on your PATH. Its corpus `eval` indexes the identity-model source at "
    f"`{IDENTITY_MODEL}`{{documents}}. "
    '`codecite search eval "<query>"` prints ranked results with citations, and '
    '`codecite read eval "<citation>"` prints the text a citation points at.'
)
# The note says the corpus holds documents, never which: naming the spec
# would tell the codecite arm what the prompt keeps from both arms.
DOCUMENTS_NOTE = " and the specification documents it implements"
PI_TOOLS = "read,bash,edit,write,grep,find,ls"
PI_FLAGS = ["--no-session", "--no-mcp", "--no-extensions", "--no-skills", "--no-prompt-templates", "--no-themes", "--no-approve"]
AGENT_TIMEOUT = 600


def task_json(task_id: str) -> dict:
    return json.loads((config.task_dir(task_id) / "task.json").read_text())


def prompt(task_id: str, arm: str) -> str:
    text = (config.task_dir(task_id) / "prompt.md").read_text().replace("{identity_model}", IDENTITY_MODEL)
    if arm != "codecite":
        return text
    documents = DOCUMENTS_NOTE if task_json(task_id)["corpus"].get("documents") else ""
    return f"{text}\n\n{CODECITE_NOTE.format(documents=documents)}\n"


def score_command(task_id: str) -> list[str]:
    """The task's hidden-test command from task.json, run on the copied tests."""
    return [a.replace("{hidden}", "/score/hidden") for a in task_json(task_id)["score"]]


def eval_task(task_id: str) -> Task:
    return Task(
        dataset=[Sample(id=arm, input=prompt(task_id, arm), metadata={"arm": arm}) for arm in ("baseline", "codecite")],
        solver=[arm_to_store(), pi()],
        scorer=hidden_tests_and_checks(task_id),
        sandbox=("docker", str(config.compose(task_id))),
    )


@task
def fastapi_middleware() -> Task:
    return eval_task("fastapi-middleware")


@task
def backchannel_logout() -> Task:
    return eval_task("backchannel-logout")


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
        path = ("/opt/codecite/bin:" if arm == "codecite" else "") + "/usr/local/bin:/usr/bin:/bin"
        async with sandbox_agent_bridge(state) as bridge:
            try:
                result = await sb.exec(
                    ["pi", "-p", "--mode", "json", "--provider", "inspect", "--model", "inspect", *PI_FLAGS, "--tools", PI_TOOLS, "--", state.messages[0].text],
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


@scorer(metrics=[accuracy(), stderr()])
def hidden_tests_and_checks(task_id: str):
    """Correct when the hidden tests and every deterministic check pass. The
    tests and checks reach the sandbox only now, after the agent has exited."""
    root = config.task_dir(task_id)
    tests_cmd = score_command(task_id)

    async def score(state: TaskState, target: Target) -> Score:
        sb = sandbox()
        for f in (root / "hidden").glob("*.py"):
            await sb.write_file(f"/score/hidden/{f.name}", f.read_text())
        await sb.write_file("/score/checks.py", (root / "checks.py").read_text())
        start = root / "project"
        for f in start.rglob("*"):
            if f.is_file():
                text = f.read_text().replace("@IDENTITY_MODEL@", IDENTITY_MODEL)
                await sb.write_file(f"/score/start/{f.relative_to(start)}", text)

        # Both run the agent's code, so a hang is the agent's failure, scored
        # like any other; letting the timeout raise would retry the sample.
        try:
            tests = await sb.exec(
                tests_cmd,
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
        why = {
            "agent_timeout": "agent timed out; ",
            "agent_output": "agent's output overflowed or didn't decode; ",
            "sandbox_error": f"sandbox failed ({sandbox_error}); ",
        }.get(stopped, "")
        return Score(
            value=1 if passed else 0,
            explanation=f"{why}hidden tests {'passed' if tests_passed else 'failed'}; failed checks: {', '.join(failed) or 'none'}",
            metadata={"stopped": stopped, "sandbox_error": sandbox_error, "tests_passed": tests_passed, "tests_tail": tests_tail, "checks": results},
        )

    return score
