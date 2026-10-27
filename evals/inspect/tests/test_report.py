"""The scorecard counts timeouts and sandbox errors apart, on logs built here."""

from datetime import datetime, timezone

from inspect_ai.log import EvalConfig, EvalDataset, EvalLog, EvalSample, EvalSpec
from inspect_ai.model import ModelUsage
from inspect_ai.scorer import Score

from report import scorecard

EPOCHS = 4


def sample(arm, epoch, passed=False, stopped=None, error=None):
    metadata = {"stopped": stopped} if stopped else {}
    if error:
        metadata["sandbox_error"] = error
    return EvalSample(
        id=arm,
        epoch=epoch,
        input="",
        target="",
        metadata={"arm": arm},
        scores={"hidden_tests_and_checks": Score(value=1 if passed else 0, metadata=metadata)},
        model_usage={"m": ModelUsage(total_tokens=1000)},
        total_time=60.0,
    )


def log(model, samples):
    return EvalLog(
        status="success",
        eval=EvalSpec(
            created=datetime.now(timezone.utc).isoformat(),
            task="fastapi_middleware",
            dataset=EvalDataset(),
            model=model,
            config=EvalConfig(epochs=EPOCHS),
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
