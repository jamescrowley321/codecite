# Correctness as the gate, with a wall-time ceiling (#177)

Goal: replace "no loss on correctness, tokens or time" with the general-purpose
design's gate (`docs/superpowers/specs/2026-10-29-general-purpose-design.md`):
any correctness loss blocks, wall time blocks past +25%, tokens are reported
only, on a Claude-only gating panel at 5 epochs with a 10-epoch rerun of a
losing pair.

## Steps

1. **Tests first** (`evals/tests/test_report.py`). Replace the per-gate tests
   with tests of one gate for every task, with or without a `task.json` and
   with or without an old `"gate"` key:
   - a sure drop in pass rate (-4 points over 100 runs) loses; one miss in 100,
     whose interval reaches 0, doesn't;
   - time +20% and +25% is no loss, +30% loses, less time never loses;
   - tokens at -90%, +40% and +900% never change the verdict, are shown in it,
     and are never starred, and don't join a time loss;
   - the scorecard states the gate above each task's tables;
   - an A/A round stars time only when its interval is wholly past ±25%.
2. **`report.py`.** One `GATE` (pass 0, tokens `None`, time 0.25) in place of
   `GATES` and the per-task `gate()` lookup. A `losses()` helper makes the
   verdict; a `past()` helper stars an interval of a gate measure when it lies
   wholly past the gate's line on either side, and keeps the 5% margin for the
   other measures. The tier headings say quick is the gate run and full the
   rerun before a loss blocks.
3. **`replay.py`.** Stop writing `"gate": "replay"` to a new task's
   `task.json`. Frozen tasks keep the key; `report.py` ignores it.
4. **Panel.** `evals/inspect/panel.sh` runs Claude Code on haiku, sonnet and
   opus at both tiers. The names `quick` and `full` stay, since frozen tasks
   key their epochs by them. pi's OpenRouter panel stays opt-in and is
   reported beside the gate. `evals.yml` gives the quick run the time for a
   third model.
5. **Docs.** `docs/design.md` (Evaluation plan and Success criteria),
   `evals/README.md` and `AGENTS.md` step 6 state the new rule.
6. **Checks.** `uv run --locked pytest` in `evals/`, and the Rust CI commands.
