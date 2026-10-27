"""The spec-question task: pi answers one question about the identity specs
and cites the section, with and without codecite.

    uv run inspect eval codecite_evals/spec_questions --model openrouter/openai/gpt-6.1-sol -M strict_tools=false

One sample per question per arm, from tasks/spec-questions/questions.toml;
epochs repeat each. The agent and the arm split are task.py's: the same pi,
PI_TOOLS and PI_FLAGS, the baseline without /opt/codecite and the codecite arm
with it on PATH and a note in the prompt. Both arms have the spec documents
as files at the path the prompt names. A sample gets only its question's
text: the answer and accepted sections reach the sandbox with the scorer,
after the agent exits.
"""

import json
import tomllib
from pathlib import Path

from inspect_ai import Task, task
from inspect_ai.dataset import Sample
from inspect_ai.scorer import Score, Target, accuracy, scorer, stderr
from inspect_ai.solver import TaskState
from inspect_ai.util import sandbox, store

from codecite_evals import config
from codecite_evals.task import condition_to_store, pi, why_stopped

TASK_ID = "spec-questions"


def spec(task_dir: Path) -> dict:
    return json.loads((task_dir / "task.json").read_text())


def questions(task_dir: Path) -> list[dict]:
    return tomllib.loads((task_dir / spec(task_dir)["questions"]).read_text())["question"]


def codecite_note(task_dir: Path) -> str:
    """The codecite arm's note: what the corpus holds and how its citations
    map onto the answer file's `section:` line."""
    return (
        f"The `codecite` command is on your PATH. Its corpus `eval` indexes the specifications in `{spec(task_dir)['specs']}` "
        "(and the identity-model source, which this question doesn't need). "
        '`codecite search eval "<query>"` prints ranked results with citations, and '
        '`codecite read eval "<citation>"` prints the text a citation points at. '
        "A citation such as `rfc6749@sha256:535362fa3b4c#section-2.3.1` may go on the `section:` line as it is."
    )


def prompt(task_dir: Path, question: dict, arm: str) -> str:
    s = spec(task_dir)
    text = (
        (task_dir / s["prompt"]).read_text()
        .replace("{specs}", s["specs"])
        .replace("{answer_file}", s["answer"])
        .replace("{question}", question["text"])
    )
    return f"{text}\n\n{codecite_note(task_dir)}\n" if arm == "codecite" else text


def sample_id(arm: str, question_id: str) -> str:
    """`<arm>:<question id>`; report.py counts an arm's planned runs by the prefix."""
    return f"{arm}:{question_id}"


@task
def spec_questions(arms: str = "ab") -> Task:
    """arms="baseline" keeps only the baseline arm's samples, a calibration;
    arms="aa" runs two baseline arms."""
    if arms not in config.ARMS:
        raise ValueError(f"arms must be one of {', '.join(config.ARMS)}, not {arms!r}")
    task_dir = config.task_dir(TASK_ID)
    return Task(
        dataset=[
            Sample(
                id=sample_id(label, q["id"]),
                input=prompt(task_dir, q, condition),
                metadata={"arm": label, "condition": condition, "question": q["id"]},
            )
            for q in questions(task_dir)
            for label, condition in config.ARMS[arms]
        ],
        solver=[condition_to_store(), pi()],
        scorer=answer_and_section(TASK_ID),
        sandbox=("docker", str(config.COMPOSE_OFFLINE)),
    )


@scorer(metrics=[accuracy(), stderr()])
def answer_and_section(task_id: str):
    """Correct when the answer file's answer is an accepted form and its
    section an accepted one (score.py, run in the sandbox). The expected
    answer and score.py reach the sandbox only now, after the agent exits."""
    task_dir = config.task_dir(task_id)
    s = spec(task_dir)
    by_id = {q["id"]: q for q in questions(task_dir)}

    async def score(state: TaskState, target: Target) -> Score:
        sb = sandbox()
        q = by_id[state.metadata["question"]]
        await sb.write_file("/score/expected.json", json.dumps({"answer": q["answer"], "accepted": q["accepted"]}))
        await sb.write_file("/score/score.py", (task_dir / s["score"]).read_text())
        try:
            result = await sb.exec(["python3", "-I", "/score/score.py", "/score/expected.json", s["answer"]], timeout=60)
            verdict = json.loads(result.stdout) if result.success else None
        except (TimeoutError, ValueError):
            verdict = None
        if not isinstance(verdict, dict):
            verdict = {"passed": False, "reason": "scorer crashed", "answer": None, "section": None}
        stopped, sandbox_error = store().get("stopped"), store().get("sandbox_error")
        passed = verdict.get("passed") is True and not stopped
        return Score(
            value=1 if passed else 0,
            answer=verdict.get("answer"),
            explanation=f"{why_stopped(stopped, sandbox_error)}{verdict.get('reason')}",
            metadata={
                "stopped": stopped,
                "sandbox_error": sandbox_error,
                "question": q["id"],
                "section": verdict.get("section"),
                "reason": verdict.get("reason"),
            },
        )

    return score
