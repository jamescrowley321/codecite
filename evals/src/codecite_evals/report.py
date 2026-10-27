"""Per model, one arm against another: pass rate, median tokens, median time,
and the no-loss verdict from docs/design.md (Evaluation plan).

A log holds two arms (config.ARMS): baseline and codecite, or, from an A/A
round (`-T arms=aa`), two baseline arms labelled apart. The first is the
reference. Each measure is a difference between the arms: pass rate in points
(second minus first), median tokens and median time relative to the first
arm's median (second / first - 1). Each gets a 95% percentile bootstrap interval, each
arm resampled on its own. codecite loses when an interval is past the margin
at its favourable end: pass rate's upper bound more than 5 points below 0, or
tokens' or time's lower bound more than 5% above 0. A model short of the
planned runs in either arm is incomplete, not a pass. An A/A round gets no
verdict: it reports which intervals noise alone pushes past the margin.

From each sample's transcript (its model events) it also reads the turns
(model calls), the output tokens, the model time (the sum of the calls'
times), the rest of the wall time, the decode rate (output tokens per second
of model time) and the upstream providers OpenRouter says served the calls.
The scorecard shows their medians per arm, the providers with counts, and a
95% interval for every measure.

Timeouts and sandbox errors both count as failures. They get columns of
their own, each sandbox error is listed under the table, and a model with
any has them named in its verdict, so a loss or pass they helped make doesn't
read as clean. An agent whose own output overflowed or didn't decode is a
plain failure, in neither column.

    uv run codecite-eval-report [--tier quick|full] [LOG_DIR_OR_FILES...]

The default logs are the tier's, inspect/logs/<tier>/, else inspect/logs/
(both under evals/, see config.py).
Given a directory, it reads one log per model: the newest finished one, else
the newest. The tier heads the scorecard: a quick verdict is directional, and
a quick loss means run the full tier.
"""

import argparse
import random
import sys
from collections import Counter, defaultdict
from dataclasses import dataclass, field
from pathlib import Path
from statistics import median

from inspect_ai.log import list_eval_logs, read_eval_log

from codecite_evals import config

MARGIN = 0.05
RESAMPLES = 10_000
SEED = 0
AA = "**A/A round**: both arms are baseline, with the same prompt and sandbox, so every difference is noise."
TIERS = {
    "quick": "**Quick tier** (directional, never the gate): a loss here means run the full tier.",
    "full": "**Full tier**: the no-loss gate in docs/design.md (Evaluation plan).",
}


def logs(paths: list[Path], tier=None):
    """The logs at these files and directories; from a directory, one per
    model. With no paths, the tier's log directory."""
    if not paths:
        paths = [config.LOGS / tier if tier else config.LOGS]
    for path in paths:
        if not path.exists():
            sys.exit(f"no logs at {path}")
        if not path.is_dir():
            yield read_eval_log(str(path))
            continue
        newest = {}  # model -> header
        for info in list_eval_logs(str(path)):
            head = read_eval_log(info.name, header_only=True)
            key = (head.status == "success", head.eval.created)
            if head.eval.model not in newest or key > newest[head.eval.model][0]:
                newest[head.eval.model] = (key, info.name)
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

    @property
    def other_seconds(self):
        return None if self.model_seconds is None else self.seconds - self.model_seconds

    @property
    def decode_rate(self):
        """Output tokens per second of model time."""
        return self.output_tokens / self.model_seconds if self.model_seconds else None


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
        ", ".join(f"{name} {n}" for name, n in providers.most_common()) or "-",
    ])


def scorecard(logs, tier=None):
    """The scorecard for these eval logs, as markdown, headed by its tier."""
    runs = defaultdict(list)  # (model, arm) -> [Run]
    errors = []  # (model, arm, epoch, sandbox error)
    planned = {}
    arms = {}  # model -> ((label, condition), (label, condition))
    for log in logs:
        planned[log.eval.model] = log.eval.config.epochs or 1
        arms[log.eval.model] = config.ARMS.get((log.eval.task_args or {}).get("arms", "ab"), config.ARMS["ab"])
        for s in log.samples or []:
            arm = s.metadata["arm"]
            if s.error or not s.scores:
                continue
            score = next(iter(s.scores.values()))
            usage = (s.model_usage or {}).values()
            metadata = score.metadata or {}
            why = stopped(metadata)
            turns, model_seconds, providers = transcript(s)
            runs[(log.eval.model, arm)].append(Run(
                passed=score.value == 1,
                tokens=sum(u.total_tokens or 0 for u in usage),
                seconds=s.total_time or 0.0,
                stopped=why,
                turns=turns,
                output_tokens=sum(u.output_tokens or 0 for u in usage),
                model_seconds=model_seconds,
                providers=providers,
            ))
            if why == "sandbox_error":
                errors.append((log.eval.model, arm, s.epoch, metadata.get("sandbox_error") or "unknown error"))

    def aa(model):
        (_, a), (_, b) = arms[model]
        return a == b

    rng = random.Random(SEED)
    out = [TIERS[tier]] if tier else []
    if any(aa(model) for model in planned):
        out.append(AA)
    out += [""] if out else []
    out += [
        "| model | arm | runs | pass | fails | timeouts | sandbox errors | median tokens | median time | verdict |",
        "|---|---|---|---|---|---|---|---|---|---|",
    ]
    detail = [
        "",
        "Medians from the transcripts, and the upstream providers that served the model calls:",
        "",
        "| model | arm | turns | output tokens | model time | other time | decode rate | providers |",
        "|---|---|---|---|---|---|---|---|",
    ]
    spans = [
        "",
        f"95% intervals, second arm against the first (* past the {MARGIN:.0%} margin on either side of 0):",
        "",
        "| model | arms | " + " | ".join(name for name, _, _ in MEASURES) + " |",
        "|---|---|" + "---|" * len(MEASURES),
    ]
    for model in sorted(planned):
        name = model.removeprefix("openrouter/")
        pair = [label for label, _ in arms[model]]
        first, second = (runs.get((model, arm), []) for arm in pair)
        if len(first) < planned[model] or len(second) < planned[model]:
            verdict = f"incomplete ({len(first)}/{len(second)} of {planned[model]} runs)"
            spans.append(f"| {name} | {pair[1]} vs {pair[0]} | " + " | ".join("-" for _ in MEASURES) + " |")
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
            spans.append(f"| {name} | {pair[1]} vs {pair[0]} | " + " | ".join(cells) + " |")
            p, t, s = found["pass"], found["tokens"], found["time"]
            ranges = f" (pass {p[0]:+.0%}..{p[1]:+.0%}, tokens {t[0]:+.0%}..{t[1]:+.0%}, time {s[0]:+.0%}..{s[1]:+.0%})"
            if aa(model):
                # Noise alone made these differences: no verdict, only which
                # intervals it pushed past the margin.
                wide = [m for m, _, _ in MEASURES if found[m] is not None and not fits(*found[m])]
                verdict = ("noise past the margin on " + ", ".join(wide) if wide else "noise inside the margin") + ranges
            else:
                losses = [n for n, lost in (("pass", p[1] < -MARGIN), ("tokens", t[0] > MARGIN), ("time", s[0] > MARGIN)) if lost]
                verdict = ("loses on " + ", ".join(losses) if losses else "no loss") + ranges
        crashed = [sum(x.stopped == "sandbox_error" for x in r) for r in (first, second)]
        if any(crashed):
            verdict += f", with sandbox errors ({pair[0]} {crashed[0]}, {pair[1]} {crashed[1]})"
        for i, (arm, r) in enumerate(zip(pair, (first, second))):
            # An arm with no usable runs still gets a row, so the verdict
            # (incomplete) always shows.
            cells = (
                f"{sum(x.passed for x in r) / len(r):.0%} | {sum(not x.passed for x in r)} | {sum(x.stopped == 'agent_timeout' for x in r)}"
                f" | {sum(x.stopped == 'sandbox_error' for x in r)} | {median(x.tokens for x in r):,.0f} | {median(x.seconds for x in r):.0f}s"
                if r
                else "- | - | - | - | - | -"
            )
            out.append(f"| {name} | {arm} | {len(r)} | {cells} | {verdict if i == 1 else ''} |")
            detail.append(f"| {name} | {arm} | {medians(r)} |")
    out += detail + spans
    if errors:
        out += ["", "Sandbox errors (scored as failures):", ""]
        for model, arm, epoch, error in sorted(errors, key=lambda e: e[:3]):
            out.append(f"- {model.removeprefix('openrouter/')}, {arm}, epoch {epoch}: {listed(error)}")
    return "\n".join(out)


def main():
    parser = argparse.ArgumentParser(description="The A/B scorecard.")
    parser.add_argument("--tier", choices=sorted(TIERS))
    parser.add_argument("logs", nargs="*", type=Path, help="log directories or files")
    args = parser.parse_args()
    print(scorecard(logs(args.logs, args.tier), args.tier))
