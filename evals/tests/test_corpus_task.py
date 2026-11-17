"""A size-dial task (house-rules): its corpus sizes, the codecite note at each,
the staged-size guard and its scorer, without a model key or a live sandbox."""

import asyncio
import importlib.util
import io
import json
import subprocess
import sys
import tarfile
import types

import pytest
from inspect_ai.util import ExecResult

from codecite_evals import _registry, config, report, task

TASK = "house-rules"


def meta():
    return json.loads((config.task_dir(TASK) / "task.json").read_text())


def test_house_rules_is_registered():
    assert _registry.house_rules is task.house_rules


@pytest.mark.parametrize("given, size", [("", "m"), ("s", "s"), ("m", "m"), ("l", "l")])
def test_the_corpus_size_is_the_given_one_or_the_default(given, size):
    assert task.corpus_size(meta(), given) == size


def test_an_unknown_corpus_size_is_refused():
    with pytest.raises(ValueError, match="corpus must be one of s, m, l"):
        task.corpus_size(meta(), "xl")


def test_a_task_without_sizes_takes_no_corpus():
    plain = json.loads((config.task_dir("backchannel-logout") / "task.json").read_text())
    assert task.corpus_size(plain) == ""
    with pytest.raises(ValueError, match="has no corpus sizes"):
        task.corpus_size(plain, "s")
    with pytest.raises(ValueError):
        task.ab_task("backchannel-logout", "ab", "s")


def test_the_note_names_what_each_size_indexes():
    d = config.task_dir(TASK)
    s, m, l = (task.codecite_note(d, size) for size in "sml")
    for note in (s, m, l):
        assert note.startswith(f"## codecite\n\ncodecite indexes the identity-stack source at `{task.PROJECT}`")
        assert all(tool in note for tool in task.CODECITE_TOOLS)
    assert task.CORPUS_DIR not in s and "requirements eval" not in s and task.SPECS_DIR not in s
    assert f"the identity-model source under `{task.CORPUS_DIR}/<name>`" in m
    assert task.SPECS_DIR in m and "requirements eval" in m
    assert "fastapi-users" in l and "authlib" in l and "fastapi-users" not in m


def test_the_arms_differ_only_by_the_note_at_every_size():
    for size in "sml":
        base, code = task.ab_task(TASK, "ab", size).dataset
        assert base.metadata == {"arm": "baseline", "condition": "baseline", "corpus": size}
        assert code.input.startswith(base.input) and code.input != base.input
        assert code.input[len(base.input):].strip() == task.codecite_note(config.task_dir(TASK), size)


def test_a_size_dial_task_checks_the_staged_size_before_the_agent():
    solvers = task.ab_task(TASK, "baseline", "l").solver
    assert len(solvers) == 3


class StagedSandbox:
    def __init__(self, staged):
        self.staged = staged

    async def read_file(self, path):
        assert path == f"{task.CORPUS_DIR}/.size"
        if self.staged is None:
            raise FileNotFoundError(path)
        return self.staged + "\n"


@pytest.mark.parametrize("staged", ["s", None])
def test_a_sample_on_an_image_of_another_size_errors(monkeypatch, staged):
    monkeypatch.setattr(task, "sandbox", lambda: StagedSandbox(staged))
    with pytest.raises(RuntimeError, match="not 'm'"):
        asyncio.run(task.corpus_staged("m")(object(), None))


def test_a_sample_on_an_image_of_its_size_runs(monkeypatch):
    monkeypatch.setattr(task, "sandbox", lambda: StagedSandbox("m"))
    state = object()
    assert asyncio.run(task.corpus_staged("m")(state, None)) is state


class Store(dict):
    def set(self, key, value):
        self[key] = value


def tarball(files: dict[str, bytes], mtime=0, links=None) -> bytes:
    """A tarball of `files`, in the order given, at `mtime`; `links` adds symlinks."""
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w") as tar:
        for name, body in files.items():
            info = tarfile.TarInfo(name)
            info.size, info.mtime = len(body), mtime
            tar.addfile(info, io.BytesIO(body))
        for name, target in (links or {}).items():
            info = tarfile.TarInfo(name)
            info.type, info.linkname = tarfile.SYMTYPE, target
            tar.addfile(info)
    return buf.getvalue()


START = tarball({"backend/pyproject.toml": b"[tool.ruff]\n", "backend/app/main.py": b"app = 1\n"})


def test_the_start_digest_is_its_files_not_their_order_or_times():
    same = tarball({"backend/app/main.py": b"app = 1\n", "backend/pyproject.toml": b"[tool.ruff]\n"}, mtime=1_800_000_000)
    assert task.tree_digest(START) == task.tree_digest(same)
    loosened = tarball({"backend/pyproject.toml": b"", "backend/app/main.py": b"app = 1\n"})
    assert task.tree_digest(loosened) != task.tree_digest(START)
    # A link into the agent's tree, so its new file reads as unchanged, moves it too.
    linked = tarball({"backend/pyproject.toml": b"[tool.ruff]\n", "backend/app/main.py": b"app = 1\n"},
                     links={"backend/app/new.py": f"{task.PROJECT}/backend/app/new.py"})
    assert task.tree_digest(linked) != task.tree_digest(START)


def test_the_task_pins_its_start_by_digest():
    assert len(meta()["corpus"]["start_sha256"]) == 64


class CorpusSandbox:
    """Records reads, writes and execs; the start tarball reads back as
    `start`, the hidden tests pass and checks.py prints `checks`."""

    def __init__(self, checks, tests_pass=True, start=START, extracts=True, venv=True):
        self.checks, self.tests_pass, self.start, self.extracts, self.venv = checks, tests_pass, start, extracts, venv
        self.log = []

    async def read_file(self, path, text=True):
        self.log.append(("read", path))
        assert path == task.PROJECT_START and text is False
        if self.start is None:
            raise FileNotFoundError(path)
        return self.start

    async def write_file(self, path, contents):
        self.log.append(("write", path, contents))

    async def exec(self, cmd, **kwargs):
        self.log.append(("exec", cmd, kwargs))
        if cmd[0] == "python3":
            return ExecResult(success=True, returncode=0, stdout=json.dumps({"checks": self.checks}), stderr="")
        if cmd[0] == "sh":
            ok = self.extracts
            return ExecResult(success=ok, returncode=0 if ok else 2, stdout="", stderr="" if ok else "tar: bad")
        if cmd[0] == "uv":
            ok = self.venv
            return ExecResult(success=ok, returncode=0 if ok else 2, stdout="", stderr="" if ok else "uv: offline")
        ok = self.tests_pass
        return ExecResult(success=ok, returncode=0 if ok else 1, stdout="26 passed" if ok else "1 failed", stderr="")


def score(monkeypatch, sb, store=None):
    monkeypatch.setattr(task, "sandbox", lambda: sb)
    monkeypatch.setattr(task, "store", lambda: store or Store())
    # The task pins identity-stack's start by digest; the fake start stands in for it.
    pinned = meta()["corpus"]["start_sha256"]
    real = task.tree_digest
    monkeypatch.setattr(task, "tree_digest", lambda data: pinned if data == START else real(data))
    state = types.SimpleNamespace(metadata={"corpus": "s"})
    return asyncio.run(task.hidden_tests_and_checks(TASK)(state, None))


def check(name, passed, control=False):
    return {"name": name, "rule": "§1", "level": "MUST", "control": control, "passed": passed, "detail": ""}


def test_the_checks_run_on_a_verified_start_with_tools_built_from_it(monkeypatch):
    sb = CorpusSandbox([check("ruff-§1", True)])
    s = score(monkeypatch, sb)
    assert s.value == 1 and s.metadata["corpus"] == "s" and s.metadata["violations"] == 0
    execs = [e for e in sb.log if e[0] == "exec"]
    (_, extract, _), (_, venv, venv_kw), (_, tests, kw), (_, checks, checks_kw) = execs
    # The start the agent may have rewritten is read, verified on the host and
    # written back to a fresh path; the old /score/start and venv go first.
    assert ("write", "/score/start.tar", START) in sb.log
    assert extract[-1].startswith("rm -rf /score/start /score/venv && ")
    assert "/score/start.tar" in extract[-1] and task.PROJECT_START not in extract[-1]
    # The tools come from a venv built from the verified start, not the agent's .venv.
    assert venv == ["uv", "sync", "--locked", "--no-install-project"]
    assert venv_kw["cwd"] == "/score/start/backend" and venv_kw["env"]["UV_PROJECT_ENVIRONMENT"] == "/score/venv"
    assert venv_kw["env"]["UV_OFFLINE"] == "1"
    assert tests == ["/score/venv/bin/python", "-m", "pytest", "-q", "-p", "no:cacheprovider", "/score/hidden"]
    assert kw["cwd"] == f"{task.PROJECT}/backend" and kw["env"]["UV_OFFLINE"] == "1"
    assert checks == ["python3", "-I", "/score/checks.py", "/score/start", task.PROJECT]
    assert checks_kw["env"]["HOUSE_RULES_VENV"] == "/score/venv/bin"
    writes = {e[1] for e in sb.log if e[0] == "write"}
    assert {"/score/hidden/test_hidden.py", "/score/hidden/conftest.py", "/score/checks.py"} <= writes
    # Nothing reaches the sandbox before scoring writes it: the writes come first.
    assert [e[0] for e in sb.log][:3] == ["write"] * 3


@pytest.mark.parametrize(
    "sandbox, why",
    [
        (lambda: CorpusSandbox([check("ruff-§1", True)], start=tarball({"backend/pyproject.toml": b""})), "does not match"),
        (lambda: CorpusSandbox([check("ruff-§1", True)], start=b"not a tarball"), "not a tarball"),
        (lambda: CorpusSandbox([check("ruff-§1", True)], start=None), "cannot read"),
        (lambda: CorpusSandbox([check("ruff-§1", True)], extracts=False), "tar: bad"),
        (lambda: CorpusSandbox([check("ruff-§1", True)], venv=False), "uv: offline"),
    ],
)
def test_a_start_that_fails_verification_fails_the_sample(monkeypatch, sandbox, why):
    sb = sandbox()
    s = score(monkeypatch, sb)
    assert s.value == 0 and "failed checks: scorer-start" in s.explanation
    assert why in s.metadata["checks"][0]["detail"]
    # Neither the hidden tests nor checks.py run on a start that isn't the image's.
    assert not [e for e in sb.log if e[0] == "exec" and e[1][0] in ("python3", "/score/venv/bin/python")]


def test_a_failed_control_is_reported_and_not_gated(monkeypatch):
    s = score(monkeypatch, CorpusSandbox([check("ruff-§1", True), check("control-print", False, control=True)]))
    assert s.value == 1
    assert "failed checks: none; failed controls (reported, not gated): control-print" in s.explanation


def test_a_rule_violation_fails_the_sample(monkeypatch):
    s = score(monkeypatch, CorpusSandbox([check("ruff-§1", False), check("control-print", True, control=True)]))
    assert s.value == 0 and s.metadata["violations"] == 1
    assert "failed checks: ruff-§1" in s.explanation


def test_crashed_checks_fail_the_sample(monkeypatch):
    s = score(monkeypatch, CorpusSandbox([]))
    assert s.value == 0 and "checks-crashed" in s.explanation


def test_failing_hidden_tests_fail_the_sample(monkeypatch):
    s = score(monkeypatch, CorpusSandbox([check("ruff-§1", True)], tests_pass=False))
    assert s.value == 0 and s.metadata["tests_tail"] == "1 failed"


def test_a_stopped_agent_cannot_pass(monkeypatch):
    s = score(monkeypatch, CorpusSandbox([check("ruff-§1", True)]), Store(stopped="agent_timeout"))
    assert s.value == 0 and s.explanation.startswith("agent timed out; ")


def checks_module():
    spec = importlib.util.spec_from_file_location("house_rules_checks", config.task_dir(TASK) / "checks.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def test_checks_refuse_a_start_missing_its_enforcement(tmp_path):
    start, project = tmp_path / "start", tmp_path / "project"
    for tree in (start, project):
        (tree / "backend" / "app").mkdir(parents=True)
    (start / "backend" / "pyproject.toml").write_text("")
    p = subprocess.run([sys.executable, "-I", str(config.task_dir(TASK) / "checks.py"), str(start), str(project)],
                       capture_output=True, text=True)
    assert p.returncode != 0 and "backend/import_contracts.py" in p.stderr and not p.stdout


def test_ruff_findings_are_named_from_the_backend_behind_a_symlink(tmp_path):
    checks = checks_module()
    real = tmp_path / "real"
    (real / "backend" / "app").mkdir(parents=True)
    (tmp_path / "link").symlink_to(real)
    backend = tmp_path / "link" / "backend"
    # Ruff reports the resolved path; the backend was named through the link.
    assert checks.relative_name(str(real / "backend" / "app" / "x.py"), backend) == "app/x.py"
    assert checks.relative_name(str(backend / "app" / "x.py"), backend) == "app/x.py"


def test_the_gating_ruff_run_ignores_noqa():
    source = (config.task_dir(TASK) / "checks.py").read_text()
    assert '"--select", "TID251,G004,G010", "--ignore-noqa"' in source


def log(task_name, args):
    return types.SimpleNamespace(eval=types.SimpleNamespace(task=task_name, task_args=args))


def test_the_scorecard_gives_each_corpus_size_its_own_table():
    assert report.task_name(log("codecite_evals/house_rules", {"arms": "ab", "corpus": "l"})) == "house_rules@l"
    assert report.task_name(log("codecite_evals/ab", {"task_id": "house-rules", "corpus": "s"})) == "house-rules@s"
    assert report.task_name(log("codecite_evals/backchannel_logout", {"arms": "ab"})) == "backchannel_logout"
    # Left out, the size is the task's default, so a default run shares its table.
    assert report.task_name(log("codecite_evals/house_rules", {"arms": "ab"})) == "house_rules@m"
    assert report.task_name(log("codecite_evals/ab", {"task_id": "house-rules"})) == "house-rules@m"
    assert report.task_name(log("codecite_evals/ab", {"task_id": "backchannel-logout"})) == "backchannel-logout"
