# One corpus of every history repository

`codecite add` (#184) builds a corpus from repository URLs alone. This run
adds the five history benchmark repositories to one corpus, `history-all`,
and indexes it, to see what a many-repository corpus costs.

Rerun from the repository root after a release build:

```sh
cargo build --release --locked -p codecite
experiments/many-repos/build.sh
```

The script reads each URL from `evals/history/*/manifest.toml`, runs
`codecite add history-all <urls>` in a temp work dir, copies the manifest it
wrote to [`history-all.toml`](history-all.toml), and indexes it into a temp
data dir under `/usr/bin/time -v`. Its full output is in [`run.txt`](run.txt).

## Results

Run on 2026-10-30, release build of `feat/codecite-add`.

| Source | Pin | License | Use |
| --- | --- | --- | --- |
| blind-peer-review | `5986580b8677` | Apache-2.0 | adapt |
| codecite | `56fc2dd2b35c` | Apache-2.0 | adapt |
| identity-model | `2fee67deb27f` | Apache-2.0 | adapt |
| identity-stack | `bef1c599e360` | none detected | approach_only |
| terraform-provider-descope | `c45225cfd391` | MIT | adapt |

Each pin is the commit the remote's default branch pointed to on the day.
identity-stack has no license file (GitHub's license API returns 404 for it
too), so `add` proposed `approach_only`, as it should.

| Measure | Value |
| --- | ---: |
| Documents | 2,381 |
| Chunks | 25,243 |
| Skipped | 11 deny-listed, 14 symlinks, 2 binary |
| `index` wall time | 3.86 s |
| `index` peak RSS | 27 MB |
| Corpus file | 116 MB |

`index` started from the clones `add` left in the cache, so its wall time is
the indexing alone; `add` is the step that touches the network. Five
repositories in one corpus index in under four seconds in a small, steady
amount of memory, so a corpus of many repositories is bounded by disk, not by
indexing time.
