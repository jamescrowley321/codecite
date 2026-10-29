"""Per task and model, one arm against another: pass rate, median tokens,
median time, and the no-loss verdict from docs/design.md (Evaluation plan).

A log holds the arms its `arms` task argument names (config.ARMS): baseline
and codecite (`ab`, the default), two baseline arms labelled apart from an A/A
round (`aa`), or the baseline arm alone, a calibration (`baseline`). In a
pair, the first arm is the reference. Each measure is a difference between
the arms: pass rate in points (second minus first), median tokens and median
time relative to the first arm's median (second / first - 1). Each gets a 95%
percentile bootstrap interval, each arm resampled on its own. codecite loses
when an interval is past the margin at its favourable end: pass rate's upper
bound more than 5 points below 0, or tokens' or time's lower bound more than
5% above 0. A model short of the planned runs in either arm is incomplete,
not a pass. A model's planned runs per arm are the arm's samples times
epochs: one sample for a code task, one per question for spec-questions. An
A/A round gets no verdict: it reports which intervals noise alone pushes past
the margin.

Each task gets its own tables, headed by its name, so one task's runs never
pool with another's.

From each sample's transcript (its model events) it also reads the turns
(model calls), the output tokens, the model time (the sum of the calls'
times), the rest of the wall time, the decode rate (output tokens per second
of model time) and the upstream providers OpenRouter says served the calls,
and a normalised time: wall time with the model time recomputed at the
model's decode rate over both arms, so a slower host for one arm drops out.
The scorecard shows their medians per arm, the providers with counts, and a
95% interval for every measure.

A Claude Code run (a claude-code/ model) makes its calls outside Inspect, so
its turns, tokens, model time and codecite calls come from the sample's
store instead (claude_code.py), and it names no provider.

Timeouts and sandbox errors both count as failures. They get columns of
their own, each sandbox error is listed under the task's tables, and a model
with any has them named in its verdict, so a loss or pass they helped make
doesn't read as clean. An agent whose own output overflowed or didn't decode
is a plain failure, in neither column.

A pair whose codecite arm called codecite in fewer than half its usable runs
is not measured: the arms then differ by a tool the agent never used, so the
comparison says nothing about codecite. "used codecite" counts, per arm, the
runs with at least one call, read from the model calls in the log: a call to
one of codecite's native tools, or a `bash` call that runs `codecite`.

A calibration (`-T arms=baseline`) is not an A/B run: per task, it gets a
calibration table, with pass rate, median turns, tokens and time per model and
the pooled pass rate against the band, and no verdict. A calibration is never
a pass.

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
from collections import Counter, defaultdict
from dataclasses import dataclass, field
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
AA = "**A/A round**: both arms are baseline, with the same prompt and sandbox, so every difference is noise."
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
    """95% percentile bootstrap interval of stat(second arm, first arm)."""
    draws = sorted(
        stat(rng.choices(code, k=len(code)), rng.choices(base, k=len(base))) for _ in range(RESAMPLES)
    )
    return draws[int(0.025 * RESAMPLES)], draws[int(0.975 * RESAMPLES) - 1]


@dataclass
class Run:
    """One scored sample."""

    passed: bool
    tokens: int
    seconds: float
    stopped: str | None = None
    output_tokens: int = 0
    # From the transcript: None for a sample whose log kept no model events.
    turns: int | None = None
    model_seconds: float | None = None
    providers: Counter = field(default_factory=Counter)
    # The model's decode rate over both arms' runs, set once they are read.
    pooled_rate: float | None = None
    # Whether any tool call ran codecite (codecite_calls).
    used: bool = False
    # Assistant messages: the turns of a sample whose log kept no model events.
    replies: int = 0

    @property
    def other_seconds(self):
        return None if self.model_seconds is None else self.seconds - self.model_seconds

    @property
    def decode_rate(self):
        """Output tokens per second of model time."""
        return self.output_tokens / self.model_seconds if self.model_seconds else None

    @property
    def normalised_seconds(self):
        """Wall time with the model time recomputed at the model's pooled
        decode rate: a decode-speed gap between the arms (a slower host)
        drops out, while more output or more tool time still counts."""
        if self.other_seconds is None or not self.pooled_rate:
            return None
        return self.other_seconds + self.output_tokens / self.pooled_rate


# Every measure with an interval: (column, how to read it from a Run, kind).
# The first three make the verdict.
MEASURES = [
    ("pass", lambda r: r.passed, "points"),
    ("tokens", lambda r: r.tokens, "ratio"),
    ("time", lambda r: r.seconds, "ratio"),
    ("turns", lambda r: r.turns, "ratio"),
    ("output tokens", lambda r: r.output_tokens, "ratio"),
    ("model time", lambda r: r.model_seconds, "ratio"),
    ("other time", lambda r: r.other_seconds, "ratio"),
    ("decode rate", lambda r: r.decode_rate, "ratio"),
    ("normalised time", lambda r: r.normalised_seconds, "ratio"),
]


def call_seconds(event):
    """How long one model call took: the generation's own time, else
    Inspect's working time, else its start to its end."""
    for seconds in (event.output.time if event.output else None, event.working_time):
        if seconds is not None:
            return seconds
    if event.completed and event.timestamp:
        return (event.completed - event.timestamp).total_seconds()
    return 0.0


def transcript(sample):
    """Turns, model seconds and providers from a sample's model events, or
    None and None for a sample with none. Calls that errored were retried, so
    only the ones that returned count."""
    turns, seconds, providers = 0, 0.0, Counter()
    for event in sample.events or []:
        if event.event != "model" or event.error or getattr(event, "pending", None):
            continue
        turns += 1
        seconds += call_seconds(event)
        # Inspect keeps a call's raw response only with --log-model-api
        # (panel.sh passes it); without, only each model's first few.
        if event.call is None:
            providers["not logged"] += 1
        else:
            providers[clean(str((event.call.response or {}).get("provider") or "unknown"))] += 1
    return (turns, seconds, providers) if turns else (None, None, providers)


def clean(text, limit=40):
    """External text made safe for a table cell."""
    return " ".join(text.replace("|", "/").replace("`", "'").split())[:limit]


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


def fits(lo, hi):
    """Whether an interval lies inside the margin on both sides of 0."""
    return -MARGIN <= lo and hi <= MARGIN


def span(lo, hi, kind):
    if kind == "points":
        return f"{lo * 100:+.0f}..{hi * 100:+.0f} pts"
    return f"{lo:+.0%}..{hi:+.0%}"


def seconds(value):
    return f"{value:.0f}s" if value >= 10 else f"{value:.1f}s"


def medians(runs):
    """The transcript cells for one arm's runs: each measure's median over
    the runs that have it."""

    def cell(value, form):
        known = [v for v in map(value, runs) if v is not None]
        return form(median(known)) if known else "-"

    providers = sum((r.providers for r in runs), Counter())
    return " | ".join([
        cell(lambda r: r.turns, lambda m: f"{m:.0f}"),
        cell(lambda r: r.output_tokens, lambda m: f"{m:,.0f}"),
        cell(lambda r: r.model_seconds, seconds),
        cell(lambda r: r.other_seconds, seconds),
        cell(lambda r: r.decode_rate, lambda m: f"{m:.0f} tok/s"),
        cell(lambda r: r.normalised_seconds, seconds),
        ", ".join(f"{name} {n}" for name, n in providers.most_common()) or "-",
    ])


def task_name(log):
    return log.eval.task.removeprefix("codecite_evals/")


def per_arm(log, arm, arms=2):
    """How many samples of the dataset are this arm's: sample ids are the arm
    (a code task) or `<arm>:<question>` (spec-questions). Without ids, the
    samples split evenly over the log's arms."""
    ids = log.eval.dataset.sample_ids
    if ids:
        return sum(str(i) == arm or str(i).startswith(f"{arm}:") for i in ids) or 1
    return max(1, (log.eval.dataset.samples or arms) // arms)


def calibrating(log):
    """Whether this log ran the baseline arm alone."""
    return (log.eval.task_args or {}).get("arms") == "baseline"


def calibrating(log):
    """Whether this log ran the baseline arm alone."""
    return (log.eval.task_args or {}).get("arms") == "baseline"


def scorecard(logs, tier=None):
    """The scorecard for these eval logs, as markdown, headed by its tier,
    with tables per task: the A/B (or A/A) tables, or the calibration table
    when every log ran baseline alone."""
    logs = list(logs)
    kinds = {calibrating(log) for log in logs}
    if len(kinds) > 1:
        raise ValueError("these logs mix calibration runs (arms=baseline) with A/B runs; report them apart")
    runs = defaultdict(list)  # (task, model, arm) -> [Run]
    errors = defaultdict(list)  # task -> [(model, arm, question, epoch, sandbox error)]
    planned = {}  # (task, model, arm) -> runs
    arms = {}  # (task, model) -> ((label, condition), ...)
    for log in logs:
        name, model = task_name(log), log.eval.model
        pair = config.ARMS.get((log.eval.task_args or {}).get("arms", "ab"), config.ARMS["ab"])
        arms[(name, model)] = pair
        for label, _ in pair:
            planned[(name, model, label)] = per_arm(log, label, len(pair)) * (log.eval.config.epochs or 1)
        for s in log.samples or []:
            if s.error or not s.scores:
                continue
            arm = s.metadata["arm"]
            score = next(iter(s.scores.values()))
            usage = (s.model_usage or {}).values()
            metadata = score.metadata or {}
            why = stopped(metadata)
            turns, model_seconds, providers = transcript(s)
            # Claude Code's calls bypass Inspect: its agent keeps their
            # measures in the store (claude_code.py).
            agent = s.store or {}
            if "agent_turns" in agent:
                turns, model_seconds = agent["agent_turns"] or None, agent.get("agent_model_seconds")
            runs[(name, model, arm)].append(Run(
                passed=score.value == 1,
                tokens=agent.get("agent_tokens", sum(u.total_tokens or 0 for u in usage)),
                seconds=s.total_time or 0.0,
                stopped=why,
                turns=turns,
                output_tokens=agent.get("agent_output_tokens", sum(u.output_tokens or 0 for u in usage)),
                model_seconds=model_seconds,
                providers=providers,
                used=agent.get("codecite_calls", codecite_calls(s)) > 0,
                replies=sum(isinstance(m, ChatMessageAssistant) for m in s.messages or []),
            ))
            if why == "sandbox_error":
                # A question's sample is named by its question; a code task's by its arm alone.
                errors[name].append((model, arm, s.metadata.get("question"), s.epoch, metadata.get("sandbox_error") or "unknown error"))

    rng = random.Random(SEED)
    if kinds == {True}:
        out = [CALIBRATION.format(tier=tier or "this")]
    else:
        out = [TIERS[tier]] if tier else []
        if any(a == b for pair in arms.values() if len(pair) == 2 for (_, a), (_, b) in [pair]):
            out.append(AA)
    for name in sorted({k[0] for k in planned}):
        if out:
            out.append("")
        if kinds == {True}:
            out += calibration(name, runs, planned)
        else:
            out += table(rng, name, runs, planned, arms)
        out += listed_errors(errors[name])
    return "\n".join(out)


def counts(r):
    """The cells every table shares: pass, fails, timeouts, sandbox errors."""
    return (
        f"{sum(x.passed for x in r) / len(r):.0%} | {sum(not x.passed for x in r)} | {sum(x.stopped == 'agent_timeout' for x in r)}"
        f" | {sum(x.stopped == 'sandbox_error' for x in r)}"
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
        if r and all(x.passed for x in r):
            perfect.append(label)
        pooled += [x.passed for x in r]
        cells = (
            f"{counts(r)} | {median(x.turns if x.turns is not None else x.replies for x in r):.0f}"
            f" | {median(x.tokens for x in r):,.0f} | {median(x.seconds for x in r):.0f}s"
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


def table(rng, name, runs, planned, arms):
    """One task's tables: per model, both arms and the verdict; the
    transcript medians; and every measure's interval."""
    out = [
        f"**Task: `{name}`**",
        "",
        "| model | arm | runs | used codecite | pass | fails | timeouts | sandbox errors | median tokens | median time | verdict |",
        "|---|---|---|---|---|---|---|---|---|---|---|",
    ]
    detail = [
        "",
        "Medians from the transcripts, and the upstream providers that served the model calls:",
        "",
        "| model | arm | turns | output tokens | model time | other time | decode rate | normalised time | providers |",
        "|---|---|---|---|---|---|---|---|---|",
    ]
    spans = [
        "",
        f"95% intervals, second arm against the first (* past the {MARGIN:.0%} margin on either side of 0):",
        "",
        "| model | arms | " + " | ".join(measure for measure, _, _ in MEASURES) + " |",
        "|---|---|" + "---|" * len(MEASURES),
    ]
    for model in sorted({k[1] for k in planned if k[0] == name}):
        label = model.removeprefix("openrouter/")
        pair = arms[(name, model)]
        (a_label, _), (b_label, b_condition) = pair
        aa = pair[0][1] == b_condition
        first, second = runs.get((name, model, a_label), []), runs.get((name, model, b_label), [])
        want_a, want_b = planned[(name, model, a_label)], planned[(name, model, b_label)]
        timed = [r for r in first + second if r.model_seconds]
        if timed:
            pooled = sum(r.output_tokens for r in timed) / sum(r.model_seconds for r in timed)
            for r in first + second:
                r.pooled_rate = pooled
        if len(first) < want_a or len(second) < want_b:
            of = want_a if want_a == want_b else f"{want_a}/{want_b}"
            verdict = f"incomplete ({len(first)}/{len(second)} of {of} runs)"
            spans.append(f"| {label} | {b_label} vs {a_label} | " + " | ".join("-" for _ in MEASURES) + " |")
        else:
            found = {}
            for measure, value, kind in MEASURES:
                a = [v for v in map(value, first) if v is not None]
                b = [v for v in map(value, second) if v is not None]
                found[measure] = interval(rng, a, b, pass_diff if kind == "points" else median_ratio) if a and b else None
            cells = [
                "-" if found[m] is None else span(*found[m], kind) + ("" if fits(*found[m]) else " *")
                for m, _, kind in MEASURES
            ]
            spans.append(f"| {label} | {b_label} vs {a_label} | " + " | ".join(cells) + " |")
            p, t, s = found["pass"], found["tokens"], found["time"]
            ranges = f" (pass {p[0]:+.0%}..{p[1]:+.0%}, tokens {t[0]:+.0%}..{t[1]:+.0%}, time {s[0]:+.0%}..{s[1]:+.0%})"
            if aa:
                # Noise alone made these differences: no verdict, only which
                # intervals it pushed past the margin.
                wide = [m for m, _, _ in MEASURES if found[m] is not None and not fits(*found[m])]
                verdict = ("noise past the margin on " + ", ".join(wide) if wide else "noise inside the margin") + ranges
            elif (used := sum(x.used for x in second)) < ADOPTION * len(second):
                verdict = f"not measured (codecite used in {used} of {len(second)} runs)"
            else:
                losses = [n for n, lost in (("pass", p[1] < -MARGIN), ("tokens", t[0] > MARGIN), ("time", s[0] > MARGIN)) if lost]
                verdict = ("loses on " + ", ".join(losses) if losses else "no loss") + ranges
        crashed = [sum(x.stopped == "sandbox_error" for x in r) for r in (first, second)]
        if any(crashed):
            verdict += f", with sandbox errors ({a_label} {crashed[0]}, {b_label} {crashed[1]})"
        for i, (arm, r) in enumerate(((a_label, first), (b_label, second))):
            # An arm with no usable runs still gets a row, so the verdict
            # (incomplete) always shows.
            cells = (
                f"{counts(r)} | {median(x.tokens for x in r):,.0f} | {median(x.seconds for x in r):.0f}s"
                if r
                else "- | - | - | - | - | -"
            )
            out.append(f"| {label} | {arm} | {len(r)} | {sum(x.used for x in r)}/{len(r)} | {cells} | {verdict if i == 1 else ''} |")
            detail.append(f"| {label} | {arm} | {medians(r)} |")
    return out + detail + spans


def listed_errors(errors):
    """A task's sandbox errors, listed under its tables."""
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
