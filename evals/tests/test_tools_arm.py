"""The `tools` arm set: baseline, codecite and codegraph in one run."""

from codecite_evals import config, task


def test_the_tools_arm_runs_three_arms():
    assert config.ARMS["tools"] == (
        ("baseline", "baseline"),
        ("codecite", "codecite"),
        ("codegraph", "codegraph"),
    )


def test_one_sample_per_arm_named_by_its_arm():
    dataset = task.fastapi_middleware(arms="tools").dataset
    assert [s.id for s in dataset] == ["baseline", "codecite", "codegraph"]
    assert [s.metadata["condition"] for s in dataset] == ["baseline", "codecite", "codegraph"]


def test_each_note_names_only_its_own_tool():
    prompts = {s.id: s.input for s in task.fastapi_middleware(arms="tools").dataset}
    assert "codecite" in prompts["codecite"]
    assert "codegraph" not in prompts["codecite"].lower()
    assert "codegraph" in prompts["codegraph"]
    assert "codecite" not in prompts["codegraph"].lower()


def test_the_baseline_prompt_carries_no_note():
    prompts = {s.id: s.input for s in task.fastapi_middleware(arms="tools").dataset}
    prompt = (config.task_dir("fastapi-middleware") / "prompt.md").read_text()
    assert prompts["baseline"] == prompt.replace("{identity_model}", task.IDENTITY_MODEL)


def test_the_two_notes_are_of_a_length():
    """A note longer than the other would let note length, not the tool,
    explain a result: within a quarter of each other's length."""
    d = config.task_dir("fastapi-middleware")
    codecite, codegraph = task.codecite_note(d), task.codegraph_note(d)
    assert abs(len(codegraph) - len(codecite)) <= len(codecite) / 4


def test_only_the_two_tool_arms_get_a_note():
    d = config.task_dir("fastapi-middleware")
    assert task.note(d, "baseline") is None
    assert task.note(d, "assist") is None
    assert task.note(d, "codecite") == task.codecite_note(d)
    assert task.note(d, "codegraph") == task.codegraph_note(d)
