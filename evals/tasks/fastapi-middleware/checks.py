"""Deterministic checks for the fastapi-middleware task.

Usage: python3 -I checks.py START_PROJECT FINAL_PROJECT

Reads the agent's code with `ast` (never imports or runs it) and prints one
JSON object: {"checks": [{"name", "passed", "detail"}, ...]}. A solution
passes the task only when the hidden tests and every check pass.
"""

import ast
import json
import sys
import tomllib
from pathlib import Path

PKG = "fastapi_identity_model"
IGNORED_PARTS = {".venv", "__pycache__", ".pytest_cache", ".ruff_cache", ".mypy_cache"}
IGNORED_FILES = {"uv.lock"}


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


def library_names(tree, wanted):
    """Local names bound to `wanted` imported from the package (or a submodule),
    and local names bound to the package module itself."""
    names, modules = set(), set()
    for node in ast.walk(tree):
        if isinstance(node, ast.ImportFrom) and node.module and (node.module == PKG or node.module.startswith(PKG + ".")):
            for alias in node.names:
                if alias.name == wanted:
                    names.add(alias.asname or alias.name)
        elif isinstance(node, ast.Import):
            for alias in node.names:
                if alias.name == PKG or alias.name.startswith(PKG + "."):
                    modules.add(alias.asname or alias.name.split(".")[0])
    return names, modules


def refers_to(expr, names, modules, wanted):
    """Whether `expr` is a reference to the library's `wanted`."""
    if isinstance(expr, ast.Name):
        return expr.id in names
    if isinstance(expr, ast.Attribute) and expr.attr == wanted:
        root = expr.value
        while isinstance(root, ast.Attribute):
            root = root.value
        return isinstance(root, ast.Name) and root.id in modules
    return False


def check_middleware(trees):
    for path, tree in trees.items():
        if tree is None:
            continue
        names, modules = library_names(tree, "TokenValidationMiddleware")
        if not names and not modules:
            continue
        for node in ast.walk(tree):
            if not isinstance(node, ast.Call) or not node.args:
                continue
            func = node.func
            fname = func.attr if isinstance(func, ast.Attribute) else func.id if isinstance(func, ast.Name) else None
            if fname in ("add_middleware", "Middleware") and refers_to(node.args[0], names, modules, "TokenValidationMiddleware"):
                return True, f"{path.name}: {fname}(TokenValidationMiddleware, ...)"
    return False, "no add_middleware(...) or Middleware(...) with identity-model's TokenValidationMiddleware"


def route_path(decorator):
    if isinstance(decorator, ast.Call) and isinstance(decorator.func, ast.Attribute) and decorator.func.attr in ("get", "api_route"):
        if decorator.args and isinstance(decorator.args[0], ast.Constant):
            return decorator.args[0].value
    return None


def check_current_user(trees):
    found_me = False
    for path, tree in trees.items():
        if tree is None:
            continue
        names, modules = library_names(tree, "CurrentUser")
        for node in ast.walk(tree):
            if not isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
                continue
            if not any(route_path(d) == "/me" for d in node.decorator_list):
                continue
            found_me = True
            args = node.args.args + node.args.kwonlyargs
            for a in args:
                if a.annotation is not None and any(refers_to(n, names, modules, "CurrentUser") for n in ast.walk(a.annotation)):
                    return True, f"{path.name}: {node.name}({a.arg}: CurrentUser)"
    if not found_me:
        return False, "no /me route found"
    return False, "the /me handler takes no parameter annotated with identity-model's CurrentUser"


def env_reads(tree):
    """String keys read through os.environ[...], os.environ.get(...) or os.getenv(...)."""
    keys = set()
    for node in ast.walk(tree):
        if isinstance(node, ast.Subscript) and isinstance(node.value, ast.Attribute) and node.value.attr == "environ":
            if isinstance(node.slice, ast.Constant) and isinstance(node.slice.value, str):
                keys.add(node.slice.value)
        elif isinstance(node, ast.Call) and node.args and isinstance(node.args[0], ast.Constant) and isinstance(node.args[0].value, str):
            f = node.func
            if isinstance(f, ast.Attribute) and (f.attr == "getenv" or (f.attr == "get" and isinstance(f.value, ast.Attribute) and f.value.attr == "environ")):
                keys.add(node.args[0].value)
            elif isinstance(f, ast.Name) and f.id == "getenv":
                keys.add(node.args[0].value)
    return keys


def settings_from_env(tree):
    """Keys read by identity-model's OIDCSettings.from_env() (default prefix OIDC_)."""
    names, modules = library_names(tree, "OIDCSettings")
    for node in ast.walk(tree):
        if isinstance(node, ast.Call) and isinstance(node.func, ast.Attribute) and node.func.attr == "from_env":
            if refers_to(node.func.value, names, modules, "OIDCSettings"):
                prefix = "OIDC_"
                arg = node.args[0] if node.args else next((k.value for k in node.keywords if k.arg == "prefix"), None)
                if arg is not None:
                    prefix = arg.value if isinstance(arg, ast.Constant) and isinstance(arg.value, str) else None
                if prefix is not None:
                    return {prefix + "DISCOVERY_URL", prefix + "AUDIENCE"}
    return set()


def check_env(trees):
    keys, urls = set(), []
    for path, tree in trees.items():
        if tree is None:
            continue
        keys |= env_reads(tree) | settings_from_env(tree)
        for node in ast.walk(tree):
            if isinstance(node, ast.Constant) and isinstance(node.value, str) and node.value.lower().startswith(("http://", "https://")):
                urls.append(f"{path.name}:{node.lineno}")
    missing = sorted({"OIDC_DISCOVERY_URL", "OIDC_AUDIENCE"} - keys)
    if missing:
        return False, f"not read from the environment: {', '.join(missing)}"
    if urls:
        return False, f"URL literals in app/: {', '.join(urls[:3])}"
    return True, "both settings read from the environment, no URL literals"


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


def check_scope(start: Path, final: Path):
    a, b = files(start), files(final)
    problems = []
    for rel, p in a.items():
        if rel not in b:
            problems.append(f"deleted {rel}")
        # Docs may change (agents document the new settings); pyproject.toml is
        # deps-unchanged's (its path is substituted per run).
        elif not rel.startswith("app/") and not rel.endswith(".md") and rel != "pyproject.toml" and p.read_bytes() != b[rel].read_bytes():
            problems.append(f"changed {rel}")
    if problems:
        return False, "; ".join(problems[:5])
    return True, "no start file deleted, and none outside app/ changed but docs"


def main():
    start, final = Path(sys.argv[1]), Path(sys.argv[2])
    trees = parse_all(final)
    broken = [p.name for p, t in trees.items() if t is None]
    results = []
    for name, fn in (
        ("uses-middleware", lambda: check_middleware(trees)),
        ("uses-current-user", lambda: check_current_user(trees)),
        ("reads-env", lambda: check_env(trees)),
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
