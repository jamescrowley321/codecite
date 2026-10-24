#!/usr/bin/env bash
# Run the A/B panel: every model through pi, one model at a time (parallel
# sandbox starts collide on network creation). OpenRouter picks the host
# within the account's privacy settings (ZDR, allowed providers).
set -uo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
EPOCHS="${EPOCHS:-10}"
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
for model in "${PANEL[@]}"; do
  uv run --locked inspect eval inspect/task.py --model "openrouter/$model" \
    -M strict_tools=false \
    --epochs "$EPOCHS" --max-samples 10 --retry-on-error 2 --log-dir inspect/logs --display plain \
    2>&1 | grep -E "^accuracy|^total time|Error" | sed "s|^|$model: |"
done
uv run --locked python inspect/report.py
