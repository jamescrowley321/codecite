#!/usr/bin/env bash
# Run the A/B eval at one tier, then its scorecard.
#
#   inspect/panel.sh quick|full [inspect eval options...]
#
# quick: directional, for PRs.  full: the gate in docs/design.md.
#
# Every tier runs Claude Code on Claude models (claude_code.py), billed to
# the Claude subscription CLAUDE_CODE_OAUTH_TOKEN belongs to. OPENROUTER=1
# adds pi on the tier's OpenRouter panel, billed per token to
# OPENROUTER_API_KEY; OpenRouter picks the host within the account's privacy
# settings (ZDR, allowed providers). It is off unless set.
#
# Every model runs in one Inspect process. Each sandbox creates two Docker
# networks (its own and the proxy's way out), so twice MAX_SANDBOXES must stay
# under the daemon's free address pools (about 31 by default; 12 is safe). CI gives the daemon /24 pools and raises
# it. Inspect also caps its own subprocesses and docker CLI calls at the CPU
# count and twice it, and every sandbox exec takes one, so a sample past those
# caps waits, and the wait lands in its wall time; both caps are raised to
# twice the sandboxes. RETRY=1 reruns only the missing samples of the tier's unfinished logs
# (`inspect eval-retry`) instead of starting over. Extra arguments go to
# `inspect eval` (CI passes --sandbox-prebuilt). --log-model-api keeps every
# call's raw response, which names the upstream provider; by default Inspect
# keeps only the first few.
#
# TASK_ID names the task under tasks/ (default fastapi-middleware), run as
# codecite_evals/<task> with `_` for `-`; stage its image first (prepare.sh
# <task-id>). A task's task.json may set its own epochs per tier ("epochs":
# {"quick": 1}): spec-questions has a sample per question, so one epoch is
# already a run per question. ARMS picks the arms: ab (baseline against
# codecite, the default), aa (an A/A round, two baseline arms, logged to
# inspect/logs/<tier>-aa/<task>/), or baseline (a calibration,
# evals/README.md: quick tier only, logged to inspect/logs/calibration/<task>/
# and reported with no verdict).
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
TIER="${1:-}"
shift || true
case "$TIER" in
  quick)
    TIER_EPOCHS=5
    CLAUDE_PANEL=(claude-haiku-5-5 claude-sonnet-5-5)
    # The Anthropic pair and the fastest pi model by round one's medians
    # (docs/superpowers/specs/2026-10-21-fast-eval-design.md).
    OPENROUTER_PANEL=(
      anthropic/claude-haiku-5.5
      anthropic/claude-sonnet-5.5
      openai/gpt-6.1-sol
    )
    ;;
  full)
    TIER_EPOCHS=10
    CLAUDE_PANEL=(claude-haiku-5-5 claude-sonnet-5-5)
    # The models blind-peer-review measured in its latest round
    # (evals/results/2026-10-14-ceiling-32000).
    OPENROUTER_PANEL=(
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
[[ -n "${CLAUDE_CODE_OAUTH_TOKEN:-}" ]] || { echo "CLAUDE_CODE_OAUTH_TOKEN must be set (claude setup-token)" >&2; exit 2; }
# Each agent's panel runs in its own Inspect process and log directory
# (<log dir>/claude-code/, <log dir>/openrouter/), at the same time, sharing
# MAX_SANDBOXES. The pi process never holds the Claude token, so no pi sample's
# sandbox gets it, and the Claude Code process never holds the OpenRouter key.
CLAUDE_MODELS=("${CLAUDE_PANEL[@]/#/claude-code/}")
OPENROUTER_MODELS=()
case "${OPENROUTER:-0}" in
  1)
    [[ -n "${OPENROUTER_API_KEY:-}" ]] || { echo "OPENROUTER=1 needs OPENROUTER_API_KEY" >&2; exit 2; }
    OPENROUTER_MODELS=("${OPENROUTER_PANEL[@]/#/openrouter/}")
    ;;
  0) ;;
  *)
    echo "OPENROUTER must be 0 or 1, not '$OPENROUTER'" >&2
    exit 2
    ;;
esac
TASK_ID="${TASK_ID:-fastapi-middleware}"
[[ -f "tasks/$TASK_ID/task.json" ]] || { echo "no task $TASK_ID under tasks/" >&2; exit 2; }
TASK_EPOCHS="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("epochs", {}).get(sys.argv[2], ""))' "tasks/$TASK_ID/task.json" "$TIER")"
EPOCHS="${EPOCHS:-${TASK_EPOCHS:-$TIER_EPOCHS}}"
[[ "$EPOCHS" =~ ^[1-9][0-9]*$ ]] || { echo "EPOCHS must be a positive whole number, not '$EPOCHS'" >&2; exit 2; }
ARMS="${ARMS:-ab}"
case "$ARMS" in
  ab) LOG_DIR="inspect/logs/$TIER/$TASK_ID" ;;
  aa) LOG_DIR="inspect/logs/$TIER-aa/$TASK_ID" ;;
  baseline)
    [[ "$TIER" == quick ]] || { echo "calibrate (ARMS=baseline) on the quick tier" >&2; exit 2; }
    LOG_DIR="inspect/logs/calibration/$TASK_ID"
    ;;
  *)
    echo "ARMS must be ab, aa or baseline, not '$ARMS'" >&2
    exit 2
    ;;
esac
MAX_SANDBOXES="${MAX_SANDBOXES:-12}"
mkdir -p "$LOG_DIR"
AGENTS=$((1 + (${#OPENROUTER_MODELS[@]} > 0)))
((MAX_SANDBOXES >= AGENTS)) || { echo "MAX_SANDBOXES must be at least $AGENTS, one per agent" >&2; exit 2; }
SANDBOXES=$((MAX_SANDBOXES / AGENTS))
export INSPECT_DOCKER_CLI_CONCURRENCY="${INSPECT_DOCKER_CLI_CONCURRENCY:-$((2 * SANDBOXES))}"

# One agent's panel into <log dir>/<agent>/: its models, or with RETRY=1 its
# unfinished logs. --max-retries bounds Inspect's own retries of a failing
# model call, so a spent OpenRouter key fails the samples quickly instead of
# backing off for the whole run.
panel() {
  local agent="$1" dir="$LOG_DIR/$1"
  shift
  local models=("$@")
  local limits=(--max-tasks "${#models[@]}" --max-samples 10 --max-sandboxes "$SANDBOXES"
    --max-subprocesses "$((2 * SANDBOXES))" --retry-on-error 2 --max-retries 3)
  local args=()
  [[ "$agent" == openrouter ]] && args=(-M strict_tools=false)
  mkdir -p "$dir"
  if [[ "${RETRY:-0}" == 1 ]]; then
    local unfinished
    mapfile -t unfinished < <(
      for s in started error cancelled; do
        uv run --locked inspect log list --log-dir "$dir" --status "$s" --absolute
      done
    )
    if ((${#unfinished[@]} == 0)); then
      echo "no unfinished $agent logs in $dir" >&2
      return 0
    fi
    uv run --locked inspect eval-retry "${unfinished[@]}" "${limits[@]}" --log-dir "$dir" --display log "${EXTRA[@]}"
  else
    uv run --locked inspect eval "codecite_evals/${TASK_ID//-/_}" -T arms="$ARMS" --model "$(IFS=,; echo "${models[*]}")" "${args[@]}" \
      --epochs "$EPOCHS" --log-model-api "${limits[@]}" --log-dir "$dir" --display log "${EXTRA[@]}"
  fi
}

EXTRA=("$@")
if [[ "${RETRY:-0}" == 1 ]]; then
  # A retry covers only the agents' folders: unfinished logs elsewhere (from
  # a run before the folders, or an agent now switched off) stay unfinished.
  covered=(-e "^$PWD/$LOG_DIR/claude-code/")
  ((${#OPENROUTER_MODELS[@]})) && covered+=(-e "^$PWD/$LOG_DIR/openrouter/")
  stray=$(for s in started error cancelled; do
    uv run --locked inspect log list --log-dir "$LOG_DIR" --status "$s" --absolute
  done | grep -v "${covered[@]}" || true)
  if [[ -n "$stray" ]]; then
    printf 'unfinished logs this retry will not cover:\n%s\n' "$stray" >&2
    exit 2
  fi
fi
start=$SECONDS
(unset OPENROUTER_API_KEY; panel claude-code "${CLAUDE_MODELS[@]}") &
pids=($!)
if ((${#OPENROUTER_MODELS[@]})); then
  (unset CLAUDE_CODE_OAUTH_TOKEN; panel openrouter "${OPENROUTER_MODELS[@]}") &
  pids+=($!)
fi
status=0
for pid in "${pids[@]}"; do
  wait "$pid" || status=$?
done
minutes=$(((SECONDS - start) / 60))
# The scorecard shows what finished even when a model's run failed. CI posts
# the copy in the log directory.
{
  uv run --locked codecite-eval-report --tier "$TIER" "$LOG_DIR"
  printf '\n%s, arms %s: %s models, %s epochs per sample, at most %s sandboxes: Inspect ran %s min (exit %s).\n' \
    "$TASK_ID" "$ARMS" "$((${#CLAUDE_MODELS[@]} + ${#OPENROUTER_MODELS[@]}))" "$EPOCHS" "$MAX_SANDBOXES" "$minutes" "$status"
} | tee "$LOG_DIR/scorecard.md"
exit "$status"
