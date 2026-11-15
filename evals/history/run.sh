#!/usr/bin/env bash
# Run the history benchmark: for each mined repository (all of them, or the
# ones named), index its pinned corpus, score codecite, the git grep
# baseline and the query-blind popular-files floor on its queries, measure
# the output bytes and time of codecite and rg per query, and rebuild
# results/scorecard.md.
#
#   evals/history/run.sh [repo ...]
#
# Needs release builds of codecite and codecite-bench
# (`cargo build --release --locked -p codecite -p codecite-bench`), uv and
# ripgrep (RG names the program, default `rg`).
# A repository that fails is reported in the scorecard and the run goes on;
# one whose cost step fails keeps its recall rows, with the failure under the
# cost table. The script exits non-zero at the end if any repository failed;
# a failed cost step alone doesn't, since cost is a report, not a gate.
set -uo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"
bin="${CODECITE_BIN:-$root/target/release}"
clones="${HISTORY_CLONES:-$HOME/.cache/codecite/history}"
results="$here/results"
mkdir -p "$results"
# A fresh index every run, so each repository's chunk count and time in the
# scorecard describe a full build, not what changed since the last one.
XDG_DATA_HOME="$(mktemp -d)"
export XDG_DATA_HOME
checkout="$(mktemp -d)"
trap 'rm -rf "$XDG_DATA_HOME" "$checkout"' EXIT

if [ "$#" -eq 0 ]; then
  set -- $(cd "$here" && ls -d */ | tr -d / | grep -v '^results$')
fi

failed=0
for repo in "$@"; do
  dir="$here/$repo"
  for kind in codecite.json grep.json popular.json cost.json index.txt failed.txt; do
    rm -f "$results/$repo-$kind"
  done
  # `.`, not `-`, so no repository's `<repo>-failed.txt` reads as another's.
  rm -f "$results/$repo.cost-failed.txt"
  fail() {
    printf '%s\n' "$1" | head -n 1 >"$results/$repo-failed.txt"
    echo "$repo: failed: $1" >&2
    failed=1
  }
  # Reported, but leaves the exit status alone.
  cost_failed() {
    printf '%s\n' "$1" | head -n 1 >"$results/$repo.cost-failed.txt"
    echo "$repo: cost failed: $1" >&2
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
  # Each report is written whole or not at all, so a failed run leaves no
  # partial JSON for the scorecard to trip on.
  report() {
    local engine="$1"
    shift
    if "$bin/codecite-bench" --queries "$dir/queries.toml" --json "$@" >"$results/.$repo-$engine.json"; then
      mv "$results/.$repo-$engine.json" "$results/$repo-$engine.json"
    else
      rm -f "$results/.$repo-$engine.json"
      return 1
    fi
  }
  report codecite --codecite "$bin/codecite" || { fail "codecite-bench failed for codecite"; continue; }
  report grep --grep "$clone" --commit "$commit" || { fail "codecite-bench failed for grep"; continue; }
  report popular --popular || { fail "codecite-bench failed for popular"; continue; }
  # The cost report compares with rg over the pinned tree, extracted from
  # the clone so no work-tree state leaks in.
  mkdir -p "$checkout/$repo"
  git -C "$clone" archive "$commit" | tar -x -C "$checkout/$repo" || { cost_failed "cannot extract $commit"; continue; }
  report cost --codecite "$bin/codecite" --rg "${RG:-rg}" --cost "$checkout/$repo" ||
    cost_failed "codecite-bench failed for cost"
done

# A scorecard that can't be rebuilt is removed, not left standing as this
# run's result.
rm -f "$results/scorecard.md"
if ! (cd "$root/evals" && uv run --locked -q codecite-history-scorecard "$results"); then
  echo "scorecard: failed" >&2
  failed=1
fi
exit "$failed"
