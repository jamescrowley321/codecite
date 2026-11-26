"""The panel keeps each arm set's logs and retries apart."""

import os
import shutil
import subprocess

import pytest

from codecite_evals import config


@pytest.mark.parametrize("arms, run_name", [("ab", "quick"), ("aa", "quick-aa"), ("assist", "quick-assist"), ("tools", "quick-tools"), ("baseline", "calibration")])
def test_panel_logs_and_retries_only_its_arm_set(tmp_path, arms, run_name):
    inspect = tmp_path / "inspect"
    inspect.mkdir()
    shutil.copy(config.EVALS_DIR / "inspect/panel.sh", inspect / "panel.sh")
    task = tmp_path / "tasks/fastapi-middleware"
    task.mkdir(parents=True)
    (task / "task.json").write_text('{}')
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    uv = bin_dir / "uv"
    uv.write_text('#!/bin/sh\nprintf "%s\\n" "$*" >> "$CALLS"\n')
    uv.chmod(0o755)
    calls = tmp_path / "calls"
    env = {"PATH": f"{bin_dir}:{os.defpath}", "HOME": str(tmp_path), "CLAUDE_CODE_OAUTH_TOKEN": "test-only", "ARMS": arms, "CALLS": str(calls)}
    script = str(inspect / "panel.sh")
    subprocess.run([script, "quick"], env=env, check=True, capture_output=True, text=True)
    directory = f"inspect/logs/{run_name}/fastapi-middleware"
    text = calls.read_text()
    assert f"-T arms={arms}" in text
    assert f"--log-dir {directory}/claude-code" in text
    assert f"codecite-eval-report --tier quick {directory}" in text
    assert (tmp_path / directory / "scorecard.md").exists()
    calls.write_text("")
    subprocess.run([script, "quick"], env=env | {"RETRY": "1"}, check=True, capture_output=True, text=True)
    assert f"--log-dir {directory}/claude-code" in calls.read_text()


def test_dispatch_tools_choice_uses_panel_log_and_artifact_name():
    workflow = (config.ROOT / ".github/workflows/evals.yml").read_text()
    assert "options: [ab, aa, baseline, assist, plugin, tools]" in workflow
    assert "inputs.arms == 'tools' && '-tools'" in workflow
    assert "eval-logs-${{ env.RUN_NAME }}-${{ env.LOG_TASK }}" in workflow
