#!/usr/bin/env bash
# Build the private inputs for the CodeGraph reuse benchmark (#295). See README.
#
#   prepare.sh <run dir> <codecite binary> <src dir> <codegraph snapshot dir> <scip dir> [repo...]
#
# <src dir>/<repo> must be a clean checkout at the pin in corpora.toml.
# <codegraph snapshot dir>/<repo>/.codegraph/codegraph.db is a CodeGraph
# database built from that checkout (#294's prepare.sh builds them); it is
# copied, never opened for writing, after checking that its WAL is empty.
# <scip dir>/<repo>.scip, when present, is a SCIP index a standard indexer
# produced from the same pin; corpora.toml records which indexer.
#
# For each repository this builds, inside <run dir>:
#   - codecite corpus <repo>        tree-sitter only (no SCIP);
#   - codecite corpus <repo>-scip   the same source plus its SCIP index, pinned
#                                   by sha256, when a SCIP index exists;
#   - a copy of the CodeGraph database in <src dir>/<repo>/.codegraph/ so the
#     codegraph CLI answers from a private snapshot.
# codecite's data and cache homes are inside <run dir>; each tool runs with
# no network (`unshare -rn`). Timings go to <run dir>/index/.
set -euo pipefail

run=$1 codecite=$2 src=$3 cgsnap=$4 scipdir=$5
shift 5
here=$(cd "$(dirname "$0")" && pwd)
repos=("$@")
if [ ${#repos[@]} -eq 0 ]; then
  mapfile -t repos < <(python3 -I -c 'import sys,tomllib;[print(r["name"]) for r in tomllib.load(open(sys.argv[1],"rb"))["repo"]]' "$here/corpora.toml")
fi
mkdir -p "$run/manifests" "$run/data" "$run/cache" "$run/index"
export XDG_DATA_HOME=$run/data XDG_CACHE_HOME=$run/cache
export DO_NOT_TRACK=1 CODEGRAPH_NO_UPDATE_CHECK=1 CODEGRAPH_TELEMETRY=0

manifest() { # name checkout license use [scip sha]
  local out=$run/manifests/$1.toml
  {
    printf 'name = "%s"\nmax_age_days = 365\n\n[[source]]\nkind = "local"\nname = "%s"\n' "$1" "${1%-scip}"
    printf 'path = "%s"\nlicense = "%s"\nuse = "%s"\nexclude = [".codegraph/**"]\n' "$2" "$3" "$4"
    if [ $# -ge 6 ]; then printf 'scip = "%s"\nscip_sha256 = "%s"\n' "$5" "$6"; fi
  } > "$out"
  echo "$out"
}

for repo in "${repos[@]}"; do
  read -r commit license use < <(python3 -I -c '
import sys, tomllib
r = {r["name"]: r for r in tomllib.load(open(sys.argv[1], "rb"))["repo"]}[sys.argv[2]]
print(r["commit"], r["license"].replace(" ", "_"), r["use"])' "$here/corpora.toml" "$repo")
  license=${license//_/ }
  checkout=$src/$repo
  test "$(git -C "$checkout" rev-parse HEAD)" = "$commit" || { echo "$repo: not at $commit" >&2; exit 1; }
  test -z "$(git -C "$checkout" status --porcelain --untracked-files=all)" || { echo "$repo: checkout not clean" >&2; exit 1; }

  # CodeGraph snapshot: refuse a database with pending WAL frames.
  db=$cgsnap/$repo/.codegraph/codegraph.db
  if [ -s "$db-wal" ]; then echo "$repo: $db has a nonempty WAL" >&2; exit 1; fi
  mkdir -p "$checkout/.codegraph"
  cp "$db" "$checkout/.codegraph/codegraph.db"
  cp "$cgsnap/$repo/.codegraph/.gitignore" "$checkout/.codegraph/.gitignore"
  sha256sum "$db" "$checkout/.codegraph/codegraph.db" > "$run/index/$repo.codegraph.sha256"

  m=$(manifest "$repo" "$checkout" "$license" "$use")
  echo "== $repo codecite index" >&2
  /usr/bin/time -f '%e %M' -o "$run/index/$repo.codecite.time" \
    unshare -rn "$codecite" index "$repo" --manifest "$m" \
    > "$run/index/$repo.codecite.out" 2> "$run/index/$repo.codecite.err"

  scip=$scipdir/$repo.scip
  if [ -f "$scip" ]; then
    sha=$(sha256sum "$scip" | cut -d' ' -f1)
    m=$(manifest "$repo-scip" "$checkout" "$license" "$use" "$scip" "$sha")
    echo "== $repo-scip codecite index" >&2
    /usr/bin/time -f '%e %M' -o "$run/index/$repo-scip.codecite.time" \
      unshare -rn "$codecite" index "$repo-scip" --manifest "$m" \
      > "$run/index/$repo-scip.codecite.out" 2> "$run/index/$repo-scip.codecite.err"
  fi
done
