# grep timing

How long `codecite grep` (#107) takes on the retrieval benchmark's corpus,
`evals/retrieval/identity-bench.toml`: identity-model at its pinned commit
and the 39 OAuth, OpenID and FAPI documents it cites.

```sh
cargo build --release
target/release/codecite index identity-bench --manifest evals/retrieval/identity-bench.toml
experiments/grep-timing/run.sh            # [path/to/codecite] [runs]
```

## Results

2026-10-26, a Claude Code cloud machine (4 vCPU), release build, 20 runs a
case, `--limit 50`. The corpus file is 42 MB; grep scans 826 code files
(6.3 MB, 159k lines) and 2,745 spec sections (2.7 MB, 41k lines).

| Case | Matches | Median ms | Max ms |
| --- | ---: | ---: | ---: |
| no match, regex | 0 | 36.1 | 47.4 |
| no match, -i | 0 | 31.6 | 39.1 |
| no match, --only spec | 0 | 12.9 | 18.6 |
| fixed "jti" | 50 | 20.6 | 24.5 |
| regex token_type_hint=\w+ | 12 | 31.7 | 43.0 |
| -i invalid_dpop_proof, spec | 3 | 15.0 | 20.0 |
| rust ^pub fn | 27 | 9.3 | 11.5 |

Times are wall time for the whole process. Starting it, opening the corpus
and returning the first line that matches `a` takes about 7.5 ms, so a scan
of every line costs about 25 to 30 ms more. A pattern that matches nothing scans
everything, the worst case; a common one stops at the limit. That leaves no
case for an index of its own (a trigram index, say) at this size.

A first version sorted the documents with their text (`ORDER BY` over rows
holding `document.text`), which copied all 6.3 MB into a temporary B-tree
before scanning the first file: that fixed cost was about 20 ms, and the
full scans 40 to 45 ms. Sorting only the small columns and reading each
file's text by id took it to the numbers above.
