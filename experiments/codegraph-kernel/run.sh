#!/usr/bin/env bash
# Run both extractors, and a full `codecite index`, over each pinned
# repository, serially, with no network. See README, "Reproduce".
#
#   run.sh <run dir> <src dir> <codecite binary> [repo...]
#
# <src dir>/<repo> must be a clean checkout at the pin in
# ../retrieval-generalization/corpora.toml. Outputs go to <run dir>/<repo>/;
# codecite's data and cache homes are inside <run dir>.
set -euo pipefail

run=$1 src=$2 codecite=$3
shift 3
here=$(cd "$(dirname "$0")" && pwd)
pins=$here/../retrieval-generalization/corpora.toml
repos=("$@")
if [ ${#repos[@]} -eq 0 ]; then repos=(ripgrep django excalidraw keycloak linux); fi
cc=$here/codecite-extract/target/release/codecite-extract
kx=$here/kernel-extract/target/release/kernel-extract
threads=$(nproc)
export XDG_DATA_HOME=$run/data XDG_CACHE_HOME=$run/cache
mkdir -p "$run/data" "$run/cache"

for repo in "${repos[@]}"; do
  commit=$(python3 -c 'import sys,tomllib;print({r["name"]:r for r in tomllib.load(open(sys.argv[1],"rb"))["repo"]}[sys.argv[2]]["commit"])' "$pins" "$repo")
  checkout=$src/$repo
  test "$(git -C "$checkout" rev-parse HEAD)" = "$commit" || { echo "$repo: not at $commit" >&2; exit 1; }
  test -z "$(git -C "$checkout" status --porcelain)" || { echo "$repo: checkout not clean" >&2; exit 1; }
  out=$run/$repo
  mkdir -p "$out"
  echo "== $repo codecite-extract" >&2
  /usr/bin/time -f '%e %M' -o "$out/codecite-extract.time" unshare -rn "$cc" "$checkout" "$out"
  echo "== $repo kernel-extract (1 thread)" >&2
  /usr/bin/time -f '%e %M' -o "$out/kernel-extract.time" unshare -rn "$kx" "$checkout" "$out/files.tsv" "$out"
  echo "== $repo kernel-extract ($threads threads)" >&2
  /usr/bin/time -f '%e %M' -o "$out/kernel-extract-mt.time" unshare -rn "$kx" "$checkout" "$out/files.tsv" "$out/mt" --threads "$threads"
  rm -f "$out/mt/kernel.jsonl"
  echo "== $repo codecite index" >&2
  cat > "$out/manifest.toml" <<EOF
name = "$repo"
max_age_days = 365

[[source]]
kind = "local"
name = "$repo"
path = "$checkout"
license = "spike"
use = "adapt"
exclude = [".codegraph/**"]
EOF
  rm -rf "$run/data/codecite/$repo"* 2>/dev/null || true
  /usr/bin/time -f '%e %M' -o "$out/codecite-index.time" unshare -rn "$codecite" index "$repo" --manifest "$out/manifest.toml" \
    > "$out/codecite-index.out" 2> "$out/codecite-index.err"
done
