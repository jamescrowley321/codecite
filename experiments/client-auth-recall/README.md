# Why codecite costs Sonnet the client-auth-uri task (#170)

On the frozen `client-auth-uri` task, Sonnet passes 13 of 15 baseline runs and
0 of 15 codecite runs. Every failure, on every model and both arms, is one hidden
test: `test_repeated_client_id_rejected`, a `client_id` sent twice. That is RFC
6749 §3.2, "Request and response parameters MUST NOT be included more than once",
which §5.2's `invalid_request` echoes as "repeats a parameter".

This replays the agents' own codecite calls offline, with no model, to see
whether codecite delivered the rule, and reads the transcripts to see what
the agent did with it.

## Data

Inspect logs from three quick-tier runs, 15 codecite samples per model in all:

| run | branch | codecite |
|---|---|---|
| 40894935686 | `main` at `b33b772` | `search`, `read` |
| 40637087363 | #167 at `69828db` | adds `requirements` |
| 40639608970 | #167 at `0bcec9d` | `requirements` with the `context` relation |

`calls.json` holds every codecite call (114) with its run, model, sample and
outcome. Corpus: `identity-bench` (`evals/retrieval/identity-bench.toml`), the
same documents and identity-model commit as the task's `eval` corpus.

## Results

### `search` doesn't surface the rule (`search-replay.md`)

- §3.2 is outside the top 20 for 18 of the 21 distinct searches, and never
  inside the result limit the agent asked for. Its best rank is 10.
- §3.1, the authorization endpoint's copy of the rule, reaches rank 5 twice.
  §5.2 ranks first only when the query already uses `invalid_request` terms,
  which only Haiku wrote.
- A spec-worded query without `--only spec` returns only identity-model code.
  "client MUST NOT use more than one authentication method in each request"
  is nearly verbatim §2.3, but its top 4–6 results are all code, with no spec
  section.

### `requirements` delivers it, and Sonnet drops it (`exposure.txt`, `sonnet-after-rule.txt`)

- On #167 at `0bcec9d`, `requirements` for §2.3 or §2.3.1 returns the §3.2
  rule under `Token Endpoint (context)`. Sonnet asked it 10 times, so the rule
  was in its context in 5 of 5 runs. It failed all 5.
- Its next turn after the rule names only what it asked about (multiple
  authentication methods, credentials in the query string). One run notes
  "duplicate params should be rejected" and then implements only a client_id
  that differs between header and body.
- Haiku passes 10 of 10 runs in which `requirements` delivered the rule.
- gpt-6.1-sol received the rule in 10 of 10 runs and read §5.2 13 times. It
  failed all of them, as it does on baseline.

### Failures (`failures.txt`)

| | baseline | codecite |
|---|---|---|
| Sonnet | 13 of 15 pass | 0 of 15 |
| Haiku | 1 of 15 | 10 of 15, all on #167 |
| gpt-6.1-sol | 0 of 15 | 0 of 15 |

## What it means

Delivery is necessary but not sufficient. Without codecite, Sonnet recalls §3.2
on its own. With codecite, it frames the task around the rule it searched for
and treats the rest of the result as background. #167 fixes delivery: Haiku
goes from 1 to 10 passes. Sonnet's loss needs the general endpoint rules to
read as rules that govern this request, not as context.

Levers, to test offline first and then with one quick-tier run each:

1. **Make the rule stand out in `requirements`:** list the rules that govern
   every request to the endpoint, such as §3.2's, as their own group that
   says it applies to this request, instead of under a `context` heading
   after the section asked about.
2. **Search mixing:** a spec-worded query without `--only` should still return
   spec sections, not only code.
3. **Instructions:** say that a section's own rules aren't the whole contract,
   and that the endpoint's general rules apply too.

## Re-running

```sh
gh run download <run id> -R jamescrowley321/codecite -D logs/<run id>
python3 -I extract.py logs/* > calls.json
XDG_DATA_HOME=$PWD/xdg codecite index identity-bench --manifest ../../evals/retrieval/identity-bench.toml
XDG_DATA_HOME=$PWD/xdg python3 -I replay.py codecite identity-bench calls.json > search-replay.md
python3 -I failures.py logs/* > failures.txt
for d in logs/*; do echo "## $d"; python3 -I exposure.py $d; done > exposure.txt
python3 -I next_turn.py logs/40639608970 > sonnet-after-rule.txt
```

`requirements` exists only on #167's branch, so replay it with that branch's
binary: `codecite requirements identity-bench "RFC 6749 §2.3.1"`.
