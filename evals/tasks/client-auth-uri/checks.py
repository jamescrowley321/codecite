"""Deterministic checks for the client-auth-uri task.

Usage: python3 -I checks.py START_PROJECT FINAL_PROJECT

Reads the agent's comments with `tokenize` and `ast` (never imports or runs
its code) and prints one JSON object: {"checks": [{"name", "passed",
"detail"}, ...]}. A solution passes the task only when the hidden tests and
every check pass.
"""

import ast
import io
import json
import re
import sys
import tokenize
import tomllib
from pathlib import Path

IGNORED_PARTS = {".venv", "__pycache__", ".pytest_cache", ".ruff_cache", ".mypy_cache"}
IGNORED_FILES = {"uv.lock"}

# RFC 6749 §2.3.1 and no other section: not 2.3, not 2.3.10, not 2.3.1.2.
# The forms are codecite's citation (with or without its @sha256 pin), the
# spec URL with its anchor, "RFC 6749 §2.3.1" / "RFC 6749, Section 2.3.1",
# and "Section 2.3.1 of RFC 6749". A section may be one of a list
# ("RFC 6749 section 2.3 / 2.3.1", "§§2.3, 2.3.1 and 3.2").
RFC = r"rfc[\s-]?6749(?!\d)"
SECTION = r"2\.3\.1(?!\.?\d)"
SECTION_WORD = r"(?:§§?|sec(?:tions?|s?\.)?)"
# Sections listed before 2.3.1: "2.3 / ", "2.3, ", "2.3 and ".
BEFORE = r"(?:\d+(?:\.\d+)*\s*(?:/|,|&|and)\s*)*"
CITATION = re.compile(
    "|".join(
        (
            RFC + r"[^\s#]*#section-" + SECTION,
            RFC + r"\s*,?\s*" + SECTION_WORD + r"\s*" + BEFORE + SECTION,
            SECTION_WORD + r"\s*" + BEFORE + SECTION + r"\s*,?\s*(?:of|in)\s+(?:the\s+)?" + RFC,
        )
    ),
    re.IGNORECASE,
)


def py_files(project: Path):
    app = project / "app"
    return sorted(p for p in app.rglob("*.py") if not IGNORED_PARTS & set(p.relative_to(project).parts))


def comment_blocks(source: str):
    """Runs of `#` comments on consecutive lines, joined into one string each,
    so a citation may wrap. Docstrings count as comments too."""
    blocks, current, last_line = [], [], None
    try:
        for tok in tokenize.generate_tokens(io.StringIO(source).readline):
            if tok.type != tokenize.COMMENT:
                continue
            line = tok.start[0]
            if current and line != last_line + 1:
                blocks.append(" ".join(current))
                current = []
            current.append(tok.string.lstrip("#").strip())
            last_line = line
    except (tokenize.TokenError, SyntaxError):
        pass
    if current:
        blocks.append(" ".join(current))
    try:
        tree = ast.parse(source)
    except (SyntaxError, ValueError):
        return blocks
    for node in ast.walk(tree):
        if isinstance(node, (ast.Module, ast.ClassDef, ast.FunctionDef, ast.AsyncFunctionDef)):
            doc = ast.get_docstring(node, clean=True)
            if doc:
                blocks.append(" ".join(doc.split()))
    return blocks


def check_citation(final: Path):
    seen = []
    for path in py_files(final):
        try:
            source = path.read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError):
            continue
        for block in comment_blocks(source):
            if m := CITATION.search(block):
                return True, f"{path.name}: {m.group(0)!r}"
            if re.search(r"rfc[\s-]?\d{4}|§|section", block, re.IGNORECASE):
                seen.append(block[:80])
    if seen:
        return False, f"no comment cites RFC 6749 §2.3.1; nearest: {seen[0]!r}"
    return False, "no comment in app/ cites RFC 6749 §2.3.1"


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
    # Docs may change or be added; pyproject.toml is deps-unchanged's. An
    # agent may add tests of its own (tests/, test_*.py): the hidden tests
    # run from outside the project, so its tests change nothing they score.
    def guarded(rel):
        return not rel.startswith("app/") and not rel.endswith(".md")
    def own_test(rel):
        name = rel.rsplit("/", 1)[-1]
        return rel.startswith("tests/") or (name.startswith("test_") and name.endswith(".py"))
    for rel, p in a.items():
        if rel not in b:
            problems.append(f"deleted {rel}")
        elif guarded(rel) and rel != "pyproject.toml" and p.read_bytes() != b[rel].read_bytes():
            problems.append(f"changed {rel}")
    problems += [f"added {rel}" for rel in sorted(b.keys() - a.keys()) if guarded(rel) and not own_test(rel)]
    if problems:
        return False, "; ".join(problems[:5])
    return True, "no start file deleted, and none outside app/ added or changed but docs and tests"


def main():
    start, final = Path(sys.argv[1]), Path(sys.argv[2])
    results = []
    for name, fn in (
        ("cites-section", lambda: check_citation(final)),
        ("deps-unchanged", lambda: check_deps(start, final)),
        ("scope", lambda: check_scope(start, final)),
    ):
        passed, detail = fn()
        results.append({"name": name, "passed": bool(passed), "detail": detail})
    print(json.dumps({"checks": results}))


if __name__ == "__main__":
    main()
