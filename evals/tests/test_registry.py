"""Every registered task builds from its own directory, without a sandbox."""

import json

import pytest

from codecite_evals import _registry, config, task

TASKS = {"fastapi_middleware": "fastapi-middleware", "backchannel_logout": "backchannel-logout"}


@pytest.mark.parametrize("name, task_id", TASKS.items())
def test_each_task_is_registered_and_builds(name, task_id):
    built = getattr(_registry, name)()
    assert [s.id for s in built.dataset] == ["baseline", "codecite"]
    assert built.sandbox.config == str(config.compose(task_id))
    prompt = (config.task_dir(task_id) / "prompt.md").read_text().replace("{identity_model}", task.IDENTITY_MODEL)
    baseline, codecite = (s.input for s in built.dataset)
    assert baseline == prompt
    assert codecite.startswith(prompt) and "codecite search" in codecite


def test_the_codecite_note_names_spec_documents_only_when_the_corpus_has_them():
    # The note may say documents are indexed, never which; the prompt is the
    # task's and names no spec.
    assert "specification documents" not in task.prompt("fastapi-middleware", "codecite")
    note = task.prompt("backchannel-logout", "codecite")
    assert "specification documents" in note
    assert "Back-Channel" not in note and "OpenID" not in note


@pytest.mark.parametrize("task_id", TASKS.values())
def test_the_score_command_is_the_tasks_own(task_id):
    want = json.loads((config.task_dir(task_id) / "task.json").read_text())["score"]
    cmd = task.score_command(task_id)
    assert cmd == [a.replace("{hidden}", "/score/hidden") for a in want]
    assert "{hidden}" not in " ".join(cmd)
