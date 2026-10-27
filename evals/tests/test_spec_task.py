"""The spec-question task's samples, prompts and scorer, without a model key
or a live sandbox: the scorer's sandbox runs score.py on this machine."""

import asyncio
import subprocess
import sys
from pathlib import Path

import pytest
from inspect_ai.util import ExecResult

from codecite_evals import _registry, config, spec_task
from codecite_evals.spec_task import TASK_ID

TASK_DIR = config.task_dir(TASK_ID)
QUESTIONS = spec_task.questions(TASK_DIR)
ANSWER = spec_task.spec(TASK_DIR)["answer"]


class Store(dict):
    def set(self, key, value):
        self[key] = value


def test_is_registered():
    assert callable(_registry.spec_questions)


def test_one_sample_per_question_per_arm():
    dataset = spec_task.spec_questions().dataset
    assert len(dataset) == 2 * len(QUESTIONS)
    assert len({s.id for s in dataset}) == len(dataset)
    for s in dataset:
        arm, _, question = s.id.partition(":")
        assert s.metadata == {"arm": arm, "question": question}
        assert arm in ("baseline", "codecite")


def test_arms_baseline_keeps_only_the_baseline_samples():
    dataset = spec_task.spec_questions(arms="baseline").dataset
    assert dataset and {s.metadata["arm"] for s in dataset} == {"baseline"}
    with pytest.raises(ValueError, match="arms must be one of both, baseline"):
        spec_task.spec_questions(arms="codecite")


def test_a_sample_holds_no_answer():
    # The answers reach the sandbox only with the scorer.
    for s in spec_task.spec_questions().dataset:
        assert not s.target
        assert set(s.metadata) == {"arm", "question"}


def test_the_arms_get_the_same_question_and_the_codecite_arm_a_note():
    q = QUESTIONS[0]
    baseline, codecite = spec_task.prompt(TASK_DIR, q, "baseline"), spec_task.prompt(TASK_DIR, q, "codecite")
    assert q["text"] in baseline and ANSWER in baseline and "/opt/specs" in baseline
    assert "{" not in baseline
    assert "codecite" not in baseline.lower()
    assert codecite.startswith(baseline) and "`codecite search eval" in codecite


class LocalSandbox:
    """Files under a temporary root; exec runs score.py here, as the sandbox would."""

    def __init__(self, root: Path, answer: str | None):
        self.root = root
        if answer is not None:
            self.path(ANSWER).parent.mkdir(parents=True, exist_ok=True)
            self.path(ANSWER).write_text(answer)

    def path(self, p):
        return self.root / p.lstrip("/")

    async def write_file(self, path, contents):
        self.path(path).parent.mkdir(parents=True, exist_ok=True)
        self.path(path).write_text(contents)

    async def exec(self, cmd, **kwargs):
        assert cmd[:3] == ["python3", "-I", "/score/score.py"]
        out = subprocess.run([sys.executable, "-I", *(str(self.path(a)) for a in cmd[2:])], capture_output=True, text=True)
        return ExecResult(success=out.returncode == 0, returncode=out.returncode, stdout=out.stdout, stderr=out.stderr)


class Broken(LocalSandbox):
    async def exec(self, cmd, **kwargs):
        return ExecResult(success=False, returncode=1, stdout="", stderr="Traceback")


def score(monkeypatch, tmp_path, answer, store=None, sandbox=LocalSandbox):
    q = QUESTIONS[0]
    sb = sandbox(tmp_path, answer)
    monkeypatch.setattr(spec_task, "sandbox", lambda: sb)
    monkeypatch.setattr(spec_task, "store", lambda: store if store is not None else Store())
    state = type("State", (), {"metadata": {"arm": "baseline", "question": q["id"]}})()
    return asyncio.run(spec_task.answer_and_section(TASK_ID)(state, None)), sb


def reference(q):
    return f"answer: {q['answer'][0]}\nsection: {q['accepted'][0]}\n"


def test_a_right_answer_and_section_pass(monkeypatch, tmp_path):
    result, sb = score(monkeypatch, tmp_path, reference(QUESTIONS[0]))
    assert result.value == 1, result.explanation
    assert result.metadata["question"] == QUESTIONS[0]["id"]
    # The expected answer reached the sandbox at scoring, from questions.toml.
    assert QUESTIONS[0]["answer"][0] in sb.path("/score/expected.json").read_text()


def test_a_wrong_answer_fails(monkeypatch, tmp_path):
    q = QUESTIONS[0]
    result, _ = score(monkeypatch, tmp_path, f"answer: not-it\nsection: {q['accepted'][0]}\n")
    assert result.value == 0 and "wrong answer" in result.explanation


def test_no_answer_file_fails(monkeypatch, tmp_path):
    result, _ = score(monkeypatch, tmp_path, None)
    assert result.value == 0 and "no answer file" in result.explanation


@pytest.mark.parametrize("stopped", ["agent_timeout", "agent_output", "sandbox_error"])
def test_an_agent_that_did_not_finish_fails_even_when_right(monkeypatch, tmp_path, stopped):
    store = Store(stopped=stopped, sandbox_error="RuntimeError: container killed" if stopped == "sandbox_error" else None)
    result, _ = score(monkeypatch, tmp_path, reference(QUESTIONS[0]), store)
    assert result.value == 0
    assert result.metadata["stopped"] == stopped
    assert result.explanation.endswith("correct")


def test_a_crashed_scorer_fails(monkeypatch, tmp_path):
    result, _ = score(monkeypatch, tmp_path, reference(QUESTIONS[0]), sandbox=Broken)
    assert result.value == 0 and "scorer crashed" in result.explanation
