"""Fail when a frozen task has changed since it froze.

A task's prompt, project, hidden tests and checks freeze when its first A/B
run starts (docs/design.md, Agent benchmark), and its task.json records that
run's commit as `frozen_at`. From then on every file under the task's
directory must match that commit, task.json apart from `frozen_at` itself.
A change makes it a new task with a new id. Given the base branch, it also
refuses moving a task's `frozen_at`, which would re-freeze it on new files.

    uv run codecite-eval-frozen [--base REF]

CI runs it on every PR against the base branch. With a base, it also
refuses a change to the history benchmark's mined files
(`evals/history/<repo>/`) unless queries.toml's version moves up.
"""

import argparse
import json
import subprocess
import sys
from pathlib import Path

from codecite_evals import config


def git(root, *args, check=True):
    return subprocess.run(["git", "-C", str(root), *args], capture_output=True, text=True, check=check)


def task_json(text):
    """task.json without frozen_at, which is the one field freezing adds."""
    data = json.loads(text)
    data.pop("frozen_at", None)
    return data


def show(root, ref, path):
    """A file's text at ref, or None when it isn't there."""
    result = git(root, "show", f"{ref}:{path}", check=False)
    return result.stdout if result.returncode == 0 else None


def problems(evals_dir: Path, base=None):
    """What's wrong with the frozen tasks under evals_dir/tasks, one line each."""
    root = Path(git(evals_dir, "rev-parse", "--show-toplevel").stdout.strip())
    # An unknown base would read as a task frozen nowhere, skipping the check.
    if base and git(root, "cat-file", "-e", f"{base}^{{commit}}", check=False).returncode != 0:
        return [f"base ref {base} is not a commit here (CI needs the full history)"]
    found = []
    for spec in sorted((evals_dir / "tasks").glob("*/task.json")):
        task_id = spec.parent.name
        frozen_at = json.loads(spec.read_text()).get("frozen_at")
        rel = spec.parent.resolve().relative_to(root.resolve()).as_posix()
        if base and (before := show(root, base, f"{rel}/task.json")) is not None:
            was = json.loads(before).get("frozen_at")
            if was and was != frozen_at:
                found.append(f"{task_id}: frozen_at moved from {was} to {frozen_at}; a frozen task can't re-freeze")
                continue
        if not frozen_at:
            continue
        if git(root, "cat-file", "-e", f"{frozen_at}^{{commit}}", check=False).returncode != 0:
            found.append(f"{task_id}: frozen_at {frozen_at} is not a commit here (CI needs the full history)")
            continue
        changed = git(root, "diff", "--name-only", frozen_at, "--", rel).stdout.split()
        untracked = git(root, "ls-files", "--others", "--exclude-standard", "--", rel).stdout.split()
        for path in sorted(set(changed + untracked)):
            if path == f"{rel}/task.json":
                old = show(root, frozen_at, path)
                if old is not None and task_json(old) == task_json(spec.read_text()):
                    continue
            found.append(f"{task_id}: {path} changed since it froze at {frozen_at}")
    if base:
        found += history_problems(root, evals_dir, base)
    return found


def history_version(text):
    """The `# version: N` line of a mined history queries file, or 0."""
    for line in (text or "").splitlines():
        if line.startswith("# version:"):
            return int(line.split(":", 1)[1])
    return 0


def history_problems(root, evals_dir, base):
    """Mined history queries and manifests are frozen against the base
    branch: a change needs a higher version in queries.toml, which only
    remining writes (codecite_evals.history)."""
    found = []
    history = evals_dir / "history"
    rel_history = history.resolve().relative_to(root.resolve()).as_posix()
    # Every repository mined at the base, including any deleted since.
    listed = git(root, "ls-tree", "--name-only", f"{base}:{rel_history}", check=False)
    at_base = set(listed.stdout.split()) if listed.returncode == 0 else set()
    here = {p.name for p in history.glob("*") if p.is_dir()}
    for name in sorted(at_base | here):
        mined = history / name
        rel = f"{rel_history}/{name}"
        was = show(root, base, f"{rel}/queries.toml")
        if was is None:
            continue
        if not mined.is_dir():
            found.append(f"history {name}: {rel} was mined at the base and is gone")
            continue
        now = (mined / "queries.toml").read_text() if (mined / "queries.toml").exists() else None
        try:
            remined = history_version(now) > history_version(was)
        except ValueError:
            found.append(f"history {name}: {rel}/queries.toml has a `# version:` line that isn't a number")
            continue
        if remined:
            continue
        for filename in ("manifest.toml", "queries.toml"):
            path = mined / filename
            current = path.read_text() if path.exists() else None
            if current != show(root, base, f"{rel}/{filename}"):
                found.append(f"history {name}: {rel}/{filename} changed without a new version in queries.toml")
    return found


def main():
    parser = argparse.ArgumentParser(description="Fail when a frozen task has changed since it froze.")
    parser.add_argument("--base", help="the base branch's ref, to refuse moving frozen_at")
    args = parser.parse_args()
    found = problems(config.EVALS_DIR, args.base)
    for line in found:
        print(line, file=sys.stderr)
    if found:
        sys.exit("a frozen task's files changed: give the changed task a new id instead (evals/README.md)")
    print("frozen tasks unchanged")
