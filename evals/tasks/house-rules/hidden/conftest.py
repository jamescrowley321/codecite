"""The hidden tests' harness: the app over a fresh SQLite file per test, a stub
partner catalogue, and tokens whose claims the test chooses.

Everything the app reads from its environment is set here, before any test
imports `app`, so a value the agent fixed at import time sees it too. The tests
run from identity-stack's `backend/` (`python -m pytest`), which puts `app` on
the path; they import nothing the agent was asked to write except through the
HTTP API and `app.domain.access.role_name`, which the starting tree already has.
"""

from __future__ import annotations

import asyncio
import json
import os
import tempfile
import threading
import time
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from unittest.mock import AsyncMock, patch

import pytest

for proxy in ("HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "http_proxy", "https_proxy", "all_proxy"):
    os.environ.pop(proxy, None)
os.environ["NO_PROXY"] = os.environ["no_proxy"] = "127.0.0.1,localhost"

TENANT = "51c5957b-684a-453f-8ab1-8f239999c4d8"
OTHER_TENANT = "0d1f4f9e-3c55-4b55-9a41-6f0d7c1e2a10"
# What the stub catalogue answers per tenant: (status, body) or a behaviour.
CATALOGUE: dict[str, object] = {}
SEEN: list[tuple[str, str]] = []


class _Catalogue(BaseHTTPRequestHandler):
    def do_GET(self):  # noqa: N802
        SEEN.append(("GET", self.path))
        parts = self.path.strip("/").split("/")
        answer = CATALOGUE.get(parts[1]) if len(parts) == 3 and parts[0] == "tenants" and parts[2] == "roles" else None
        if answer == "hang":
            time.sleep(12)
            answer = (200, {"roles": []})
        if answer == "drop":
            self.close_connection = True
            self.connection.shutdown(2)
            return
        status, body = answer if isinstance(answer, tuple) else (404, {"error": "unknown tenant"})
        data = json.dumps(body).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, *args):
        pass


_server = ThreadingHTTPServer(("127.0.0.1", 0), _Catalogue)
_server.daemon_threads = True
threading.Thread(target=_server.serve_forever, daemon=True).start()

_DB_DIR = Path(tempfile.mkdtemp(prefix="house-rules-"))
os.environ.update(
    {
        "PARTNER_CATALOGUE_URL": f"http://127.0.0.1:{_server.server_port}",
        "DATABASE_URL": f"sqlite+aiosqlite:///{_DB_DIR / 'boot.db'}",
        "DESCOPE_PROJECT_ID": "test-project-id",
        "DESCOPE_MANAGEMENT_KEY": "test-management-key",
        "RATE_LIMIT_AUTH": "1000/minute",
        "RATE_LIMIT_DEFAULT": "1000/minute",
    }
)
os.environ.pop("REDIS_URL", None)


def claims(roles: list[str], tenant: str = TENANT) -> dict:
    return {"sub": "user123", "dct": tenant, "tenants": {tenant: {"roles": roles, "permissions": []}}}


@pytest.fixture
def db(monkeypatch):
    """A fresh SQLite database the app's engine points at; yields a sync engine on it."""
    import sqlalchemy as sa
    from sqlmodel import SQLModel

    import app.models.document  # noqa: F401  (registers the tables)
    import app.models.identity  # noqa: F401
    import app.models.tenant  # noqa: F401
    from app.models.database import reset_engine

    path = _DB_DIR / f"{uuid.uuid4().hex}.db"
    monkeypatch.setenv("DATABASE_URL", f"sqlite+aiosqlite:///{path}")
    engine = sa.create_engine(f"sqlite:///{path}")
    SQLModel.metadata.create_all(engine)
    with engine.begin() as conn:
        # SQLite ignores postgresql_where, so the global-name index would span
        # every tenant; make it partial, as the migration does on Postgres.
        conn.execute(sa.text("DROP INDEX IF EXISTS ix_roles_name_global"))
        conn.execute(sa.text("CREATE UNIQUE INDEX ix_roles_name_global ON roles (name) WHERE tenant_id IS NULL"))
    reset_engine()
    yield engine
    reset_engine()
    engine.dispose()


@pytest.fixture
def api(db):
    """call(method, path, roles=..., tenant=..., json=..., auth=True) -> (status, body, seconds)."""
    from httpx import ASGITransport, AsyncClient

    from app.main import app
    from app.middleware.rate_limit import limiter
    from app.models.database import get_engine
    from app.services.cache_invalidation import CacheInvalidationPublisher

    app.state.descope_client = AsyncMock()
    app.state.cache_publisher = CacheInvalidationPublisher()
    app.state.redis_client = None
    limiter.reset()
    SEEN.clear()

    def call(method, path, *, roles=("admin",), tenant=TENANT, json=None, auth=True):
        async def go():
            headers = {"Authorization": "Bearer valid.token"} if auth else {}
            try:
                with patch("app.middleware.auth.validate_token", new_callable=AsyncMock) as validate:
                    validate.return_value = claims(list(roles), tenant)
                    async with AsyncClient(transport=ASGITransport(app=app), base_url="http://test") as client:
                        started = time.monotonic()
                        response = await client.request(method, path, headers=headers, json=json)
                        return response, time.monotonic() - started
            finally:
                await get_engine().dispose()

        response, seconds = asyncio.run(go())
        try:
            body = response.json()
        except ValueError:
            body = response.text
        return response.status_code, body, seconds

    yield call
    CATALOGUE.clear()
