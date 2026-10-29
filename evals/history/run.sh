#!/usr/bin/env bash
# Run the history benchmark: for each mined repository (all of them, or the
# ones named), index its pinned corpus, score codecite and the git grep
# baseline on its queries, and rebuild results/scorecard.md.
#
#   evals/history/run.sh [repo ...]
#
# Needs release builds of codecite and codecite-bench
# (`cargo build --release --locked -p codecite -p codecite-bench`) and uv.
# A repository that fails is reported in the scorecard and the run goes on;
# the script exits non-zero at the end if any failed.
set -uo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"
bin="${CODECITE_BIN:-$root/target/release}"
clones="${HISTORY_CLONES:-$HOME/.cache/codecite/history}"
results="$here/results"
mkdir -p "$results"

if [ "$#" -eq 0 ]; then
  set -- $(cd "$here" && ls -d */ | tr -d / | grep -v '^results$')
fi

failed=0
for repo in "$@"; do
  dir="$here/$repo"
  rm -f "$results/$repo"-*
  fail() {
    printf '%s\n' "$1" | head -n 1 >"$results/$repo-failed.txt"
    echo "$repo: failed: $1" >&2
    failed=1
  }
  commit="$(sed -n 's/^commit = "\([0-9a-f]*\)"$/\1/p' "$dir/manifest.toml")"
  url="$(sed -n 's/^url = "\(.*\)"$/\1/p' "$dir/manifest.toml")"
  clone="$clones/$repo"
  if [ ! -d "$clone" ]; then
    git clone -q --filter=blob:none "$url" "$clone" 2>/dev/null || { fail "cannot clone $url"; continue; }
  fi
  git -C "$clone" cat-file -e "$commit^{commit}" 2>/dev/null || git -C "$clone" fetch -q origin
  if ! out="$("$bin/codecite" index "history-$repo" --manifest "$dir/manifest.toml" 2>&1)"; then
    fail "$(printf '%s\n' "$out" | grep -m1 -i error || printf '%s\n' "$out" | tail -n 1)"
    continue
  fi
  printf '%s\n' "$out" | grep -E '^(chunks|elapsed):' | paste -sd ' ' | sed 's/ elapsed/ · elapsed/' >"$results/$repo-index.txt"
  "$bin/codecite-bench" --codecite "$bin/codecite" --queries "$dir/queries.toml" --json \
    >"$results/$repo-codecite.json" || { fail "codecite-bench failed"; continue; }
  "$bin/codecite-bench" --queries "$dir/queries.toml" --grep "$clone" --commit "$commit" --json \
    >"$results/$repo-grep.json" || { fail "grep baseline failed"; continue; }
done

(cd "$root/evals" && uv run --locked -q codecite-history-scorecard "$results")
exit "$failed"
