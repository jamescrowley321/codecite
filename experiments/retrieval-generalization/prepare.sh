#!/usr/bin/env bash
# Build private codecite and CodeGraph indexes for every repository in
# corpora.toml, serially, from pinned scratch checkouts. See README.
#
#   prepare.sh <run dir> <codecite binary> <src dir> [repo...]
#
# <src dir>/<repo> must be a clean checkout at the pinned commit (README,
# "Reproduce"). Nothing here touches a shared cache, a user's checkout or
# the codecite repository: codecite's data and cache homes and CodeGraph's
# HOME are inside <run dir>, and each tool runs with no network
# (`unshare -rn`). Raw index logs and timings go to <run dir>/index/.
set -euo pipefail

run=$1 codecite=$2 src=$3
shift 3
here=$(cd "$(dirname "$0")" && pwd)
repos=("$@")
if [ ${#repos[@]} -eq 0 ]; then
  mapfile -t repos < <(python3 -c 'import sys,tomllib;[print(r["name"]) for r in tomllib.load(open(sys.argv[1],"rb"))["repo"]]' "$here/corpora.toml")
fi
mkdir -p "$run/manifests" "$run/data" "$run/cache" "$run/home" "$run/index"
export XDG_DATA_HOME=$run/data XDG_CACHE_HOME=$run/cache
export DO_NOT_TRACK=1 CODEGRAPH_NO_UPDATE_CHECK=1 CODEGRAPH_TELEMETRY=0

for repo in "${repos[@]}"; do
  read -r commit license use < <(python3 -c '
import sys, tomllib
r = {r["name"]: r for r in tomllib.load(open(sys.argv[1], "rb"))["repo"]}[sys.argv[2]]
print(r["commit"], r["license"].replace(" ", "_"), r["use"])' "$here/corpora.toml" "$repo")
  license=${license//_/ }
  checkout=$src/$repo
  test "$(git -C "$checkout" rev-parse HEAD)" = "$commit" || { echo "$repo: not at $commit" >&2; exit 1; }
  test -z "$(git -C "$checkout" status --porcelain --untracked-files=all)" || { echo "$repo: checkout not clean" >&2; exit 1; }
  manifest=$run/manifests/$repo.toml
  cat > "$manifest" <<EOF
name = "$repo"
max_age_days = 365

[[source]]
kind = "local"
name = "$repo"
path = "$checkout"
license = "$license"
use = "$use"
exclude = [".codegraph/**"]
EOF
  echo "== $repo codecite index" >&2
  /usr/bin/time -f '%e %M' -o "$run/index/$repo.codecite.time" \
    unshare -rn "$codecite" index "$repo" --manifest "$manifest" \
    > "$run/index/$repo.codecite.out" 2> "$run/index/$repo.codecite.err"
  echo "== $repo codegraph init" >&2
  HOME=$run/home /usr/bin/time -f '%e %M' -o "$run/index/$repo.codegraph.time" \
    timeout 5400 unshare -rn codegraph init -y "$checkout" \
    > "$run/index/$repo.codegraph.out" 2> "$run/index/$repo.codegraph.err" \
    || echo "$repo: codegraph init failed ($?)" >&2
done
