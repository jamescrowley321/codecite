#!/usr/bin/env bash
# Sweep the two ranking weights that apply to every chunk, one knob at a
# time, on the history benchmark (#183):
#
#   docs        the `Docs` role weight in role.rs (today 0.8, adopted from
#               this sweep; its base row was measured at 1.0): prose
#               against code
#   name-boost  NAME_BOOST in store/search.rs (today 3.0)
#
#   experiments/one-ranking/sweep.sh [knob=value ...]
#
# For each value it patches the constant (the constants stay `const`; both
# files are copied aside first and put back after each run and on exit, so
# uncommitted edits to them survive), rebuilds release, indexes every history
# repository afresh, runs the codecite arm only and writes the pooled `all`
# rows to results/<knob>-<value>.md. The grep, popular and cost arms don't
# depend on ranking, so their committed reports are reused for the scorecard.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"
history="$root/evals/history"
clones="${HISTORY_CLONES:-$HOME/.cache/codecite/history}"
role="$root/crates/codecite-core/src/role.rs"
search="$root/crates/codecite-core/src/store/search.rs"
saved="$(mktemp -d)"
cp "$role" "$saved/role.rs"
cp "$search" "$saved/search.rs"
restore() { cp "$saved/role.rs" "$role"; cp "$saved/search.rs" "$search"; }
trap 'restore; rm -rf "$saved"' EXIT

if [ "$#" -eq 0 ]; then
  set -- docs=0.6 docs=0.8 docs=1.2 name-boost=2.0 name-boost=5.0
fi

for run in "$@"; do
  knob="${run%%=*}"
  value="${run#*=}"
  restore
  case "$knob" in
    docs)
      sed -i -E 's/(Role::Docs => )[0-9.]+,/\1'"$value"',/' "$role"
      grep -q "Role::Docs => $value," "$role" ||
        { echo "could not set the Docs weight in $role" >&2; exit 1; }
      ;;
    name-boost)
      sed -i -E 's/^(const NAME_BOOST: f64 = )[0-9.]+;/\1'"$value"';/' "$search"
      grep -q "^const NAME_BOOST: f64 = $value;" "$search" ||
        { echo "could not set NAME_BOOST in $search" >&2; exit 1; }
      ;;
    *) echo "unknown knob: $knob" >&2; exit 2 ;;
  esac
  (cd "$root" && cargo build --release --locked -q -p codecite -p codecite-bench)

  results="$(mktemp -d)"
  XDG_DATA_HOME="$(mktemp -d)"
  export XDG_DATA_HOME
  cp "$history/results/"*-grep.json "$history/results/"*-popular.json "$results/"
  for dir in "$history"/*/; do
    repo="$(basename "$dir")"
    [ "$repo" = results ] && continue
    commit="$(sed -n 's/^commit = "\([0-9a-f]*\)"$/\1/p' "$dir/manifest.toml")"
    git -C "$clones/$repo" cat-file -e "$commit^{commit}"
    "$root/target/release/codecite" index "history-$repo" --manifest "$dir/manifest.toml" >/dev/null
    "$root/target/release/codecite-bench" --queries "$dir/queries.toml" --json \
      --codecite "$root/target/release/codecite" >"$results/$repo-codecite.json"
  done
  (cd "$root/evals" && uv run --locked -q codecite-history-scorecard "$results" >/dev/null)
  {
    echo "# $knob = $value"
    echo
    sed -n '3,4p' "$results/scorecard.md"
    grep '| all | codecite |' "$results/scorecard.md"
  } >"$here/results/$knob-$value.md"
  cat "$here/results/$knob-$value.md"
  rm -rf "$results" "$XDG_DATA_HOME"
done
