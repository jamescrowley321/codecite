"""Planted solutions for house-rules: the reference with one house rule broken,
or one helper hand-rolled, each of which must fail.

    python3 planted.py <reference tree> <venv bin> <hidden dir> <checks.py> <start tree> [variant ...]

Each variant copies the reference tree, applies its edits (exact text, which
must occur once) and names what must catch it: a check of checks.py by name,
or `hidden` for the hidden tests. Behaviour-preserving variants must leave the
hidden tests passing, so a planted rule break is caught by the rule check, not
by a broken feature. Exits non-zero when a variant isn't caught.
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROLE = "backend/app/services/role.py"
ROUTER = "backend/app/routers/roles.py"
VALUES = "backend/app/domain/access/values.py"
ENV_DOC = "docs/environment-variables.md"
KNOB_READ = 'base = knob("PARTNER_CATALOGUE_URL", "http://partner-catalogue.internal").rstrip("/")'
ROUTE_CALL = 'result = await role_service.import_roles(tenant_id=_parse_uuid(tenant_id, "tenant_id"))'
SLUG_CHECK = """    if fault is None and _ROLE_SLUG.fullmatch(raw) is None:
        fault = NameFault.NOT_A_SLUG
"""

# name -> (edits, what must fail, whether the hidden tests must still pass)
VARIANTS = {
    # §1: configuration read from os.environ instead of app.knobs.
    "os-environ": (
        [(ROLE, KNOB_READ, 'base = os.environ.get("PARTNER_CATALOGUE_URL", "http://partner-catalogue.internal").rstrip("/")'),
         (ROLE, "import uuid\n", "import os\nimport uuid\n")],
        ["ruff-§1", "test_only_knobs_reads_the_environment"],
        True,
    ),
    # §1, reuse: a settings library instead of app.knobs.
    "dotenv": (
        [(ROLE, "import uuid\n", "import uuid\n\nimport dotenv  # noqa: F401\n")],
        ["reuse-app.knobs"],
        None,
    ),
    # §2: a new variable with no row in docs/environment-variables.md.
    "undocumented": (
        [(ENV_DOC, "| `PARTNER_CATALOGUE_URL` | `http://partner-catalogue.internal` | Base URL of the partner role catalogue that `POST /api/roles/import` reads (`services/role.py`). |\n", "")],
        ["test_every_knob_is_documented_with_its_default"],
        True,
    ),
    # §2: the documented default is not the call site's.
    "wrong-default": (
        [(ENV_DOC, "| `PARTNER_CATALOGUE_URL` | `http://partner-catalogue.internal` |", "| `PARTNER_CATALOGUE_URL` | `http://catalogue` |")],
        ["test_every_knob_is_documented_with_its_default"],
        True,
    ),
    # §4: httpx called directly instead of through app.outbound.
    "raw-httpx": (
        [(ROLE, "async with outbound_client(timeout=_CATALOGUE_TIMEOUT) as client:", "async with httpx.AsyncClient(timeout=_CATALOGUE_TIMEOUT) as client:"),
         (ROLE, "import uuid\n", "import uuid\n\nimport httpx\n")],
        ["ruff-§4"],
        True,
    ),
    # §4, reuse: another HTTP library instead of app.outbound.
    "urllib": (
        [(ROLE, "import uuid\n", "import urllib.request  # noqa: F401\nimport uuid\n")],
        ["reuse-app.outbound"],
        True,
    ),
    # §7: a logger from logging.getLogger instead of app.logbook.
    "getlogger": (
        [(ROLE, "logger = logbook(__name__)", "logger = logging.getLogger(__name__)"),
         (ROLE, "import uuid\n", "import logging\nimport uuid\n")],
        ["ruff-§7"],
        True,
    ),
    # §8: an f-string log call.
    "fstring-log": (
        [(ROLE, 'logger.warning("partner catalogue unreachable for tenant %s: %s", tenant_id, type(exc).__name__)',
          'logger.warning(f"partner catalogue unreachable for tenant {tenant_id}: {type(exc).__name__}")')],
        ["ruff-§8"],
        True,
    ),
    # §12: the route turns an Error into an HTTPException.
    "httpexception": (
        [(ROUTER, "    " + ROUTE_CALL + "\n    return result_to_response(result, request)\n",
          "    " + ROUTE_CALL + "\n    if result.is_error():\n        raise HTTPException(status_code=502, detail=result.error.message)\n"
          "    return result_to_response(result, request)\n")],
        ["result-to-response"],
        None,
    ),
    # §15: a tenant-scoped route with no authorization dependency.
    "no-authorization": (
        [(ROUTER, """    tenant_id: str = Depends(get_tenant_id),
    _admin_roles: list[str] = Depends(require_role("owner", "admin")),
    role_service: RoleService = Depends(get_role_service),
):
    \"\"\"Import""", """    tenant_id: str = Depends(get_tenant_id),
    role_service: RoleService = Depends(get_role_service),
):
    \"\"\"Import""")],
        ["test_every_tenant_scoped_route_declares_authorization", "hidden"],
        False,
    ),
    # §16: the tenant read off request.state instead of get_tenant_id.
    "request-state": (
        [(ROUTER, ROUTE_CALL, 'result = await role_service.import_roles(tenant_id=_parse_uuid(request.state.claims["dct"], "tenant_id"))')],
        ["test_router_code_does_not_read_request_state", "tenant-from-dependency"],
        True,
    ),
    # §16: the tenant read from the dct claim, through get_claims.
    "dct-claim": (
        [(ROUTER, ROUTE_CALL, 'claims = get_claims(request)\n    result = await role_service.import_roles(tenant_id=_parse_uuid(claims["dct"], "tenant_id"))'),
         (ROUTER, "from app.dependencies.identity import get_role_service\n", "from app.dependencies.auth import get_claims\nfrom app.dependencies.identity import get_role_service\n")],
        ["tenant-from-dependency"],
        True,
    ),
    # §20: a public service method with no span of its own.
    "no-span": (
        [(ROLE, 'with tracer.start_as_current_span("RoleService.import_roles") as span:', 'with tracer.start_as_current_span("roles.import") as span:')],
        ["test_public_service_methods_open_their_own_span"],
        True,
    ),
    # §23: the domain logs.
    "impure-domain": (
        [(VALUES, "import re\n", "import logging\nimport re\n"),
         (VALUES, SLUG_CHECK, SLUG_CHECK + "    logging.debug(\"role name %s\", raw)\n")],
        ["import-linter-§23", "test_domain_is_pure"],
        True,
    ),
    # §24: the domain raises for an expected failure.
    "domain-raises": (
        [(VALUES, SLUG_CHECK, "    if fault is None and _ROLE_SLUG.fullmatch(raw) is None:\n        raise ValueError(raw)\n")],
        ["test_domain_does_not_raise", "hidden"],
        False,
    ),
    # §26: the router calls the catalogue itself, through app.outbound.
    "router-outbound": (
        [(ROUTER, "from app.services.role import RoleService\n",
          "from app.outbound import outbound_client  # noqa: F401\nfrom app.services.role import RoleService\n")],
        ["import-linter-§26"],
        True,
    ),
    # The decision outside the domain: the slug rule in the service, not in
    # access.role_name, so the domain test fails though the API behaves.
    "decision-in-service": (
        [(VALUES, SLUG_CHECK, ""),
         (ROLE, "            parsed = access.role_name(name)\n            if parsed.is_error():\n                return Error(identity_error(parsed.error))\n            command = access.CreateRole(",
          "            parsed = access.role_name(name)\n            if parsed.is_ok() and not _SLUG.fullmatch(name):\n"
          "                parsed = Error(access.InvalidName(access.NameSubject.ROLE, access.NameFault.NOT_A_SLUG))\n"
          "            if parsed.is_error():\n                return Error(identity_error(parsed.error))\n            command = access.CreateRole("),
         (ROLE, "import uuid\n", "import re\nimport uuid\n"),
         (ROLE, "_CATALOGUE_TIMEOUT = 5.0\n", "_CATALOGUE_TIMEOUT = 5.0\n_SLUG = re.compile(r\"[a-z][a-z0-9]*(?:-[a-z0-9]+)*\")\n")],
        ["hidden"],
        False,
    ),
}


def apply(tree: Path, edits) -> None:
    for rel, old, new in edits:
        path = tree / rel
        text = path.read_text()
        if text.count(old) != 1:
            raise SystemExit(f"planted: {rel} holds {text.count(old)} copies of {old[:60]!r}, not one")
        path.write_text(text.replace(old, new))


def hidden_pass(tree: Path, venv: Path, hidden: Path) -> bool:
    env = {**os.environ, "PYTHONDONTWRITEBYTECODE": "1"}
    p = subprocess.run([str(venv / "python"), "-m", "pytest", "-q", "-p", "no:cacheprovider", str(hidden)],
                       cwd=tree / "backend", capture_output=True, text=True, env=env, timeout=600)
    return p.returncode == 0


def failed_checks(tree: Path, venv: Path, checks: Path, start: Path) -> set[str]:
    env = {**os.environ, "HOUSE_RULES_VENV": str(venv), "HOUSE_RULES_SKIP_UNIT": "1"}
    p = subprocess.run([sys.executable, "-I", str(checks), str(start), str(tree)], capture_output=True, text=True, env=env, timeout=600)
    if p.returncode != 0:
        raise SystemExit(f"planted: checks.py crashed: {p.stderr[-2000:]}")
    return {c["name"] for c in json.loads(p.stdout)["checks"] if not c["passed"] and not c["control"]}


def main(argv) -> int:
    reference, venv, hidden, checks, start = map(Path, argv[:5])
    names = argv[5:] or list(VARIANTS)
    bad = 0
    for name in names:
        edits, expect, still_works = VARIANTS[name]
        tree = Path(tempfile.mkdtemp(prefix=f"house-rules-{name}-"))
        try:
            shutil.copytree(reference, tree, dirs_exist_ok=True, ignore=shutil.ignore_patterns(".venv", "__pycache__", ".git"))
            apply(tree, edits)
            failed = failed_checks(tree, venv, checks, start)
            passed_hidden = hidden_pass(tree, venv, hidden) if ("hidden" in expect or still_works is not None) else None
            if passed_hidden is False:
                failed.add("hidden")
            missing = [e for e in expect if e not in failed]
            works = still_works is None or passed_hidden == still_works
            print(f"{'ok' if not missing and works else 'FAIL'} {name}: failed {sorted(failed)}"
                  + (f"; expected {missing} to fail" if missing else "")
                  + ("" if works else f"; hidden tests {'passed' if passed_hidden else 'failed'}, expected {'pass' if still_works else 'fail'}"))
            bad += bool(missing) or not works
        finally:
            shutil.rmtree(tree, ignore_errors=True)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
