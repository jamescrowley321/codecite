# Eval noise: an A/A round on the quick panel (#90)

Wall time was too noisy to call a time loss at 10 runs per arm: arms that
differed only by one prompt sentence moved by 9–33%, and deepseek's time loss
in quick-2 was a slower decode on the same provider, not extra work. This
finding measures the noise directly and names the time measure the verdict
should use. Changing the verdict is the owner's decision, made in review.

## The run

An A/A round: two baseline arms (`baseline-a`, `baseline-b`) with the same
prompt and the same sandbox, on the quick panel, 10 epochs per arm, 8
sandboxes on the CI runner's 4 vCPU. [Run
40441057255](https://github.com/jamescrowley321/codecite/actions/runs/40441057255)
(head 38accf2) finished every sample: 60 of 60, all passing, no timeouts or
sandbox errors. The tables below are its logs (artifact `eval-logs-quick-aa`)
read by this PR's final `report.py`, which adds normalised time to what the
run printed; the run's own numbers are the same apart from bootstrap draws
shifting by a point.

A second run ([40443826417](https://github.com/jamescrowley321/codecite/actions/runs/40443826417),
with `--log-model-api`) stopped when the eval key's OpenRouter balance ran out
(402s after 14 haiku samples). It shows only that every call's provider is now
logged; it carries no numbers here.

### Medians per arm

| model | arm | median tokens | median time | turns | output tokens | model time | other time | decode rate | normalised time |
|---|---|---|---|---|---|---|---|---|---|
| claude-haiku-5.5 | baseline-a | 170,104 | 62s | 8 | 3,794 | 21s | 40s | 183 tok/s | 61s |
| claude-haiku-5.5 | baseline-b | 170,084 | 60s | 8 | 3,766 | 21s | 41s | 183 tok/s | 61s |
| claude-sonnet-5.5 | baseline-a | 113,492 | 54s | 7 | 1,999 | 21s | 33s | 95 tok/s | 55s |
| claude-sonnet-5.5 | baseline-b | 121,284 | 60s | 8 | 2,324 | 22s | 37s | 101 tok/s | 60s |
| gpt-6.1-sol | baseline-a | 72,514 | 88s | 13 | 1,031 | 45s | 42s | 23 tok/s | 88s |
| gpt-6.1-sol | baseline-b | 70,906 | 91s | 13 | 1,017 | 45s | 45s | 22 tok/s | 93s |

### 95% intervals, baseline-b against baseline-a

`*` marks an interval reaching past 5% (5 points for pass) on either side of 0.

| model | pass | tokens | time | turns | output tokens | model time | other time | decode rate | normalised time |
|---|---|---|---|---|---|---|---|---|---|
| claude-haiku-5.5 | +0..+0 pts | -36%..+42% * | -16%..+22% * | -27%..+38% * | -19%..+25% * | -25%..+23% * | -12%..+27% * | -5%..+8% * | -13%..+23% * |
| claude-sonnet-5.5 | +0..+0 pts | -19%..+27% * | -3%..+22% * | -14%..+25% * | -9%..+46% * | -9%..+25% * | -4%..+27% * | -3%..+16% * | -0%..+27% * |
| gpt-6.1-sol | +0..+0 pts | -26%..+44% * | -3%..+22% * | -11%..+27% * | -9%..+14% * | -16%..+32% * | -1%..+17% * | -15%..+12% * | -2%..+12% * |

## Per measure: does the A/A interval fit inside the 5% margin at 10 epochs?

| measure | fits? | A/A interval width |
|---|---|---|
| pass rate | yes, trivially: every run passed in both arms | 0 points; this task is too easy for these models to show pass-rate noise |
| total tokens | no, for every model | 46–78 points |
| wall time | no, for every model | 25–38 points |
| turns | no | 38–65 points |
| output tokens | no | 23–55 points |
| model time | no | 34–48 points |
| other time | no | 18–39 points |
| decode rate | no, though narrowest | 13–27 points |
| normalised time | no | 14–36 points |

No measure other than a saturated pass rate fits. Noise alone puts the upper
bound of every time measure at +12% to +32%, so under today's rule (a loss is
a lower bound above +5%) a 10-epoch arm can lose on time with no effect at
all, and a real 10–20% slowdown can't be told from noise. The same holds for
tokens, whose rule is not this issue's subject but has the widest intervals
of all.

The intervals narrow roughly with the square root of the runs: getting a
±20% interval down to ±5% takes about 16 times the epochs (160 per arm),
which the full tier can't afford.

Other observations:

- **No provider gap between the arms.** Every logged call was served by the
  same upstream in both arms: Google for both Anthropic models, Azure for
  gpt-6.1-sol. Decode-rate medians match within 6%. The run logged only each
  sample's first five calls' responses (Inspect's default); this PR passes
  `--log-model-api` so every call's provider is logged from now on.
- **Most of the wall time isn't the model.** Model time is 36–51% of wall
  time; the rest is tools, the sandbox and waiting for the runner's CPUs
  (8 sandboxes on 4 vCPU). Other time is as noisy as model time.
- **Decode rate varies within a provider.** Even with one provider per model,
  per-sample decode rate moves ±8–17%, which is what quick-2's deepseek
  arms showed at a larger scale.

## The time measure the verdict should use

**Normalised time**: each sample's other time plus its output tokens at the
model's decode rate pooled over both arms.

- It counts what codecite can change: more turns and output, more tool and
  sandbox time. It drops what codecite can't: which host OpenRouter picked
  and how fast it decoded that minute. quick-2's deepseek loss (fewer turns,
  fewer input tokens, the same tool time, a slower decode) would not have
  been a time loss under it, and shouldn't have been.
- It is no noisier than wall time here: narrower for gpt-6.1-sol (14 points
  against 25) and haiku (36 against 38), slightly wider for sonnet (27
  against 25). Model time alone is noisier than either, and turns are the
  noisiest.
- Wall time stays on the scorecard, with model time, other time and decode
  rate beside it, so a host or speed gap stays visible rather than hidden.

## What the margin can't do at this n

Switching measure doesn't fix the margin: at 10 epochs normalised time's A/A
upper bound is still +12% to +27%. Three ways to resolve it, for the owner:

1. **Margin from the A/A round** (recommended): a time loss is a lower bound
   above the model's A/A upper bound for normalised time, measured once per
   panel at the tier's epochs and kept with the panel. A loss then means
   more than noise alone produced.
2. **Time advisory at 10 epochs:** report time and its interval, but only
   pass rate and tokens stop work, until a tier runs enough epochs for 5%.
3. **More epochs:** about 160 per arm for ±5%, beyond the full tier's budget.

Tokens need the same decision. Until it is made, `report.py`'s verdict and
`docs/design.md` (Measures, Success criteria) stay on wall time with the 5%
margin; once the owner picks, both change in this PR.
