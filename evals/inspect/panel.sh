#!/usr/bin/env bash
# Run the A/B eval at one tier, then its scorecard. OpenRouter picks the host
# within the account's privacy settings (ZDR, allowed providers).
#
#   inspect/panel.sh quick|full [inspect eval options...]
#
# quick: 3 fast models, 5 epochs; directional, for PRs.
# full:  the whole panel, 10 epochs; the gate in docs/design.md.
#
# Every model runs in one Inspect process. Each sandbox creates its own Docker
# network, so MAX_SANDBOXES must stay under the daemon's free address pools
# (about 31 by default; 14 is safe). CI gives the daemon /24 pools and raises
# it. Inspect also caps its own subprocesses and docker CLI calls at the CPU
# count and twice it, and every sandbox exec takes one, so a sample past those
# caps waits, and the wait lands in its wall time; both caps are raised to
# twice MAX_SANDBOXES. RETRY=1 reruns only the missing samples of the tier's unfinished logs
# (`inspect eval-retry`) instead of starting over. Extra arguments go to
# `inspect eval` (CI passes --sandbox-prebuilt).
#
# TASK_ID names the task under tasks/ (default fastapi-middleware), run as
# codecite_evals/<task> with `_` for `-`; stage its image first (prepare.sh
# <task-id>). A task's task.json may set its own epochs per tier ("epochs":
# {"quick": 1}): spec-questions has a sample per question, so one epoch is
# already a run per question. ARMS=baseline runs only the baseline arm, a
# calibration (evals/README.md): quick tier only, logged to
# inspect/logs/calibration/<task>/ and reported with no verdict.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
TIER="${1:-}"
shift || true
case "$TIER" in
  quick)
    TIER_EPOCHS=5
    # The Anthropic pair and the fastest pi model by round one's medians
    # (docs/superpowers/specs/2026-10-21-fast-eval-design.md).
    PANEL=(
      anthropic/claude-haiku-5.5
      anthropic/claude-sonnet-5.5
      openai/gpt-6.1-sol
    )
    ;;
  full)
    TIER_EPOCHS=10
    # The models blind-peer-review measured in its latest round
    # (evals/results/2026-10-14-ceiling-32000).
    PANEL=(
      anthropic/claude-haiku-5.5
      anthropic/claude-sonnet-5.5
      google/gemini-3.8-flash
      google/gemini-4-argon
      openai/gpt-6.1-sol
      openai/gpt-6-luna-pro
      z-ai/glm-5.2
      z-ai/glm-5.3
      mistralai/mistral-large-4-0
    )
    ;;
  *)
    echo "usage: $0 quick|full [inspect eval options...]" >&2
    exit 2
    ;;
esac
TASK_ID="${TASK_ID:-fastapi-middleware}"
[[ -f "tasks/$TASK_ID/task.json" ]] || { echo "no task $TASK_ID under tasks/" >&2; exit 2; }
TASK_EPOCHS="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("epochs", {}).get(sys.argv[2], ""))' "tasks/$TASK_ID/task.json" "$TIER")"
EPOCHS="${EPOCHS:-${TASK_EPOCHS:-$TIER_EPOCHS}}"
ARMS="${ARMS:-both}"
case "$ARMS" in
  both) LOG_DIR="inspect/logs/$TIER/$TASK_ID" ;;
  baseline)
    [[ "$TIER" == quick ]] || { echo "calibrate (ARMS=baseline) on the quick tier" >&2; exit 2; }
    LOG_DIR="inspect/logs/calibration/$TASK_ID"
    ;;
  *) echo "ARMS must be both or baseline" >&2; exit 2 ;;
esac
MAX_SANDBOXES="${MAX_SANDBOXES:-14}"
mkdir -p "$LOG_DIR"
export INSPECT_DOCKER_CLI_CONCURRENCY="${INSPECT_DOCKER_CLI_CONCURRENCY:-$((2 * MAX_SANDBOXES))}"
LIMITS=(--max-tasks "${#PANEL[@]}" --max-samples 10 --max-sandboxes "$MAX_SANDBOXES"
  --max-subprocesses "$((2 * MAX_SANDBOXES))" --retry-on-error 2)

start=$SECONDS
status=0
if [[ "${RETRY:-0}" == 1 ]]; then
  mapfile -t unfinished < <(
    for s in started error cancelled; do
      uv run --locked inspect log list --log-dir "$LOG_DIR" --status "$s" --absolute
    done
  )
  if ((${#unfinished[@]})); then
    uv run --locked inspect eval-retry "${unfinished[@]}" "${LIMITS[@]}" --log-dir "$LOG_DIR" --display log "$@" || status=$?
  else
    echo "no unfinished logs in $LOG_DIR" >&2
  fi
else
  models="$(printf 'openrouter/%s,' "${PANEL[@]}")"
  uv run --locked inspect eval "codecite_evals/${TASK_ID//-/_}" -T arms="$ARMS" --model "${models%,}" -M strict_tools=false \
    --epochs "$EPOCHS" "${LIMITS[@]}" --log-dir "$LOG_DIR" --display log "$@" || status=$?
fi
minutes=$(((SECONDS - start) / 60))
# The scorecard shows what finished even when a model's run failed. CI posts
# the copy in the log directory.
{
  uv run --locked codecite-eval-report --tier "$TIER" "$LOG_DIR"
  printf '\n%s, arms %s: %s models, %s epochs per sample, at most %s sandboxes: Inspect ran %s min (exit %s).\n' \
    "$TASK_ID" "$ARMS" "${#PANEL[@]}" "$EPOCHS" "$MAX_SANDBOXES" "$minutes" "$status"
} | tee "$LOG_DIR/scorecard.md"
exit "$status"
