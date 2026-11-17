"""A size-dial task (house-rules): its corpus sizes, the codecite note at each,
the staged-size guard and its scorer, without a model key or a live sandbox."""

import asyncio
import importlib.util
import io
import json
import shutil
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


def expected_hidden():
    return checks_module().expected_hidden(config.task_dir(TASK) / "hidden")


def report_xml(cases):
    """A junit report of (name, outcome) cases: outcome passed, failure, error or skipped."""
    body = "".join(
        f'<testcase classname="test_hidden" name="{n}">' + ("" if o == "passed" else f'<{o} message="x"/>') + "</testcase>"
        for n, o in cases
    )
    return f'<testsuites><testsuite name="pytest">{body}</testsuite></testsuites>'


def passing_report(skip=()):
    return report_xml([(f"{n}[0]", "passed") for n in sorted(expected_hidden()) if n not in skip])


class CorpusSandbox:
    """Records reads, writes and execs; the start tarball reads back as
    `start`, the hidden tests pass and checks.py prints `checks`."""

    def __init__(self, checks, tests_pass=True, start=START, extracts=True, venv=True, report=None):
        self.checks, self.tests_pass, self.start, self.extracts, self.venv = checks, tests_pass, start, extracts, venv
        self.report = passing_report() if report is None else report
        self.log = []

    async def read_file(self, path, text=True):
        self.log.append(("read", path))
        if path == task.HIDDEN_REPORT:
            if self.report is False:
                raise FileNotFoundError(path)
            return self.report
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
        if cmd[0] == "sh" and "tar -xf" not in cmd[-1]:
            return ExecResult(success=True, returncode=0, stdout="", stderr="")
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


HIDDEN_CMD = ["/score/venv/bin/python", "-P", "-m", "pytest", "-q", "-p", "no:cacheprovider", "--junitxml=/score/report/hidden.xml",
              "-c", "/dev/null", "--rootdir=/score/hidden", "--confcutdir=/score/hidden",
              "-o", f"pythonpath={task.PROJECT}/backend", "/score/hidden"]
CLEAR = (task.KILL_LEFTOVERS + "\nrm -rf /score/start /score/venv /score/hidden /score/report /score/start.tar /score/checks.py"
         " && mkdir -p /score/start /score/hidden /score/report")
EXTRACT = "tar -xf /score/start.tar -C /score/start"
UV = ["uv", "sync", "--locked", "--no-install-project"]


def steps(log):
    """The scorer's steps, short: a write by path, an exec by its command
    (a shell's by its script), a read."""
    out = []
    for e in log:
        if e[0] == "exec":
            out.append(("exec", e[1][-1] if e[1][0] == "sh" else e[1][0]))
        else:
            out.append((e[0], e[1]))
    return out


def test_the_checks_run_on_a_verified_start_with_tools_built_from_it(monkeypatch):
    sb = CorpusSandbox([check("ruff-§1", True)])
    s = score(monkeypatch, sb)
    assert s.value == 1 and s.metadata["corpus"] == "s" and s.metadata["violations"] == 0
    lay_out = [("exec", CLEAR), ("write", "/score/start.tar"), ("exec", EXTRACT), ("exec", "uv")]
    hidden = sorted(("write", f"/score/hidden/{f.name}") for f in (config.task_dir(TASK) / "hidden").glob("*.py"))
    assert steps(sb.log) == [
        # All of /score goes first, so nothing the agent left there is in reach.
        ("exec", "rm -rf /score && mkdir -p /score/hidden"),
        # The start is read once, verified on the host, and laid out from
        # those bytes, never from PROJECT_START in the sandbox.
        ("read", task.PROJECT_START),
        *lay_out,
        # checks.py goes in only once the agent's leftover processes are
        # gone, so none can swap it before it runs.
        ("write", "/score/checks.py"),
        ("exec", "python3"),
        # checks.py ran the agent's code as root, so everything that code
        # could reach is laid out again from the verified bytes, and the
        # hidden tests are written only now, just before they run.
        *lay_out,
        *hidden,
        ("exec", "/score/venv/bin/python"),
        # The hidden tests' verdict is read back on the host from their report.
        ("read", task.HIDDEN_REPORT),
    ]
    assert all(e[2] == START for e in sb.log if e[0] == "write" and e[1] == "/score/start.tar")
    execs = {e[1][0] if e[1][0] != "sh" else e[1][-1]: e for e in sb.log if e[0] == "exec"}
    _, venv, venv_kw = execs["uv"]
    assert venv == UV and venv_kw["cwd"] == "/score/start/backend"
    assert venv_kw["env"]["UV_PROJECT_ENVIRONMENT"] == "/score/venv" and venv_kw["env"]["UV_OFFLINE"] == "1"
    _, checks, checks_kw = execs["python3"]
    assert checks == ["python3", "-I", "/score/checks.py", "/score/start", task.PROJECT]
    assert checks_kw["env"]["HOUSE_RULES_VENV"] == "/score/venv/bin" and checks_kw["timeout"] == task.CHECKS_TIMEOUT
    _, tests, kw = execs["/score/venv/bin/python"]
    assert tests == HIDDEN_CMD
    assert kw["cwd"] == f"{task.PROJECT}/backend" and kw["env"]["UV_OFFLINE"] == "1"


@pytest.mark.parametrize(
    "report, why",
    [
        (False, "no report"),
        ("<not xml", "no report"),
        (lambda: passing_report(skip={"test_the_domain_accepts_a_slug"}), "missing"),
        (lambda: report_xml([(f"{n}[0]", "skipped" if i == 0 else "passed") for i, n in enumerate(sorted(expected_hidden()))]), "skipped"),
        (lambda: report_xml([(f"{n}[0]", "failure" if i == 0 else "passed") for i, n in enumerate(sorted(expected_hidden()))]), "failure"),
    ],
)
def test_a_clean_exit_without_a_full_passing_report_fails_the_hidden_tests(monkeypatch, report, why):
    # Code the hidden tests import can end the process with status 0 before
    # any test runs; only a report naming every hidden test as passed counts.
    sb = CorpusSandbox([check("ruff-§1", True)], report=report() if callable(report) else report)
    s = score(monkeypatch, sb)
    assert s.value == 0 and not s.metadata["tests_passed"] and why in s.metadata["tests_tail"]


def test_the_expected_hidden_tests_come_from_the_hidden_files():
    names = expected_hidden()
    assert "test_the_domain_accepts_a_slug" in names and "test_import_needs_a_token" in names and len(names) == 11


def test_a_passing_report_and_a_clean_exit_pass():
    checks = checks_module()
    assert checks.hidden_verdict(expected_hidden(), passing_report(), 0) == (True, "")
    assert not checks.hidden_verdict(expected_hidden(), passing_report(), 1)[0]


def test_an_existing_route_that_gains_a_hand_built_answer_or_a_dct_read_breaks_the_rule():
    checks = checks_module()
    route = "backend/app/routers/roles.py"
    before = {route: "async def create_role(request):\n    return result_to_response(r, request)\n"}
    raised = {route: "async def create_role(request):\n    raise HTTPException(status_code=422)\n    return result_to_response(r, request)\n"}
    dct = {route: 'async def create_role(request):\n    t = claims["dct"]\n    return result_to_response(r, request)\n'}
    for after, failing in ((raised, "result-to-response"), (dct, "tenant-from-dependency")):
        results = []
        checks.static_checks(results, None, None, after, before)
        assert [c["name"] for c in results if not c["passed"]] == [failing]
    # A route that keeps the ones it had is left alone.
    kept = {route: "async def create_role(request):\n    raise HTTPException(status_code=422)\n"}
    results = []
    checks.static_checks(results, None, None, {route: kept[route] + "\n# edited\n"}, kept)
    assert all(c["passed"] for c in results)


def test_the_copy_of_the_agents_tree_survives_dangling_links_and_loops(tmp_path):
    checks = checks_module()
    project = tmp_path / "project"
    (project / "backend" / "app").mkdir(parents=True)
    (project / "backend" / "app" / "main.py").write_text("app = 1\n")
    (project / "backend" / ".env").symlink_to("../.env.missing")
    (project / "backend" / "app" / "gone.py").symlink_to("nowhere.py")
    (project / "backend" / "loop").symlink_to(project / "backend")
    (project / "docs").mkdir()
    (project / "docs" / "dead").symlink_to("missing")
    work = tmp_path / "work"
    work.mkdir()
    assert checks.copy_project(project, work) is None
    assert (work / "backend" / "app" / "main.py").read_text() == "app = 1\n"
    assert (work / "backend" / ".env").is_symlink() and (work / "docs" / "dead").is_symlink()
    assert set(checks.app_files(project)) == {"backend/app/main.py"}


def test_a_linked_directory_in_the_agents_backend_is_dropped_from_the_copy(tmp_path):
    # Through a linked tests/, the enforcement would be written into the
    # agent's own directory, and a conftest.py there would escape the scrub.
    checks = checks_module()
    project, elsewhere = tmp_path / "project", tmp_path / "elsewhere"
    (project / "backend" / "app").mkdir(parents=True)
    (elsewhere / "unit").mkdir(parents=True)
    (elsewhere / "conftest.py").write_text("agent")
    (project / "backend" / "tests").symlink_to(elsewhere, target_is_directory=True)
    (project / "backend" / "app" / "linked").symlink_to(elsewhere, target_is_directory=True)
    work = tmp_path / "work"
    work.mkdir()
    assert checks.copy_project(project, work) is None
    assert not (work / "backend" / "tests").exists() and not (work / "backend" / "tests").is_symlink()
    assert not (work / "backend" / "app" / "linked").is_symlink()
    assert (elsewhere / "conftest.py").read_text() == "agent"


def test_a_copy_that_fails_is_named_as_one_failed_check(tmp_path, monkeypatch):
    checks = checks_module()

    def broken(*args, **kwargs):
        raise shutil.Error([("a", "b", "boom")])

    monkeypatch.setattr(checks.shutil, "copytree", broken)
    (tmp_path / "project" / "backend").mkdir(parents=True)
    assert "boom" in checks.copy_project(tmp_path / "project", tmp_path / "work")


def test_the_kill_spares_init_the_containers_main_process_and_itself():
    # compose.yaml runs the sandbox with `init: true`: PID 1 is tini, which
    # exits, stopping the container, when its child (the image's `tail`) dies.
    kill = task.KILL_LEFTOVERS
    assert "-1" not in kill.replace("PPid", "")
    assert "1|$$|" in kill and "starttime" in kill
    assert subprocess.run(["sh", "-n", "-c", kill]).returncode == 0  # parses; never run on the host


SMOKE_IMAGE = "node:24-trixie-slim"  # the sandbox image's base (inspect/Dockerfile)


def docker_ready() -> bool:
    try:
        return subprocess.run(["docker", "image", "inspect", SMOKE_IMAGE], capture_output=True).returncode == 0
    except OSError:
        return False


class DockerSandbox:
    """Inspect's sandbox calls, as plain docker exec on one container."""

    def __init__(self, cid):
        self.cid = cid

    async def exec(self, cmd, cwd=None, env=None, timeout=None):
        args = ["docker", "exec", *(["-w", cwd] if cwd else []), *[f"-e{k}={v}" for k, v in (env or {}).items()], self.cid, *cmd]
        p = subprocess.run(args, capture_output=True, text=True, timeout=timeout)
        return ExecResult(success=p.returncode == 0, returncode=p.returncode, stdout=p.stdout, stderr=p.stderr)

    async def write_file(self, path, contents):
        data = contents.encode() if isinstance(contents, str) else contents
        subprocess.run(["docker", "exec", "-i", self.cid, "sh", "-c", 'mkdir -p "$(dirname "$1")" && cat > "$1"', "sh", path],
                       input=data, check=True)


@pytest.mark.skipif(not docker_ready(), reason=f"needs docker and the {SMOKE_IMAGE} image")
def test_laying_out_the_start_in_a_real_container_keeps_it_running():
    cid = subprocess.run(["docker", "run", "-d", "--init", SMOKE_IMAGE, "tail", "-f", "/dev/null"],
                         capture_output=True, text=True, check=True).stdout.strip()
    try:
        # What an agent might leave: a detached process, and one reparented to init.
        subprocess.run(["docker", "exec", "-d", cid, "sleep", "1001"], check=True)
        subprocess.run(["docker", "exec", cid, "sh", "-c", "(sleep 1002 &)"], check=True)
        sb = DockerSandbox(cid)
        meta = {"score_setup": ["true"], "workdir": "backend", "language": "python"}
        data = tarball({"backend/pyproject.toml": b"[tool.ruff]\n"})
        for _ in range(2):
            assert asyncio.run(task.lay_out_start(sb, meta, data)) is None
        running = subprocess.run(["docker", "inspect", "-f", "{{.State.Running}}", cid], capture_output=True, text=True).stdout
        assert running.strip() == "true"
        laid = subprocess.run(["docker", "exec", cid, "cat", "/score/start/backend/pyproject.toml"], capture_output=True, text=True)
        assert laid.stdout == "[tool.ruff]\n"
        procs = subprocess.run(["docker", "exec", cid, "sh", "-c", "cat /proc/[0-9]*/cmdline | tr '\\0' ' '"],
                               capture_output=True, text=True).stdout
        assert "sleep" not in procs and "tail -f /dev/null" in procs
    finally:
        subprocess.run(["docker", "rm", "-f", cid], capture_output=True)


def test_the_checks_timeout_covers_every_tool_checks_py_runs():
    assert task.CHECKS_TIMEOUT >= checks_module().budget() + 60


def test_a_venv_that_cant_be_rebuilt_fails_the_hidden_tests(monkeypatch):
    class Once(CorpusSandbox):
        builds = 0

        async def exec(self, cmd, **kwargs):
            if cmd[0] == "uv":
                self.builds += 1
                self.venv = self.builds == 1
            return await super().exec(cmd, **kwargs)

    sb = Once([check("ruff-§1", True)])
    s = score(monkeypatch, sb)
    assert s.value == 0 and not s.metadata["tests_passed"] and "uv: offline" in s.metadata["tests_tail"]
    assert not [e for e in sb.log if e[0] == "exec" and e[1][0] == "/score/venv/bin/python"]


def test_the_start_is_read_as_a_plain_tarball_only():
    import gzip

    with pytest.raises(tarfile.TarError):
        task.tree_digest(gzip.compress(START))


def test_a_start_past_the_size_cap_is_refused(monkeypatch):
    monkeypatch.setattr(task, "MAX_START", len(START) - 1)
    with pytest.raises(tarfile.TarError, match="larger than"):
        task.tree_digest(START)


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


def test_only_the_starts_tool_configs_and_conftests_reach_the_checks(tmp_path):
    checks = checks_module()
    start, work = tmp_path / "start" / "backend", tmp_path / "work" / "backend"
    for rel, text in {"pyproject.toml": "start", "tests/unit/conftest.py": "start", "tests/unit/domain/conftest.py": "start"}.items():
        (start / rel).parent.mkdir(parents=True, exist_ok=True)
        (start / rel).write_text(text)
    agent = ["ruff.toml", ".ruff.toml", "app/ruff.toml", "app/.ruff.toml", "app/pyproject.toml", "pytest.ini",
             "tox.ini", "setup.cfg", ".importlinter", "conftest.py", "tests/conftest.py", "app/conftest.py"]
    for rel in [*agent, "pyproject.toml", "tests/unit/conftest.py", "tests/unit/domain/conftest.py", "app/main.py"]:
        (work / rel).parent.mkdir(parents=True, exist_ok=True)
        (work / rel).write_text("agent")
    checks.lay_start_configs(start, work)
    assert not [rel for rel in agent if (work / rel).exists()]
    assert (work / "pyproject.toml").read_text() == "start"
    assert (work / "tests/unit/domain/conftest.py").read_text() == "start"
    assert (work / "app/main.py").read_text() == "agent"


def test_the_tools_run_on_the_starts_config_alone():
    source = (config.task_dir(TASK) / "checks.py").read_text()
    assert source.count('"--config", "pyproject.toml"') == 3  # both ruff runs and lint-imports
    assert '"-c", "pyproject.toml", "--rootdir", "."' in source


def junit(tmp_path, failed=()):
    """A junit report with every house-rule test, `failed` among them failing."""
    cases = "".join(
        f'<testcase name="{t}">' + ('<failure message="boom"/>' if t in failed else "") + "</testcase>"
        for t in checks_module().TESTS
    )
    path = tmp_path / "junit.xml"
    path.write_text(f'<testsuites><testsuite name="pytest">{cases}</testsuite></testsuites>')
    return path


def test_a_clean_report_from_a_failed_run_fails_every_house_rule_test(tmp_path):
    checks = checks_module()
    # pytest exited 1, yet the report shows no failure: the agent's code,
    # which the run imported, could have rewritten it.
    outcomes = checks.house_test_outcomes(1, junit(tmp_path), "1 failed")
    assert set(outcomes) == set(checks.TESTS) and all(outcomes.values())
    assert "doesn't match" in next(iter(outcomes.values()))


def test_a_failing_report_from_a_clean_run_fails_every_house_rule_test(tmp_path):
    checks = checks_module()
    outcomes = checks.house_test_outcomes(0, junit(tmp_path, failed={"test_domain_is_pure"}), "")
    assert all(outcomes.values())


def test_a_missing_report_fails_every_house_rule_test(tmp_path):
    checks = checks_module()
    outcomes = checks.house_test_outcomes(0, tmp_path / "junit.xml", "")
    assert set(outcomes) == set(checks.TESTS) and all(outcomes.values())


def test_a_report_that_matches_its_run_is_read(tmp_path):
    checks = checks_module()
    assert not any(checks.house_test_outcomes(0, junit(tmp_path), "").values())
    failing = checks.house_test_outcomes(1, junit(tmp_path, failed={"test_domain_is_pure"}), "1 failed")
    assert [t for t, why in failing.items() if why] == ["test_domain_is_pure"]


def test_pytest_is_never_imported_from_the_agents_directory():
    source = (config.task_dir(TASK) / "checks.py").read_text()
    assert '"-m", "pytest"' in source and source.count('"-P", "-m", "pytest"') == source.count('"-m", "pytest"')
    assert meta()["score"][:4] == ["{venv}/python", "-P", "-m", "pytest"]


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
