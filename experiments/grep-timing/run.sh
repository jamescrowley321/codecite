#!/usr/bin/env bash
# Times `codecite grep` on the retrieval benchmark's corpus (#107).
#
# Index the corpus first:
#   codecite index identity-bench --manifest evals/retrieval/identity-bench.toml
# then run from the repository root:
#   experiments/grep-timing/run.sh [path/to/codecite] [runs]
#
# Each case runs `runs` times (default 20); the table gives the median and
# the slowest wall time per grep, process start included, and how many
# matches it printed. A pattern that matches nothing scans every line, so it
# is the worst case for a corpus.
set -euo pipefail

codecite=${1:-target/release/codecite}
runs=${2:-20}
corpus=identity-bench

cases=(
  "no match, regex|zq_no_such_text_\\d+"
  "no match, -i|zq_no_such_text|-i"
  "no match, --only spec|zq_no_such_text|--only spec"
  "fixed \"jti\"|\"jti\"|--fixed"
  "regex token_type_hint=\\w+|token_type_hint=\\w+"
  "-i invalid_dpop_proof, spec|invalid_dpop_proof|-i --only spec"
  "rust ^pub fn|^pub fn |--language rust"
)

echo "| Case | Matches | Median ms | Max ms |"
echo "| --- | ---: | ---: | ---: |"
for case in "${cases[@]}"; do
  IFS='|' read -r label pattern flags <<<"$case"
  # shellcheck disable=SC2086 # flags is a list of words
  matches=$("$codecite" grep "$corpus" "$pattern" $flags --limit 50 --json |
    { grep -o '"citation"' || true; } | wc -l)
  times=()
  for _ in $(seq "$runs"); do
    start=$(date +%s%N)
    # shellcheck disable=SC2086
    "$codecite" grep "$corpus" "$pattern" $flags --limit 50 --json >/dev/null
    end=$(date +%s%N)
    times+=($(((end - start) / 1000)))
  done
  sorted=$(printf '%s\n' "${times[@]}" | sort -n)
  median=$(sed -n "$(((runs + 1) / 2))p" <<<"$sorted")
  max=$(tail -1 <<<"$sorted")
  printf '| %s | %d | %.1f | %.1f |\n' "$label" "$matches" \
    "$(bc -l <<<"$median / 1000")" "$(bc -l <<<"$max / 1000")"
done
