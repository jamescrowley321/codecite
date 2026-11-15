#!/usr/bin/env bash
# Measures `codecite assist`'s latency on the retrieval benchmark's corpus
# (#162): every labelled query as a free-text grep pattern and its first
# word as a symbol pattern, three runs each, the fastest counting.
#
# Run from the repository root:
#   experiments/assist-latency/run.sh
# It builds release, indexes identity-bench into a scratch data directory
# (XDG_DATA_HOME, default /var/tmp/assist-latency) unless it is already
# there, and writes results.txt next to this script. The guardrail is a
# p95 under 150 ms.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
export XDG_DATA_HOME=${XDG_DATA_HOME:-/var/tmp/assist-latency}

cargo build --release --locked -p codecite -p codecite-bench
target/release/codecite index identity-bench \
  --manifest evals/retrieval/identity-bench.toml >&2
{
  echo "# codecite-bench --assist on identity-bench, $(git rev-parse --short HEAD)"
  echo "# $(uname -srm), $(nproc) CPUs"
  target/release/codecite-bench --assist
} >"$here/results.txt"
cat "$here/results.txt"
