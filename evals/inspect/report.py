"""Per model, baseline vs codecite: pass rate, median tokens, median time,
and the no-loss verdict from docs/design.md (Evaluation plan).

Each measure is a difference between the arms: pass rate in points (codecite
minus baseline), median tokens and median time relative to baseline's median
(codecite / baseline - 1). Each gets a 95% percentile bootstrap interval, each
arm resampled on its own. codecite loses when an interval is past the margin
at its favourable end: pass rate's upper bound more than 5 points below 0, or
tokens' or time's lower bound more than 5% above 0. A model short of the
planned runs in either arm is incomplete, not a pass.

Timeouts and sandbox errors both count as failures. They get columns of
their own, each sandbox error is listed under the table, and a model with
any has them named in its verdict, so a loss or pass they helped make doesn't
read as clean. An agent whose own output overflowed or didn't decode is a
plain failure, in neither column.

    uv run python inspect/report.py [LOG_DIR_OR_FILES...]   (default: inspect/logs)

Given a directory, it reads one log per model: the newest finished one, else
the newest.
"""

import random
import sys
from collections import defaultdict
from pathlib import Path
from statistics import median

from inspect_ai.log import list_eval_logs, read_eval_log

MARGIN = 0.05
RESAMPLES = 10_000
SEED = 0


def logs(args):
    if not args:
        args = [str(Path(__file__).with_name("logs"))]
    for a in args:
        if not Path(a).is_dir():
            yield read_eval_log(a)
            continue
        newest = {}  # model -> header
        for info in list_eval_logs(a):
            head = read_eval_log(info.name, header_only=True)
            key = (head.status == "success", head.eval.created)
            if head.eval.model not in newest or key > newest[head.eval.model][0]:
                newest[head.eval.model] = (key, info.name)
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


def listed(error, limit=200):
    """An error on one line, short and fenced: the scorecard is posted where
    agents read it, and the text can carry whatever the sandbox printed."""
    text = " ".join(error.split()).replace("`", "'")
    return f"`{text[:limit]}…`" if len(text) > limit else f"`{text}`"


def main(args):
    runs = defaultdict(list)  # (model, arm) -> [(passed, tokens, seconds, stopped)]
    errors = []  # (model, arm, epoch, sandbox error)
    planned = {}
    for log in logs(args):
        planned[log.eval.model] = log.eval.config.epochs or 1
        for s in log.samples or []:
            if s.error or not s.scores:
                continue
            score = next(iter(s.scores.values()))
            tokens = sum(u.total_tokens or 0 for u in (s.model_usage or {}).values())
            metadata = score.metadata or {}
            why = stopped(metadata)
            runs[(log.eval.model, s.metadata["arm"])].append((score.value == 1, tokens, s.total_time or 0.0, why))
            if why == "sandbox_error":
                errors.append((log.eval.model, s.metadata["arm"], s.epoch, metadata.get("sandbox_error") or "unknown error"))

    rng = random.Random(SEED)
    print("| model | arm | runs | pass | fails | timeouts | sandbox errors | median tokens | median time | verdict |")
    print("|---|---|---|---|---|---|---|---|---|---|")
    for model in sorted(planned):
        base, code = runs.get((model, "baseline"), []), runs.get((model, "codecite"), [])
        if len(base) < planned[model] or len(code) < planned[model]:
            verdict = f"incomplete ({len(base)}/{len(code)} of {planned[model]} runs)"
        else:
            p = interval(rng, [r[0] for r in base], [r[0] for r in code], pass_diff)
            t = interval(rng, [r[1] for r in base], [r[1] for r in code], median_ratio)
            s = interval(rng, [r[2] for r in base], [r[2] for r in code], median_ratio)
            losses = [name for name, lost in (("pass", p[1] < -MARGIN), ("tokens", t[0] > MARGIN), ("time", s[0] > MARGIN)) if lost]
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
                f"{sum(x[0] for x in r) / len(r):.0%} | {sum(not x[0] for x in r)} | {sum(x[3] == 'agent_timeout' for x in r)}"
                f" | {sum(x[3] == 'sandbox_error' for x in r)} | {median(x[1] for x in r):,.0f} | {median(x[2] for x in r):.0f}s"
                if r
                else "- | - | - | - | - | -"
            )
            print(f"| {model.removeprefix('openrouter/')} | {arm} | {len(r)} | {cells} | {verdict if arm == 'codecite' else ''} |")
    if errors:
        print("\nSandbox errors (scored as failures):\n")
        for model, arm, epoch, error in sorted(errors, key=lambda e: e[:3]):
            print(f"- {model.removeprefix('openrouter/')}, {arm}, epoch {epoch}: {listed(error)}")


if __name__ == "__main__":
    main(sys.argv[1:])
