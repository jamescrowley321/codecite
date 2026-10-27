"""Deterministic checks for the backchannel-logout task.

Usage: python3 -I checks.py START_PROJECT FINAL_PROJECT

Reads the agent's code with `ast` (never imports or runs it) and prints one
JSON object: {"checks": [{"name", "passed", "detail"}, ...]}. A solution
passes the task only when the hidden tests and every check pass. No check
asks for identity-model's logout validation: a correct hand-written one
passes too.
"""

import ast
import json
import sys
import tomllib
from pathlib import Path

IGNORED_PARTS = {".venv", "__pycache__", ".pytest_cache", ".ruff_cache", ".mypy_cache"}
IGNORED_FILES = {"uv.lock"}
ENV_KEYS = {"OIDC_DISCOVERY_URL", "OIDC_CLIENT_ID"}
# Event-type identifiers are URIs the spec fixes, not provider addresses.
EVENT_URI_PREFIX = "http://schemas.openid.net/event/"


def py_files(project: Path):
    app = project / "app"
    return sorted(p for p in app.rglob("*.py") if not IGNORED_PARTS & set(p.relative_to(project).parts))


def parse_all(project: Path):
    trees = {}
    for path in py_files(project):
        try:
            trees[path] = ast.parse(path.read_text(encoding="utf-8"), filename=str(path))
        except (SyntaxError, UnicodeDecodeError, ValueError):
            trees[path] = None
    return trees


def is_environ(expr):
    """`os.environ`, or `environ` imported from os."""
    return (isinstance(expr, ast.Attribute) and expr.attr == "environ") or (isinstance(expr, ast.Name) and expr.id == "environ")


def env_reads(tree):
    """String keys read through os.environ[...], os.environ.get(...) or os.getenv(...)."""
    keys = set()
    for node in ast.walk(tree):
        if isinstance(node, ast.Subscript) and is_environ(node.value):
            if isinstance(node.slice, ast.Constant) and isinstance(node.slice.value, str):
                keys.add(node.slice.value)
        elif isinstance(node, ast.Call) and node.args and isinstance(node.args[0], ast.Constant) and isinstance(node.args[0].value, str):
            f = node.func
            if isinstance(f, ast.Attribute) and (f.attr == "getenv" or (f.attr == "get" and is_environ(f.value))):
                keys.add(node.args[0].value)
            elif isinstance(f, ast.Name) and f.id == "getenv":
                keys.add(node.args[0].value)
    return keys


def check_provider(trees):
    """The provider's addresses come from the environment and its discovery
    document: no issuer, discovery or JWKS URL is written into app/."""
    keys, urls = set(), []
    for path, tree in trees.items():
        if tree is None:
            continue
        keys |= env_reads(tree)
        for node in ast.walk(tree):
            if isinstance(node, ast.Constant) and isinstance(node.value, str):
                value = node.value.lower()
                if value.startswith(("http://", "https://")) and not value.startswith(EVENT_URI_PREFIX):
                    urls.append(f"{path.name}:{node.lineno}")
    missing = sorted(ENV_KEYS - keys)
    if missing:
        return False, f"not read from the environment: {', '.join(missing)}"
    if urls:
        return False, f"URL literals in app/: {', '.join(urls[:3])}"
    return True, "settings read from the environment, no provider URL literals"


def dep_names(pyproject: Path):
    data = tomllib.loads(pyproject.read_text(encoding="utf-8"))
    def names(reqs):
        out = set()
        for r in reqs or []:
            n = r.split(";")[0].split("@")[0]
            for sep in "<>=!~[ ":
                n = n.split(sep)[0]
            out.add(n.strip().lower().replace("_", "-"))
        return out
    project = data.get("project", {})
    deps = {"dependencies": names(project.get("dependencies"))}
    for k, v in (project.get("optional-dependencies") or {}).items():
        deps[f"optional:{k}"] = names(v)
    for k, v in (data.get("dependency-groups") or {}).items():
        deps[f"group:{k}"] = names([x for x in v if isinstance(x, str)])
    return deps


def check_deps(start: Path, final: Path):
    try:
        a, b = dep_names(start / "pyproject.toml"), dep_names(final / "pyproject.toml")
    except (OSError, tomllib.TOMLDecodeError) as e:
        return False, f"pyproject.toml unreadable: {e}"
    if a != b:
        added = sorted(set().union(*b.values()) - set().union(*a.values()))
        return False, f"dependencies changed{': added ' + ', '.join(added) if added else ''}"
    return True, "dependency lists unchanged"


def files(project: Path):
    out = {}
    for p in project.rglob("*"):
        rel = p.relative_to(project)
        if p.is_file() and not IGNORED_PARTS & set(rel.parts) and rel.name not in IGNORED_FILES:
            out[rel.as_posix()] = p
    return out


def is_test_file(rel: str) -> bool:
    """A test module the agent wrote for itself. The hidden tests never collect
    it, so it can't change the score; a conftest.py could, so it isn't one."""
    name = rel.rsplit("/", 1)[-1]
    return name.endswith(".py") and (name.startswith("test_") or name.endswith("_test.py"))


def check_scope(start: Path, final: Path):
    a, b = files(start), files(final)
    problems = []
    # Inside app/ anything goes; outside it only docs may change. pyproject.toml
    # is deps-unchanged's (its path is substituted per run).
    def guarded(rel):
        return not rel.startswith("app/") and not rel.endswith(".md")
    for rel, p in a.items():
        if not guarded(rel) or rel == "pyproject.toml":
            continue
        if rel not in b:
            problems.append(f"deleted {rel}")
        elif p.read_bytes() != b[rel].read_bytes():
            problems.append(f"changed {rel}")
    problems += [f"added {rel}" for rel in sorted(b.keys() - a.keys()) if guarded(rel) and not is_test_file(rel)]
    if problems:
        return False, "; ".join(problems[:5])
    return True, "nothing outside app/ added or changed but docs and test modules"


def main():
    start, final = Path(sys.argv[1]), Path(sys.argv[2])
    trees = parse_all(final)
    broken = [p.name for p, t in trees.items() if t is None]
    results = []
    for name, fn in (
        ("provider-from-env", lambda: check_provider(trees)),
        ("deps-unchanged", lambda: check_deps(start, final)),
        ("scope", lambda: check_scope(start, final)),
    ):
        passed, detail = fn()
        if broken and not passed:
            detail += f" (unparseable: {', '.join(broken)})"
        results.append({"name": name, "passed": bool(passed), "detail": detail})
    print(json.dumps({"checks": results}))


if __name__ == "__main__":
    main()
