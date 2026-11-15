# assist latency

How long `codecite assist` adds to the agent's grep call it rides on
(#162). `codecite-bench --assist` calls `codecite assist identity-bench
--tool grep` twice per labelled query in `evals/retrieval/queries.toml`:
with the query's text as a free-text pattern (a search) and with its first
identifier-like word as a symbol pattern (a definition lookup, falling back
to search). Each call runs three times and the fastest counts; the time
includes process start. Nothing is piped on stdin, so no hit is dropped as
already matched: this is the most assist ever prints.

Rerun with `experiments/assist-latency/run.sh` from the repository root;
the raw report is in `results.txt`.

## Results

The guardrail is a p95 under 150 ms on this corpus, already indexed.

| Calls | Count | Median ms | p95 ms | Printed a block | Median block bytes |
| --- | ---: | ---: | ---: | ---: | ---: |
| free text | 122 | 32.6 | 50.2 | 100% | 858 |
| symbol | 122 | 8.0 | 17.1 | 100% | 822 |
| all | 244 | 12.5 | 46.3 | 100% | 829 |

p95 is 46 ms, a third of the guardrail. Free-text calls cost most: they
always run a search. A symbol call is a definition lookup and only searches
when nothing is defined. Every call printed a block, since every query has
something in the corpus; in use, hits in files grep already matched are
dropped, so blocks are smaller or empty. With no index at all, every call
prints nothing in about 1 ms.
