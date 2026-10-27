"""The scorecard, on logs built here: timeouts and sandbox errors counted
apart, A/A rounds, and the measures read from each sample's transcript."""

from datetime import datetime, timezone

from inspect_ai.log import EvalConfig, EvalDataset, EvalLog, EvalSample, EvalSpec, write_eval_log
from inspect_ai.event import ModelEvent
from inspect_ai.model import GenerateConfig, ModelCall, ModelOutput, ModelUsage
from inspect_ai.scorer import Score

from codecite_evals.report import logs, scorecard

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


def log(model, samples, status="success", created=None):
    return EvalLog(
        status=status,
        eval=EvalSpec(
            created=created or datetime.now(timezone.utc).isoformat(),
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


def tables(out):
    """Each markdown table in the scorecard, as (header, rows)."""
    found, lines = [], []
    for line in out.splitlines() + [""]:
        if line.startswith("|"):
            lines.append(line)
        elif lines:
            header = [c.strip() for c in lines[0].strip("|").split("|")]
            found.append((header, [dict(zip(header, (c.strip() for c in row.strip("|").split("|")))) for row in lines[2:]]))
            lines = []
    return found


def table(out, n=0, key=("model", "arm")):
    """The scorecard's nth table, its rows keyed by model and arm."""
    header, rows = tables(out)[n]
    return header, {tuple(r[k] for k in key): r for r in rows}


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
    assert scorecard(PANEL).startswith("| model |")


def test_a_directory_gives_each_model_its_newest_finished_log(tmp_path):
    for name, status, created in (("old", "success", "2026-10-01T00:00:00+00:00"),
                                  ("new", "success", "2026-10-02T00:00:00+00:00"),
                                  ("newer-unfinished", "started", "2026-10-03T00:00:00+00:00")):
        write_eval_log(log("openrouter/m", [sample("baseline", 1)], status, created), str(tmp_path / f"{name}.eval"))
    (found,) = logs([tmp_path])
    assert found.eval.created == "2026-10-02T00:00:00+00:00"


# Transcript measures: logs whose samples carry model events, as Inspect
# writes them for calls through the agent bridge.

def call(seconds, output_tokens, provider="Google"):
    return ModelEvent(
        model="openrouter/m",
        input=[],
        tools=[],
        tool_choice="auto",
        config=GenerateConfig(),
        output=ModelOutput(
            model="m",
            usage=ModelUsage(output_tokens=output_tokens, total_tokens=output_tokens),
            time=seconds,
        ),
        call=None if provider == "-" else ModelCall(request={}, response={"provider": provider} if provider else {}),
    )


def traced(arm, epoch, calls, wall=60.0, passed=True, failed_calls=0):
    """A sample whose model calls took these (seconds, output tokens, provider);
    provider None leaves it out of the response, "-" logs no response."""
    s = sample(arm, epoch, passed=passed)
    s.total_time = wall
    s.model_usage = {"m": ModelUsage(total_tokens=1000, output_tokens=sum(c[1] for c in calls))}
    s.events = [call(*c) for c in calls]
    for _ in range(failed_calls):
        failed = call(5.0, 0)
        failed.error = "rate limited"
        s.events.append(failed)
    return s


def aa_log(model, samples):
    found = log(model, samples)
    found.eval.task_args = {"arms": "aa"}
    return found


def test_aa_round_is_headed_and_labelled():
    samples = [traced(arm, e, [(10.0, 500)]) for arm in ("baseline-a", "baseline-b") for e in range(1, EPOCHS + 1)]
    out = scorecard([aa_log("openrouter/m", samples)], "quick")
    assert out.splitlines()[1].startswith("**A/A round**")
    _, rows = table(out)
    assert set(rows) == {("m", "baseline-a"), ("m", "baseline-b")}
    assert rows[("m", "baseline-b")]["verdict"].startswith("noise inside the margin")
    assert "loses" not in out


def test_aa_names_the_measures_noise_pushes_past_the_margin():
    a = [traced("baseline-a", e, [(10.0, 500)], wall=60.0 + 10 * e) for e in range(1, EPOCHS + 1)]
    b = [traced("baseline-b", e, [(10.0, 500)], wall=80.0 + 10 * e) for e in range(1, EPOCHS + 1)]
    out = scorecard([aa_log("openrouter/m", a + b)])
    _, rows = table(out)
    verdict = rows[("m", "baseline-b")]["verdict"]
    assert verdict.startswith("noise past the margin on time, other time")
    _, spans = table(out, 2, ("model", "arms"))
    row = spans[("m", "baseline-b vs baseline-a")]
    assert row["time"].endswith(" *") and row["other time"].endswith(" *")
    assert not row["model time"].endswith(" *") and not row["turns"].endswith(" *")


def test_ab_logs_stay_baseline_against_codecite():
    out = scorecard(PANEL)
    assert "A/A" not in out
    _, spans = table(out, 2, ("model", "arms"))
    assert set(spans) == {("m1", "codecite vs baseline"), ("m2", "codecite vs baseline")}


def test_transcript_medians_per_arm():
    # baseline: 3 calls of 4 s and 200 tokens in 60 s of wall time; codecite:
    # 2 calls of 10 s and 300 tokens in 50 s, plus a failed call that
    # Inspect retried, which counts neither as a turn nor as model time.
    base = [traced("baseline", e, [(4.0, 200)] * 3, wall=60.0) for e in range(1, EPOCHS + 1)]
    code = [traced("codecite", e, [(10.0, 300)] * 2, wall=50.0, failed_calls=1) for e in range(1, EPOCHS + 1)]
    header, rows = table(scorecard([log("openrouter/m", base + code)]), 1)
    assert header == ["model", "arm", "turns", "output tokens", "model time", "other time", "decode rate", "normalised time", "providers"]
    b, c = rows[("m", "baseline")], rows[("m", "codecite")]
    assert (b["turns"], b["output tokens"], b["model time"], b["other time"], b["decode rate"]) == ("3", "600", "12s", "48s", "50 tok/s")
    assert (c["turns"], c["output tokens"], c["model time"], c["other time"], c["decode rate"]) == ("2", "600", "20s", "30s", "30 tok/s")
    # Both arms decode at 4,800 tokens / 128 s = 37.5 tok/s pooled, so 600
    # output tokens count as 16 s in each.
    assert (b["normalised time"], c["normalised time"]) == ("64s", "46s")


def test_providers_are_counted_per_arm():
    base = [traced("baseline", e, [(4.0, 200, "Google"), (4.0, 200, "Google-Vertex")]) for e in range(1, EPOCHS + 1)]
    code = [traced("codecite", e, [(4.0, 200, "Google"), (4.0, 200, None)]) for e in range(1, EPOCHS + 1)]
    _, rows = table(scorecard([log("openrouter/m", base + code)]), 1)
    assert rows[("m", "baseline")]["providers"] == "Google 4, Google-Vertex 4"
    assert rows[("m", "codecite")]["providers"] == "Google 4, unknown 4"


def test_calls_without_a_logged_response_are_not_logged():
    s = traced("baseline", 1, [(4.0, 200, "Google"), (4.0, 200, "-"), (4.0, 200, "-")])
    _, rows = table(scorecard([log("openrouter/m", [s])]), 1)
    assert rows[("m", "baseline")]["providers"] == "not logged 2, Google 1"


def test_provider_names_cannot_break_the_table():
    s = traced("baseline", 1, [(4.0, 200, "evil | `name`\nnext")])
    _, rows = table(scorecard([log("openrouter/m", [s])]), 1)
    assert rows[("m", "baseline")]["providers"] == "evil / 'name' next 1"


def test_every_measure_gets_an_interval():
    base = [traced("baseline", e, [(4.0, 200)] * 3, wall=60.0 + e) for e in range(1, EPOCHS + 1)]
    code = [traced("codecite", e, [(10.0, 300)] * 2, wall=50.0 + e) for e in range(1, EPOCHS + 1)]
    header, spans = table(scorecard([log("openrouter/m", base + code)]), 2, ("model", "arms"))
    measures = ["pass", "tokens", "time", "turns", "output tokens", "model time", "other time", "decode rate", "normalised time"]
    assert header == ["model", "arms", *measures]
    row = spans[("m", "codecite vs baseline")]
    assert row["pass"] == "+0..+0 pts"
    assert row["turns"] == "-33%..-33% *"
    assert row["model time"] == "+67%..+67% *"
    assert row["decode rate"] == "-40%..-40% *"


def test_logs_without_model_events_leave_the_transcript_measures_blank():
    out = scorecard(PANEL)
    _, rows = table(out, 1)
    r = rows[("m1", "baseline")]
    assert (r["turns"], r["model time"], r["other time"], r["decode rate"], r["providers"]) == ("-", "-", "-", "-", "-")
    _, spans = table(out, 2, ("model", "arms"))
    assert spans[("m1", "codecite vs baseline")]["model time"] == "-"


def test_transcript_measures_survive_a_log_file(tmp_path):
    base = [traced("baseline", e, [(4.0, 200, "Google")] * 3) for e in range(1, EPOCHS + 1)]
    code = [traced("codecite", e, [(4.0, 200, "Anthropic")] * 3) for e in range(1, EPOCHS + 1)]
    write_eval_log(log("openrouter/m", base + code), str(tmp_path / "ab.eval"))
    (found,) = logs([tmp_path / "ab.eval"])
    _, rows = table(scorecard([found]), 1)
    assert rows[("m", "codecite")]["providers"] == "Anthropic 12"
    assert rows[("m", "baseline")]["decode rate"] == "50 tok/s"


def test_normalised_time_drops_a_decode_speed_gap():
    # The same work in both arms, but codecite's host decodes at half the
    # speed: wall time and model time lose, normalised time doesn't move.
    base = [traced("baseline", e, [(5.0, 500)] * 4, wall=40.0 + e) for e in range(1, EPOCHS + 1)]
    code = [traced("codecite", e, [(10.0, 500)] * 4, wall=60.0 + e) for e in range(1, EPOCHS + 1)]
    out = scorecard([log("openrouter/m", base + code)])
    _, spans = table(out, 2, ("model", "arms"))
    row = spans[("m", "codecite vs baseline")]
    assert row["time"].endswith(" *") and row["model time"].endswith(" *")
    assert not row["normalised time"].endswith(" *")
