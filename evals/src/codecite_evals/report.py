"""Per model, baseline vs codecite: pass rate, median tokens, median time,
and the no-loss verdict from docs/design.md (Evaluation plan).

Each measure is a difference between the arms: pass rate in points (codecite
minus baseline), median tokens and median time relative to baseline's median
(codecite / baseline - 1). Each gets a 95% percentile bootstrap interval, each
arm resampled on its own. codecite loses when an interval is past the margin
at its favourable end: pass rate's upper bound more than 5 points below 0, or
tokens' or time's lower bound more than 5% above 0. A model short of the
planned runs in either arm is incomplete, not a pass. A model's planned runs
per arm are the arm's samples times epochs: one sample for a code task, one
per question for spec-questions.

Each task gets its own table, headed by its name, so one task's runs never
pool with another's.

Timeouts and sandbox errors both count as failures. They get columns of
their own, each sandbox error is listed under the table, and a model with
any has them named in its verdict, so a loss or pass they helped make doesn't
read as clean. An agent whose own output overflowed or didn't decode is a
plain failure, in neither column.

A pair whose codecite arm called codecite in fewer than half its usable runs
is not measured: the arms then differ by a tool the agent never used, so the
comparison says nothing about codecite. "used codecite" counts, per arm, the
runs with at least one call, read from the model calls in the log: a call to
one of codecite's native tools, or a `bash` call that runs `codecite`.

A run of the baseline arm alone (`-T arms=baseline`, recorded in the log's
task arguments) is a calibration, not an A/B run: per task, it gets a
calibration table, with pass rate, median turns, tokens and time per model and
the pooled pass rate against the band, and no verdict. A calibration is never
a pass.

    uv run codecite-eval-report [--tier quick|full] [LOG_DIR_OR_FILES...]

The default logs are the tier's, inspect/logs/<tier>/, else inspect/logs/
(both under evals/, see config.py).
Given a directory, it reads one log per task and model: the newest finished
one, else the newest. The tier heads the scorecard: a quick verdict is directional, and
a quick loss means run the full tier.
"""

import argparse
import random
import re
import sys
from collections import defaultdict
from pathlib import Path
from statistics import median

from inspect_ai.event import ModelEvent
from inspect_ai.log import list_eval_logs, read_eval_log
from inspect_ai.model import ChatMessageAssistant

from codecite_evals import config

MARGIN = 0.05
# A pair is measured when its codecite arm used codecite in at least this
# share of its usable runs.
ADOPTION = 0.5
NATIVE = "mcp__codecite__"
ATTACHMENT = "attachment://"
# `codecite` at the start of a shell command: of the line, or after `;`, `&`,
# `|`, `(`, `$(` or a backtick, behind any VAR=value prefixes, by bare name or
# path. `grep codecite` or `ls /opt/codecite` don't run it.
CLI = re.compile(r"(?:^|[;&|(`\n]|\$\()\s*(?:\w+=\S*\s+)*(?:\S*/)?codecite(?=\s|$)")
RESAMPLES = 10_000
SEED = 0
# A task is in band when baseline passes this share of runs, pooled over the
# quick panel's models, and no model passes every run (docs/design.md).
BAND = (0.40, 0.80)
CALIBRATION = (
    "**Calibration** (baseline arm only, {tier} panel): no verdict, and never a pass. "
    "The task is in band when baseline passes 40–80% of runs pooled over models and no model passes every run."
)
TIERS = {
    "quick": "**Quick tier** (directional, never the gate): a loss here means run the full tier.",
    "full": "**Full tier**: the no-loss gate in docs/design.md (Evaluation plan).",
}


def logs(paths: list[Path], tier=None):
    """The logs at these files and directories; from a directory, one per
    task and model. With no paths, the tier's log directory."""
    if not paths:
        paths = [config.LOGS / tier if tier else config.LOGS]
    for path in paths:
        if not path.exists():
            sys.exit(f"no logs at {path}")
        if not path.is_dir():
            yield read_eval_log(str(path))
            continue
        newest = {}  # (task, model) -> header
        for info in list_eval_logs(str(path)):
            head = read_eval_log(info.name, header_only=True)
            key = (head.status == "success", head.eval.created)
            run = (head.eval.task, head.eval.model)
            if run not in newest or key > newest[run][0]:
                newest[run] = (key, info.name)
        for _, name in newest.values():
            yield read_eval_log(name)


def interval(rng, base, code, stat):
    """95% percentile bootstrap interval of stat(codecite, baseline)."""
    draws = sorted(
        stat(rng.choices(code, k=len(code)), rng.choices(base, k=len(base))) for _ in range(RESAMPLES)
    )
    return draws[int(0.025 * RESAMPLES)], draws[int(0.975 * RESAMPLES) - 1]


def pass_diff(code, base):
    return sum(code) / len(code) - sum(base) / len(base)


def median_ratio(code, base):
    m = median(base)
    if m == 0:
        return 0.0 if median(code) == 0 else float("inf")
    return median(code) / m - 1


def stopped(metadata):
    """Why the agent stopped: agent_timeout, agent_output, sandbox_error or None. Logs from
    before the two were told apart only carry timed_out, which meant either."""
    return metadata.get("stopped") or ("agent_timeout" if metadata.get("timed_out") else None)


def codecite_calls(sample):
    """How many tool calls in this sample's model calls ran codecite: its
    native tools, or `bash` with a codecite command. Text that only names
    codecite isn't a call."""
    calls = 0
    for event in sample.events or []:
        if not isinstance(event, ModelEvent) or not event.output.choices:
            continue
        for call in event.output.message.tool_calls or []:
            if call.function.startswith(NATIVE):
                calls += 1
            elif call.function == "bash":
                command = call.arguments.get("command") if isinstance(call.arguments, dict) else None
                if isinstance(command, str) and command.startswith(ATTACHMENT):
                    # Inspect stores long strings apart, in the sample's attachments.
                    command = (sample.attachments or {}).get(command.removeprefix(ATTACHMENT))
                calls += isinstance(command, str) and bool(CLI.search(command))
    return calls


def listed(error, limit=200):
    """An error on one line, short and fenced: the scorecard is posted where
    agents read it, and the text can carry whatever the sandbox printed."""
    text = " ".join(error.split()).replace("`", "'")
    return f"`{text[:limit]}…`" if len(text) > limit else f"`{text}`"


def task_name(log):
    return log.eval.task.removeprefix("codecite_evals/")


def per_arm(log, arm):
    """How many samples of the dataset are this arm's: sample ids are the arm
    (a code task) or `<arm>:<question>` (spec-questions)."""
    ids = log.eval.dataset.sample_ids
    if ids:
        return sum(str(i) == arm or str(i).startswith(f"{arm}:") for i in ids) or 1
    return max(1, (log.eval.dataset.samples or 2) // 2)


def calibrating(log):
    """Whether this log ran the baseline arm alone."""
    return (log.eval.task_args or {}).get("arms") == "baseline"


def scorecard(logs, tier=None):
    """The scorecard for these eval logs, as markdown, headed by its tier,
    with a table per task: the A/B table, or the calibration table when every
    log ran baseline alone."""
    logs = list(logs)
    kinds = {calibrating(log) for log in logs}
    if len(kinds) > 1:
        raise ValueError("these logs mix calibration runs (arms=baseline) with A/B runs; report them apart")
    runs = defaultdict(list)  # (task, model, arm) -> [(passed, tokens, seconds, stopped, turns, used codecite)]
    errors = defaultdict(list)  # task -> [(model, arm, sample, epoch, sandbox error)]
    planned = {}  # (task, model, arm) -> runs
    for log in logs:
        name, model = task_name(log), log.eval.model
        for arm in ("baseline", "codecite"):
            planned[(name, model, arm)] = per_arm(log, arm) * (log.eval.config.epochs or 1)
        for s in log.samples or []:
            if s.error or not s.scores:
                continue
            score = next(iter(s.scores.values()))
            tokens = sum(u.total_tokens or 0 for u in (s.model_usage or {}).values())
            metadata = score.metadata or {}
            why = stopped(metadata)
            arm = s.metadata["arm"]
            turns = sum(isinstance(m, ChatMessageAssistant) for m in s.messages or [])
            runs[(name, model, arm)].append((score.value == 1, tokens, s.total_time or 0.0, why, turns, codecite_calls(s) > 0))
            if why == "sandbox_error":
                # A question's sample is named by its question; a code task's by its arm alone.
                which = s.metadata.get("question")
                errors[name].append((model, arm, which, s.epoch, metadata.get("sandbox_error") or "unknown error"))

    rng = random.Random(SEED)
    if kinds == {True}:
        out = [CALIBRATION.format(tier=tier or "this")]
    else:
        out = [TIERS[tier]] if tier else []
    for name in sorted({k[0] for k in planned}):
        if out:
            out.append("")
        out += calibration(name, runs, planned) if kinds == {True} else table(rng, name, runs, planned)
        out += listed_errors(errors[name])
    return "\n".join(out)


def counts(r):
    """The cells every table shares: pass, fails, timeouts, sandbox errors."""
    return (
        f"{sum(x[0] for x in r) / len(r):.0%} | {sum(not x[0] for x in r)} | {sum(x[3] == 'agent_timeout' for x in r)}"
        f" | {sum(x[3] == 'sandbox_error' for x in r)}"
    )


def calibration(name, runs, planned):
    """One task's baseline alone, per model, and where the pooled pass rate
    falls against the band. No verdict: a calibration says nothing of codecite."""
    out = [
        f"**Task: `{name}`**",
        "",
        "| model | runs | pass | fails | timeouts | sandbox errors | median turns | median tokens | median time |",
        "|---|---|---|---|---|---|---|---|---|",
    ]
    short, perfect, pooled = [], [], []
    for model in sorted({k[1] for k in planned if k[0] == name}):
        r = runs.get((name, model, "baseline"), [])
        want = planned[(name, model, "baseline")]
        label = model.removeprefix("openrouter/")
        if len(r) < want:
            short.append(f"{label} {len(r)}/{want}")
        if r and all(x[0] for x in r):
            perfect.append(label)
        pooled += [x[0] for x in r]
        cells = (
            f"{counts(r)} | {median(x[4] for x in r):.0f} | {median(x[1] for x in r):,.0f} | {median(x[2] for x in r):.0f}s"
            if r
            else "- | - | - | - | - | - | -"
        )
        out.append(f"| {label} | {len(r)} | {cells} |")
    low, high = BAND
    rate = sum(pooled) / len(pooled) if pooled else 0.0
    if short:
        band = f"incomplete ({', '.join(short)} runs)"
    else:
        misses = []
        if rate < low:
            misses.append(f"too hard: pooled pass rate under {low:.0%}")
        if rate > high:
            misses.append(f"too easy: pooled pass rate over {high:.0%}")
        if perfect:
            misses.append(f"passes every run: {', '.join(perfect)}")
        band = "in band" if not misses else "out of band (" + "; ".join(misses) + ")"
    out += ["", f"Pooled baseline pass rate: {rate:.0%} of {len(pooled)} runs. Band: {band}."]
    return out


def table(rng, name, runs, planned):
    """One task's table: per model, both arms and the verdict."""
    out = [
        f"**Task: `{name}`**",
        "",
        "| model | arm | runs | used codecite | pass | fails | timeouts | sandbox errors | median tokens | median time | verdict |",
        "|---|---|---|---|---|---|---|---|---|---|---|",
    ]
    for model in sorted({k[1] for k in planned if k[0] == name}):
        base, code = runs.get((name, model, "baseline"), []), runs.get((name, model, "codecite"), [])
        want_base, want_code = planned[(name, model, "baseline")], planned[(name, model, "codecite")]
        if len(base) < want_base or len(code) < want_code:
            of = want_base if want_base == want_code else f"{want_base}/{want_code}"
            verdict = f"incomplete ({len(base)}/{len(code)} of {of} runs)"
        elif (used := sum(x[5] for x in code)) < ADOPTION * len(code):
            verdict = f"not measured (codecite used in {used} of {len(code)} runs)"
        else:
            p = interval(rng, [r[0] for r in base], [r[0] for r in code], pass_diff)
            t = interval(rng, [r[1] for r in base], [r[1] for r in code], median_ratio)
            s = interval(rng, [r[2] for r in base], [r[2] for r in code], median_ratio)
            losses = [measure for measure, lost in (("pass", p[1] < -MARGIN), ("tokens", t[0] > MARGIN), ("time", s[0] > MARGIN)) if lost]
            verdict = ("loses on " + ", ".join(losses) if losses else "no loss") + (
                f" (pass {p[0]:+.0%}..{p[1]:+.0%}, tokens {t[0]:+.0%}..{t[1]:+.0%}, time {s[0]:+.0%}..{s[1]:+.0%})"
            )
        crashed = [sum(x[3] == "sandbox_error" for x in r) for r in (base, code)]
        if any(crashed):
            verdict += f", with sandbox errors (baseline {crashed[0]}, codecite {crashed[1]})"
        for arm, r in (("baseline", base), ("codecite", code)):
            # An arm with no usable runs still gets a row, so the verdict
            # (incomplete) always shows.
            cells = (
                f"{counts(r)} | {median(x[1] for x in r):,.0f} | {median(x[2] for x in r):.0f}s"
                if r
                else "- | - | - | - | - | -"
            )
            out.append(f"| {model.removeprefix('openrouter/')} | {arm} | {len(r)} | {sum(x[5] for x in r)}/{len(r)} | {cells} | {verdict if arm == 'codecite' else ''} |")
    return out


def listed_errors(errors):
    """A task's sandbox errors, listed under its table."""
    out = []
    if errors:
        out += ["", "Sandbox errors (scored as failures):", ""]
        for model, arm, which, epoch, error in sorted(errors, key=lambda e: (e[0], e[1], e[2] or "", e[3])):
            question = f"{which}, " if which else ""
            out.append(f"- {model.removeprefix('openrouter/')}, {arm}, {question}epoch {epoch}: {listed(error)}")
    return out


def main():
    parser = argparse.ArgumentParser(description="The A/B scorecard.")
    parser.add_argument("--tier", choices=sorted(TIERS))
    parser.add_argument("logs", nargs="*", type=Path, help="log directories or files")
    args = parser.parse_args()
    try:
        print(scorecard(logs(args.logs, args.tier), args.tier))
    except ValueError as e:
        sys.exit(str(e))
