"""The scorecard counts timeouts and sandbox errors apart and codecite's use,
and reports a baseline-only run as a calibration, on logs built here."""

from datetime import datetime, timezone

import pytest
from inspect_ai.event import ModelEvent
from inspect_ai.log import EvalConfig, EvalDataset, EvalError, EvalLog, EvalSample, EvalSpec, write_eval_log
from inspect_ai.model import ChatMessageAssistant, ChatMessageUser, GenerateConfig, ModelOutput, ModelUsage
from inspect_ai.scorer import Score
from inspect_ai.tool import ToolCall

from codecite_evals.report import codecite_calls, logs, scorecard

EPOCHS = 4
NATIVE = ToolCall(id="n", function="mcp__codecite__search", arguments={"query": "TokenValidationMiddleware"})


def bash(command):
    return ToolCall(id="b", function="bash", arguments={"command": command})


def model_call(tool_calls=None, text=""):
    """One model call as the agent bridge logs it: its output's tool calls."""
    message = ChatMessageAssistant(content=text, tool_calls=tool_calls or None)
    return ModelEvent(model="m", input=[], tools=[], tool_choice="auto", config=GenerateConfig(), output=ModelOutput.from_message(message))


def sample(arm, epoch, passed=False, stopped=None, error=None, calls=None, question=None, turns=0):
    """A scored sample. Its codecite arm calls codecite natively unless calls
    (the tool calls of its one model call) says otherwise."""
    if calls is None:
        calls = [NATIVE] if arm == "codecite" else []
    metadata = {"stopped": stopped} if stopped else {}
    if error:
        metadata["sandbox_error"] = error
    return EvalSample(
        id=f"{arm}:{question}" if question else arm,
        epoch=epoch,
        input="",
        target="",
        messages=[ChatMessageUser(content="do the task")] + [ChatMessageAssistant(content="ok") for _ in range(turns)],
        metadata={"arm": arm, "question": question} if question else {"arm": arm},
        scores={"hidden_tests_and_checks": Score(value=1 if passed else 0, metadata=metadata)},
        model_usage={"m": ModelUsage(total_tokens=1000)},
        total_time=60.0,
        events=[model_call(calls)],
    )


def log(model, samples, status="success", created=None, task="fastapi_middleware", sample_ids=None, epochs=EPOCHS, arms=None):
    return EvalLog(
        status=status,
        eval=EvalSpec(
            created=created or datetime.now(timezone.utc).isoformat(),
            task=task,
            dataset=EvalDataset(samples=len(sample_ids) if sample_ids else None, sample_ids=sample_ids),
            model=model,
            task_args={"arms": arms} if arms else {},
            config=EvalConfig(epochs=epochs),
        ),
        samples=samples,
    )


# m1 is clean. m2's codecite arm loses two runs to sandbox errors.
PANEL = [
    log("openrouter/m1", [
        sample("baseline", 1, passed=True),
        sample("baseline", 2, passed=True),
        sample("baseline", 3, stopped="agent_timeout"),
        sample("baseline", 4),
        sample("codecite", 1, passed=True),
        sample("codecite", 2, passed=True),
        sample("codecite", 3, passed=True),
        sample("codecite", 4, stopped="agent_timeout"),
    ]),
    log("openrouter/m2", [sample("baseline", e, passed=True) for e in range(1, EPOCHS + 1)] + [
        sample("codecite", 1, passed=True),
        sample("codecite", 2, stopped="agent_timeout"),
        sample("codecite", 3, stopped="sandbox_error", error="ConnectionResetError: connection lost"),
        sample("codecite", 4, stopped="sandbox_error", error="RuntimeError: container killed"),
    ]),
]


def table(out):
    lines = [line for line in out.splitlines() if line.startswith("|")]
    header = [c.strip() for c in lines[0].strip("|").split("|")]
    rows = [dict(zip(header, (c.strip() for c in line.strip("|").split("|")))) for line in lines[2:]]
    return header, {(r["model"], r["arm"]): r for r in rows}


def test_separate_columns():
    header, rows = table(scorecard(PANEL))
    assert {"fails", "timeouts", "sandbox errors"} <= set(header)
    assert (rows[("m1", "baseline")]["timeouts"], rows[("m1", "baseline")]["sandbox errors"]) == ("1", "0")
    assert (rows[("m2", "codecite")]["timeouts"], rows[("m2", "codecite")]["sandbox errors"]) == ("1", "2")


def test_sandbox_errors_are_listed():
    out = scorecard(PANEL)
    assert "- m2, codecite, epoch 3: `ConnectionResetError: connection lost`" in out
    assert "- m2, codecite, epoch 4: `RuntimeError: container killed`" in out


def test_listed_errors_are_short_and_fenced():
    long = "RuntimeError: `docker` said\n" + "x" * 1000
    out = scorecard([log("openrouter/m", [sample("baseline", 1, stopped="sandbox_error", error=long)])])
    (line,) = [line for line in out.splitlines() if line.startswith("- ")]
    assert line.startswith("- m, baseline, epoch 1: `RuntimeError: 'docker' said xxx")
    assert line.endswith("…`") and len(line) < 300


def test_columns_and_list_add_up_to_the_failures():
    out = scorecard(PANEL)
    _, rows = table(out)
    listed = [line for line in out.splitlines() if line.startswith("- ")]
    assert len(listed) == sum(int(r["sandbox errors"]) for r in rows.values())
    for (model, arm), r in rows.items():
        passes = round(float(r["pass"].rstrip("%")) / 100 * int(r["runs"]))
        assert int(r["fails"]) == int(r["runs"]) - passes
        assert int(r["timeouts"]) + int(r["sandbox errors"]) <= int(r["fails"])
        assert len([line for line in listed if line.startswith(f"- {model}, {arm},")]) == int(r["sandbox errors"])


def test_verdict_flags_sandbox_errors():
    _, rows = table(scorecard(PANEL))
    assert "sandbox errors" not in rows[("m1", "codecite")]["verdict"]
    assert "sandbox errors (baseline 0, codecite 2)" in rows[("m2", "codecite")]["verdict"]


def test_old_logs_read_timed_out_as_timeouts():
    old = sample("baseline", 1)
    old.scores["hidden_tests_and_checks"].metadata = {"timed_out": True}
    _, rows = table(scorecard([log("openrouter/old", [old])]))
    assert (rows[("old", "baseline")]["timeouts"], rows[("old", "baseline")]["sandbox errors"]) == ("1", "0")


def test_tier_heads_the_scorecard():
    assert scorecard(PANEL, "quick").startswith("**Quick tier** (directional, never the gate)")
    assert scorecard(PANEL).startswith("**Task: `fastapi_middleware`**\n\n| model |")


def test_a_directory_gives_each_model_its_newest_finished_log(tmp_path):
    for name, status, created in (("old", "success", "2026-10-01T00:00:00+00:00"),
                                  ("new", "success", "2026-10-02T00:00:00+00:00"),
                                  ("newer-unfinished", "started", "2026-10-03T00:00:00+00:00")):
        write_eval_log(log("openrouter/m", [sample("baseline", 1)], status, created), str(tmp_path / f"{name}.eval"))
    (found,) = logs([tmp_path])
    assert found.eval.created == "2026-10-02T00:00:00+00:00"


@pytest.mark.parametrize(
    "calls, text, expected",
    [
        ([NATIVE], "", 1),
        ([ToolCall(id="r", function="mcp__codecite__read", arguments={"citation": "c"})], "", 1),
        ([bash('codecite search eval "token validation"')], "", 1),
        ([bash("cd /work/project && /opt/codecite/bin/codecite read eval 'a.py:1-2'")], "", 1),
        ([bash("RUST_LOG=warn codecite search eval x | head -40")], "", 1),
        ([bash("echo $(codecite search eval x)")], "", 1),
        # Naming codecite without running it is not a call.
        ([], "I could run `codecite search eval middleware` here.", 0),
        ([bash("grep -rn codecite /work/project"), bash("ls /opt/codecite/bin"), bash("echo codecite")], "", 0),
        ([ToolCall(id="g", function="grep", arguments={"pattern": "codecite"})], "", 0),
        ([NATIVE, bash("codecite search eval x")], "", 2),
    ],
)
def test_codecite_calls(calls, text, expected):
    s = sample("codecite", 1, calls=[])
    s.events = [model_call(calls, text)]
    assert codecite_calls(s) == expected


def adoption(used):
    """A clean pair whose codecite arm calls codecite in `used` of its runs:
    one by the CLI, the rest natively; the others only mention it."""
    calls = [[bash("codecite search eval x")]] + [[NATIVE]] * (used - 1) + [[]] * (EPOCHS - used)
    code = [sample("codecite", e + 1, passed=True, calls=c) for e, c in enumerate(calls)]
    for s, c in zip(code, calls):
        if not c:
            s.events = [model_call(text="`codecite search` might help, but grep will do.")]
    return log(f"openrouter/used-{used}", [sample("baseline", e, passed=True) for e in range(1, EPOCHS + 1)] + code)


def test_used_codecite_is_shown_per_arm():
    _, rows = table(scorecard([adoption(3)]))
    assert rows[("used-3", "codecite")]["used codecite"] == "3/4"
    assert rows[("used-3", "baseline")]["used codecite"] == "0/4"


def test_half_adoption_is_measured():
    _, rows = table(scorecard([adoption(EPOCHS // 2)]))
    assert rows[("used-2", "codecite")]["verdict"].startswith("no loss")


def test_below_half_adoption_is_not_measured():
    _, rows = table(scorecard([adoption(EPOCHS // 2 - 1)]))
    assert rows[("used-1", "codecite")]["verdict"] == "not measured (codecite used in 1 of 4 runs)"


def test_adoption_counts_usable_runs():
    # An errored sample isn't usable: one use in two usable runs is half.
    l = adoption(1)
    for s in l.samples:
        if s.metadata["arm"] == "codecite" and s.epoch > 2:
            s.error = EvalError(message="provider error", traceback="", traceback_ansi="")
    l.eval.config.epochs = 2
    _, rows = table(scorecard([l]))
    assert rows[("used-1", "codecite")]["used codecite"] == "1/2"
    assert rows[("used-1", "codecite")]["verdict"].startswith("no loss")


def test_calls_are_read_back_from_a_log_file(tmp_path):
    write_eval_log(adoption(3), str(tmp_path / "used.eval"))
    (found,) = logs([tmp_path])
    _, rows = table(scorecard([found]))
    assert rows[("used-3", "codecite")]["used codecite"] == "3/4"


def test_a_command_stored_as_an_attachment_is_read(tmp_path):
    # Inspect moves long strings out of the events into sample.attachments.
    s = sample("codecite", 1, calls=[bash("attachment://abc"), bash("attachment://def")])
    s.attachments = {"abc": "cd /work/project && codecite search eval x", "def": "grep -rn codecite ."}
    assert codecite_calls(s) == 1


def calibration(model, passes, turns=3, runs=EPOCHS):
    return log(model, [sample("baseline", e, passed=e <= passes, turns=turns) for e in range(1, runs + 1)], arms="baseline")


def calibration_table(out):
    lines = [line for line in out.splitlines() if line.startswith("|")]
    header = [c.strip() for c in lines[0].strip("|").split("|")]
    return header, {r["model"]: r for r in (dict(zip(header, (c.strip() for c in line.strip("|").split("|")))) for line in lines[2:])}


def test_a_baseline_only_run_is_a_calibration_with_no_verdict():
    out = scorecard([calibration("openrouter/m1", 2, turns=5), calibration("openrouter/m2", 3)], "quick")
    assert out.startswith("**Calibration** (baseline arm only, quick panel): no verdict, and never a pass.")
    header, rows = calibration_table(out)
    assert header == ["model", "runs", "pass", "fails", "timeouts", "sandbox errors", "median turns", "median tokens", "median time"]
    assert (rows["m1"]["pass"], rows["m1"]["median turns"], rows["m2"]["pass"]) == ("50%", "5", "75%")
    assert "no loss" not in out and "loses on" not in out


def test_calibration_in_band():
    out = scorecard([calibration("openrouter/m1", 2), calibration("openrouter/m2", 3)], "quick")
    assert "Pooled baseline pass rate: 62% of 8 runs. Band: in band." in out


@pytest.mark.parametrize(
    "passes, why",
    [
        ((1, 1), "too hard: pooled pass rate under 40%"),
        ((4, 3), "too easy: pooled pass rate over 80%; passes every run: m1"),
        ((4, 0), "passes every run: m1"),
    ],
)
def test_calibration_out_of_band(passes, why):
    out = scorecard([calibration("openrouter/m1", passes[0]), calibration("openrouter/m2", passes[1])], "quick")
    assert f"Band: out of band ({why})." in out


def test_a_short_calibration_is_incomplete():
    out = scorecard([calibration("openrouter/m1", 2), calibration("openrouter/m2", 2, runs=3)], "quick")
    assert "Band: incomplete (m2 3/4 runs)." in out


def test_a_calibration_never_reads_as_a_pass():
    for passes in range(EPOCHS + 1):
        out = scorecard([calibration("openrouter/m1", passes), calibration("openrouter/m2", 2)], "quick")
        assert "no loss" not in out
        assert not out.startswith(("**Quick tier**", "**Full tier**", "| model | arm |"))


def test_an_ab_run_that_lost_its_codecite_samples_is_incomplete_not_a_calibration():
    (only_baseline,) = [log("openrouter/m", [sample("baseline", e, passed=True) for e in range(1, EPOCHS + 1)])]
    _, rows = table(scorecard([only_baseline]))
    assert rows[("m", "codecite")]["verdict"] == f"incomplete (4/0 of {EPOCHS} runs)"


def test_calibration_and_ab_logs_are_not_mixed():
    with pytest.raises(ValueError, match="mix calibration"):
        scorecard([calibration("openrouter/m1", 2), PANEL[0]])


QUESTIONS = ["q1", "q2", "q3"]
SPEC_IDS = [f"{arm}:{q}" for q in QUESTIONS for arm in ("baseline", "codecite")]


def spec_log(model, samples, epochs=2):
    return log(model, samples, task="codecite_evals/spec_questions", sample_ids=SPEC_IDS, epochs=epochs)


def spec_runs(arm, epochs=2, passed=True, skip=0):
    runs = [sample(arm, e, passed=passed, question=q) for q in QUESTIONS for e in range(1, epochs + 1)]
    return runs[skip:]


def tables(out):
    """Each task's rows, by the task its table is headed with."""
    found, current = {}, None
    for line in out.splitlines():
        if line.startswith("**Task: `"):
            current = line.removeprefix("**Task: `").removesuffix("`**")
            found[current] = []
        elif line.startswith("|") and current:
            found[current].append(line)
    return {name: table("\n".join(lines))[1] for name, lines in found.items()}


def test_each_task_gets_its_own_table():
    out = scorecard(PANEL[:1] + [spec_log("openrouter/m1", spec_runs("baseline") + spec_runs("codecite", passed=False))])
    by_task = tables(out)
    assert set(by_task) == {"fastapi_middleware", "spec_questions"}
    # Neither task's runs pool into the other's.
    assert by_task["fastapi_middleware"][("m1", "baseline")]["runs"] == "4"
    assert by_task["spec_questions"][("m1", "baseline")]["runs"] == "6"
    assert by_task["spec_questions"][("m1", "codecite")]["pass"] == "0%"
    assert "loses on pass" in by_task["spec_questions"][("m1", "codecite")]["verdict"]


def test_planned_runs_are_questions_times_epochs():
    # 3 questions x 2 epochs = 6 runs per arm, not 2.
    short = tables(scorecard([spec_log("openrouter/m", spec_runs("baseline", skip=1) + spec_runs("codecite"))]))
    assert short["spec_questions"][("m", "codecite")]["verdict"] == "incomplete (5/6 of 6 runs)"
    full = tables(scorecard([spec_log("openrouter/m", spec_runs("baseline") + spec_runs("codecite"))]))
    assert full["spec_questions"][("m", "codecite")]["verdict"].startswith("no loss")


def test_a_question_sandbox_error_names_the_question():
    runs = spec_runs("baseline", epochs=1) + spec_runs("codecite", epochs=1)[1:]
    runs.append(sample("codecite", 1, stopped="sandbox_error", error="RuntimeError: container killed", question="q1"))
    out = scorecard([spec_log("openrouter/m", runs, epochs=1)])
    assert "- m, codecite, q1, epoch 1: `RuntimeError: container killed`" in out


def test_a_directory_gives_each_task_and_model_its_own_log(tmp_path):
    write_eval_log(log("openrouter/m", [sample("baseline", 1)]), str(tmp_path / "code.eval"))
    write_eval_log(spec_log("openrouter/m", spec_runs("baseline")), str(tmp_path / "spec.eval"))
    assert sorted(found.eval.task for found in logs([tmp_path])) == ["codecite_evals/spec_questions", "fastapi_middleware"]
