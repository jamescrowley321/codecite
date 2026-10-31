#!/usr/bin/env bash
# Builds one corpus of every history benchmark repository with `codecite add`
# (#184), then indexes it and records what that cost.
#
# Run from the repository root after a release build:
#   experiments/many-repos/build.sh [path/to/codecite]
#
# It reads each repository's URL from evals/history/*/manifest.toml, adds them
# all to a fresh manifest in a temp work dir, copies that manifest here as
# history-all.toml, and indexes it into a temp data dir under
# `/usr/bin/time -v`. Everything it printed goes to run.txt here: the add
# lines (pin, license, use), the index summary, the wall time, the peak RSS
# and the corpus file's size. Clones land in the usual cache, so a rerun
# skips the network for repositories it already has.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"
codecite="$(realpath "${1:-$root/target/release/codecite}")"
corpus=history-all

work="$(mktemp -d)"
XDG_DATA_HOME="$(mktemp -d)"
export XDG_DATA_HOME
trap 'rm -rf "$work" "$XDG_DATA_HOME"' EXIT

mapfile -t urls < <(sed -n 's/^url = "\(.*\)"$/\1/p' "$root"/evals/history/*/manifest.toml)

{
  echo "\$ codecite add $corpus ${urls[*]}"
  (cd "$work" && "$codecite" add "$corpus" "${urls[@]}")
  cp "$work/corpora/$corpus.toml" "$here/$corpus.toml"

  echo
  echo "\$ /usr/bin/time -v codecite index $corpus"
  (cd "$work" && /usr/bin/time -v "$codecite" index "$corpus") 2>&1 |
    grep -v -E '^\s+(Average|Exit status|Page size|Socket|Signals|Swaps|Major|Minor|Voluntary|Involuntary|File system|Percent of CPU)'

  echo
  echo "corpus file: $(stat -c %s "$XDG_DATA_HOME/codecite/$corpus.db") bytes"
} | tee "$here/run.txt"
