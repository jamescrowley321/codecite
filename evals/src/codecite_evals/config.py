"""Where the eval's files live, resolved in one place.

The task directories, the sandbox's compose file and the logs are in the
checkout's evals/ directory, not in the package. CODECITE_EVALS_DIR names that
directory; unset, it is the evals/ this package was installed from, which is
where `uv sync` (an editable install) leaves it.
"""

import os
from pathlib import Path

EVALS_DIR = Path(os.environ.get("CODECITE_EVALS_DIR") or Path(__file__).resolve().parents[2])
ROOT = EVALS_DIR.parent
COMPOSE = EVALS_DIR / "inspect" / "compose.yaml"
COMPOSE_OFFLINE = EVALS_DIR / "inspect" / "compose-offline.yaml"
LOGS = EVALS_DIR / "inspect" / "logs"
# The task's arm settings (`-T arms=`), each two arms of (label, condition).
# The label names the arm on the scorecard, the first being the reference; the
# condition sets its prompt and sandbox.
ARMS = {
    "ab": (("baseline", "baseline"), ("codecite", "codecite")),
    "aa": (("baseline-a", "baseline"), ("baseline-b", "baseline")),
    # A calibration: the baseline arm alone, reported with no verdict.
    "baseline": (("baseline", "baseline"),),
}

QUERIES = EVALS_DIR / "retrieval" / "queries.toml"
BENCH_MANIFEST = EVALS_DIR / "retrieval" / "identity-bench.toml"


def task_dir(task_id: str) -> Path:
    """A task's directory: task.json, prompt.md, project/, hidden/, checks.py."""
    path = EVALS_DIR / "tasks" / task_id
    if not path.is_dir():
        raise FileNotFoundError(f"no task {task_id!r} at {path} (set CODECITE_EVALS_DIR to the evals/ directory)")
    return path
