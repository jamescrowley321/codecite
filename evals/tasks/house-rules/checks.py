"""house-rules' deterministic checks: identity-stack's MUST rules a static
check can decide, the reuse of what identity-stack provides, and control
rules reported apart.

    python3 -I checks.py <start> <project>

<start> is identity-stack as the agent got it, <project> as it left it. Prints
{"checks": [...], "violations": n}: one entry per check, with the rule it
decides (`docs/conventions.md` section, see rules.toml), its level (`MUST`,
or `practice` for best practice that is no identity-stack rule) and whether it
is a control. A run passes only with every non-control check passed: zero
violations of the MUST rules. A control (`"control": true`), a control rule of
rules.toml or a best practice, is what both arms should keep, reported as a
sanity check and never part of the pass.

The enforcement is the start's, not the agent's: the checks run in a copy of
the agent's backend with the start's pyproject.toml (Ruff's bans, the
import-linter contracts), import_contracts.py and the house-rule tests laid
over it, so an agent that loosens a rule's enforcement still breaks the rule;
a start missing any of them is refused. Every other file that configures the
tools (a ruff.toml, a nested pyproject.toml, a pytest.ini, a conftest.py) is
the start's or gone, and the tools are pointed at the start's pyproject.toml. They run the tools of HOUSE_RULES_VENV
(the scorer's environment built from the start; else the project's own
`backend/.venv/bin`), offline. Ruff's bans ignore `# noqa`, so a comment can't
excuse a banned API.
"""

from __future__ import annotations

import ast
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import xml.etree.ElementTree as ET
from pathlib import Path

# The start's files that enforce the rules, laid over the agent's tree.
ENFORCEMENT = [
    "backend/pyproject.toml",
    "backend/import_contracts.py",
    "backend/tests/__init__.py",
    "backend/tests/unit/__init__.py",
    "backend/tests/unit/conftest.py",
    "backend/tests/unit/test_house_rules.py",
    "backend/tests/unit/test_router_authorization.py",
]

# identity-stack's own house-rule tests, by name, and the rule each decides.
TESTS = {
    "test_only_knobs_reads_the_environment": "§1",
    "test_every_knob_is_documented_with_its_default": "§2",
    "test_knob_flag_switches_are_named_for_the_secure_state": "§3",
    "test_router_functions_that_call_descope_handle_upstream_failures": "§4",
    "test_adapter_methods_that_call_descope_handle_upstream_failures": "§4",
    "test_pass_through_routers_reach_descope_only_through_the_adapter": "§5",
    "test_service_methods_return_result": "§10",
    "test_services_and_repositories_do_not_import_fastapi": "§11",
    "test_every_identity_error_is_a_registered_frozen_dataclass": "§13",
    "test_every_tenant_scoped_route_declares_authorization": "§15",
    "test_member_routes_register_has_no_stale_entries": "§15",
    "test_router_code_does_not_read_request_state": "§16",
    "test_queries_live_in_repositories": "§19",
    "test_public_service_methods_open_their_own_span": "§20",
    "test_logout_strategies_open_a_span_per_logout_kind": "§20",
    "test_descope_client_spans_leave_status_to_the_caller": "§20",
    "test_domain_is_pure": "§23",
    "test_domain_does_not_raise": "§24",
    "test_domain_types_are_frozen_dataclasses_or_enums": "§25",
}

# import-linter's contracts, by the name pyproject.toml gives them.
CONTRACTS = {
    "Layers import inward (conventions §22)": "§22",
    "The domain imports no framework and no other layer (conventions §23)": "§23",
    "Bounded contexts do not import each other (conventions §27)": "§27",
    "Routers and services skip no layer (conventions §26)": "§26",
}

# Ruff's banned APIs (TID251), by the name in the message, and the rule each enforces.
BANNED = {"os.environ": "§1", "os.getenv": "§1", "httpx": "§4", "logging.getLogger": "§7"}

# Hand-rolled stand-ins for identity-stack's helpers: a module imported in app/
# instead of app.knobs, app.outbound or app.logbook.
HAND_ROLLED = {
    "§1": ("app.knobs", {"dotenv", "pydantic_settings", "environs", "decouple"}),
    "§4": ("app.outbound", {"requests", "urllib.request", "urllib3", "aiohttp", "http.client", "pycurl"}),
    "§7": ("app.logbook", {"structlog", "loguru"}),
}


# The control rules: best practice rather than a house choice (rules.toml
# kind "control"), reported apart and never failing a run.
CONTROL_RULES = {"§6", "§9", "§11", "§17", "§19", "§22"}


def check(results, name, rule, passed, detail="", control=False):
    """One check's result. A check of a control rule, or one marked control
    (best practice that is no identity-stack rule), is reported, not gated."""
    results.append(
        {
            "name": name,
            "rule": rule,
            "level": "practice" if rule == "control" else "MUST",
            "control": control or rule in CONTROL_RULES,
            "passed": bool(passed),
            "detail": detail[:1500],
        }
    )


# Each tool's timeout, in seconds. The scorer's own timeout on checks.py
# (task.py CHECKS_TIMEOUT) covers their sum, budget(), so checks.py always
# finishes and prints its results: a slow control can't lose the gated ones.
TIMEOUTS = {"ruff": 120, "lint-imports": 120, "house-tests": 300, "control-ruff": 60, "unit-tests": 180}
CONTROL_RUFF_RUNS = 4


def budget() -> int:
    """The longest checks.py's tools can take, all timing out."""
    return sum(TIMEOUTS.values()) + (CONTROL_RUFF_RUNS - 1) * TIMEOUTS["control-ruff"]


def house_test_outcomes(code: int, junit: Path, out: str) -> dict[str, str | None]:
    """Each house-rule test's failure, or None when it passed, from pytest's
    junit report. The run imported the agent's code, which could rewrite the
    report, so a report that doesn't agree with pytest's exit status (failures
    in a clean run, none in a failed one), or is missing, fails every test."""
    cases = {}
    try:
        for case in ET.parse(junit).iter("testcase"):
            failure = case.find("failure")
            if failure is None:
                failure = case.find("error")
            cases[case.get("name")] = None if failure is None else (failure.get("message") or "failed")[:600]
    except (OSError, ET.ParseError):
        return {test: f"no report: {out[-800:]}" for test in TESTS}
    if (code == 0) != (not any(cases.values())):
        return {test: f"the report doesn't match pytest's exit {code}: {out[-800:]}" for test in TESTS}
    return {test: cases[test] if test in cases else f"did not run: {out[-800:]}" for test in TESTS}


def run(cmd, cwd, timeout=300):
    env = {**os.environ, "PYTHONDONTWRITEBYTECODE": "1", "UV_OFFLINE": "1"}
    try:
        p = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True, timeout=timeout, env=env)
        return p.returncode, p.stdout, p.stderr
    except subprocess.TimeoutExpired:
        return 124, "", f"timed out after {timeout}s"
    except OSError as e:
        return 127, "", str(e)


def app_files(root: Path) -> dict[str, str]:
    app = root / "backend" / "app"
    return {p.relative_to(root).as_posix(): p.read_text(errors="replace") for p in sorted(app.rglob("*.py"))}


def parse(source: str) -> ast.Module:
    """The module's tree; an empty one when it doesn't parse (pyflakes reports that)."""
    try:
        return ast.parse(source)
    except SyntaxError:
        return ast.Module(body=[], type_ignores=[])


def functions(source: str) -> dict[str, ast.AST]:
    """Module-level functions and methods, by qualified name."""
    tree = parse(source)
    found = {}
    for node in tree.body:
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
            found[node.name] = node
        elif isinstance(node, ast.ClassDef):
            for item in node.body:
                if isinstance(item, (ast.FunctionDef, ast.AsyncFunctionDef)):
                    found[f"{node.name}.{item.name}"] = item
    return found


def imported(source: str) -> set[str]:
    tree = parse(source)
    names = set()
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            names.update(a.name for a in node.names)
        elif isinstance(node, ast.ImportFrom) and node.module and not node.level:
            names.add(node.module)
            names.update(f"{node.module}.{a.name}" for a in node.names)
    return names


def called(node: ast.AST, names: set[str]) -> list[str]:
    hits = []
    for n in ast.walk(node):
        if isinstance(n, ast.Call):
            f = n.func
            name = f.id if isinstance(f, ast.Name) else f.attr if isinstance(f, ast.Attribute) else None
            if name in names:
                hits.append(f"{name}() at line {n.lineno}")
    return hits


def strings(node: ast.AST, wanted) -> list[str]:
    return [
        f"{n.value!r} at line {n.lineno}"
        for n in ast.walk(node)
        if isinstance(n, ast.Constant) and isinstance(n.value, str) and wanted(n.value)
    ]


def static_checks(results, start: Path, project: Path, changed: dict[str, str], before: dict[str, str]):
    # Reuse: nothing identity-stack provides is hand-rolled in a changed module.
    for rule, (helper, modules) in HAND_ROLLED.items():
        hits = [
            f"{path} imports {m}"
            for path, text in changed.items()
            for m in sorted(imported(text))
            if any(m == bad or m.startswith(bad + ".") for bad in modules)
        ]
        check(results, f"reuse-{helper}", rule, not hits, "; ".join(hits))

    # §12: a new route handler answers through result_to_response, never an
    # HTTPException or a hand-built response; problem details are built only in
    # app/errors/problem_detail.py.
    # §16: a new route handler takes its tenant from get_tenant_id, never the dct claim.
    hand_built, dct = [], []
    for path, text in changed.items():
        if not path.startswith("backend/app/routers/"):
            continue
        old = functions(before.get(path, ""))
        for qual, node in functions(text).items():
            if qual in old:
                continue
            hand_built += [f"{path}::{qual} calls {h}" for h in called(node, {"HTTPException", "JSONResponse", "Response"})]
            dct += [f"{path}::{qual} reads {h}" for h in strings(node, lambda s: s == "dct")]
    for path, text in changed.items():
        if path == "backend/app/errors/problem_detail.py":
            continue
        new = strings(parse(text), lambda s: "problem+json" in s)
        old = strings(parse(before.get(path, "")), lambda s: "problem+json" in s)
        if len(new) > len(old):
            hand_built.append(f"{path} builds a problem+json body: {', '.join(new)}")
    check(results, "result-to-response", "§12", not hand_built, "; ".join(hand_built))
    check(results, "tenant-from-dependency", "§16", not dct, "; ".join(dct))


# Files that configure Ruff, pytest or import-linter wherever they sit: the
# agent's copy of any of them could outrank or extend the start's.
CONFIGS = {"pyproject.toml", "ruff.toml", ".ruff.toml", "pytest.ini", ".pytest.ini", "tox.ini", "setup.cfg",
           ".importlinter", "conftest.py"}


def lay_start_configs(start_backend: Path, work_backend: Path) -> None:
    """In the copy of the agent's backend, make every tool config and
    conftest the start's: the start's version where the start has one at that
    path, else none."""
    for path in sorted(work_backend.rglob("*")):
        if path.name in CONFIGS and path.is_file():
            ours = start_backend / path.relative_to(work_backend)
            if ours.is_file():
                shutil.copy2(ours, path)
            else:
                path.unlink()


def relative_name(filename: str, backend: Path) -> str:
    """A file Ruff reported, named from the backend. Ruff reports the resolved
    path, which a symlinked temp directory (macOS's /var) keeps from matching."""
    name, root = Path(filename).resolve(), backend.resolve()
    return name.relative_to(root).as_posix() if name.is_relative_to(root) else str(name)


def tool_checks(results, start: Path, project: Path, venv: Path, changed: dict[str, str]):
    work = Path(tempfile.mkdtemp(prefix="house-rules-checks-"))
    try:
        shutil.copytree(project / "backend", work / "backend", ignore=shutil.ignore_patterns(".venv", "__pycache__", ".*_cache"))
        if (project / "docs").is_dir():
            shutil.copytree(project / "docs", work / "docs")
        for rel in ENFORCEMENT:
            dst = work / rel
            dst.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(start / rel, dst)
        backend = work / "backend"
        lay_start_configs(start / "backend", backend)

        # Ruff's house bans (§1, §4, §7) and lazy log calls (§8), over all of
        # app/; a `# noqa` doesn't excuse them.
        code, out, err = run([str(venv / "ruff"), "check", "--no-cache", "--select", "TID251,G004,G010", "--ignore-noqa",
                              "--config", "pyproject.toml", "--output-format", "json", "app"], backend, TIMEOUTS["ruff"])
        try:
            found = json.loads(out or "[]")
        except ValueError:
            found = None
        if found is None or code not in (0, 1):
            for rule in ("§1", "§4", "§7", "§8"):
                check(results, f"ruff-{rule}", rule, False, f"ruff failed: {err or out}")
        else:
            by_rule = {"§1": [], "§4": [], "§7": [], "§8": []}
            for v in found:
                if v.get("code") not in ("TID251", "G004", "G010"):
                    continue  # a syntax error: control-pyflakes reports it
                where = f"{relative_name(v['filename'], backend)}:{v['location']['row']} {v['code']} {v['message']}"
                if v["code"] == "TID251":
                    rule = next((r for api, r in BANNED.items() if f"`{api}`" in v["message"]), "§1")
                else:
                    rule = "§8"
                by_rule[rule].append(where)
            for rule, hits in by_rule.items():
                check(results, f"ruff-{rule}", rule, not hits, "; ".join(hits))

        # import-linter's contracts (§22, §23, §26, §27).
        code, out, err = run([str(venv / "lint-imports"), "--no-cache", "--config", "pyproject.toml"], backend, TIMEOUTS["lint-imports"])
        for contract, rule in CONTRACTS.items():
            m = re.search(re.escape(contract) + r"\s+(KEPT|BROKEN)", out)
            check(results, f"import-linter-{rule}", rule, bool(m and m.group(1) == "KEPT"),
                  "" if m and m.group(1) == "KEPT" else (out + err)[-1500:])

        # identity-stack's own house-rule tests, as the start had them. `-P`
        # keeps the backend directory off sys.path until pytest is imported,
        # so an agent's pytest.py can't stand in for pytest.
        junit = work / "junit.xml"
        code, out, err = run(
            [str(venv / "python"), "-P", "-m", "pytest", "-q", "-p", "no:cacheprovider", "-c", "pyproject.toml", "--rootdir", ".",
             "-o", f"pythonpath={backend}", f"--junitxml={junit}",
             "tests/unit/test_house_rules.py", "tests/unit/test_router_authorization.py"],
            backend, TIMEOUTS["house-tests"],
        )
        for test, why in house_test_outcomes(code, junit, out + err).items():
            check(results, test, TESTS[test], why is None, why or "")

        # Controls: best practice either arm should keep, reported apart.
        files = [str(Path(p).relative_to("backend")) for p in changed if (backend / Path(p).relative_to("backend")).exists()]
        for name, select in (("control-pyflakes", "F"), ("control-bare-except", "E722"), ("control-print", "T201"),
                             ("control-hardcoded-secret", "S105,S106,S107")):
            if not files:
                check(results, name, "control", True, control=True)
                continue
            code, out, err = run([str(venv / "ruff"), "check", "--no-cache", "--select", select, "--config", "pyproject.toml",
                                  "--output-format", "concise", *files], backend, TIMEOUTS["control-ruff"])
            check(results, name, "control", code == 0, (out + err)[-800:] if code else "", control=True)
    finally:
        shutil.rmtree(work, ignore_errors=True)

    # The agent's own unit tests, in its own tree (the repository tests need
    # Postgres). planted.sh skips them (HOUSE_RULES_SKIP_UNIT=1) for speed.
    if os.environ.get("HOUSE_RULES_SKIP_UNIT") == "1":
        return
    code, out, err = run(
        [str(venv / "python"), "-P", "-m", "pytest", "-q", "-x", "-p", "no:cacheprovider", "-o", f"pythonpath={project / 'backend'}",
         "tests/unit", "--ignore=tests/unit/repositories"],
        project / "backend", TIMEOUTS["unit-tests"],
    )
    check(results, "control-unit-tests", "control", code == 0, "" if code == 0 else (out + err)[-1200:], control=True)


def main(start: Path, project: Path) -> dict:
    missing = [rel for rel in ENFORCEMENT if not (start / rel).is_file()]
    if missing:
        sys.exit(f"checks: the start lacks {', '.join(missing)}, so the rules can't be enforced from it")
    results = []
    venv = Path(os.environ.get("HOUSE_RULES_VENV") or project / "backend" / ".venv" / "bin")
    before, after = app_files(start), app_files(project)
    changed = {p: t for p, t in after.items() if before.get(p) != t}
    static_checks(results, start, project, changed, before)
    tool_checks(results, start, project, venv, changed)
    return {"checks": results, "violations": sum(1 for c in results if not c["passed"] and not c["control"])}


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit("usage: checks.py <start> <project>")
    print(json.dumps(main(Path(sys.argv[1]), Path(sys.argv[2])), indent=2))
