"""Fixtures for the hidden tests: a TestClient over the agent's app and real
tokens from the eval IdP (evals/fixtures/node-oidc)."""

import os
import sys

import httpx
import pytest

IDP = os.environ.get("TEST_IDP", "http://127.0.0.1:9011").rstrip("/")
AUDIENCE = "urn:codecite-eval:api"
SECRET = "eval-secret"
# Requested so every token carries a `scope` claim: a correct, hardened
# solution (require_access_token_marker=True) must accept the valid ones.
SCOPE = "api:read"

# The score command runs from the project root; make `app` importable from it.
sys.path.insert(0, os.getcwd())

# The app reads these from the environment; set them before importing it.
os.environ["OIDC_DISCOVERY_URL"] = f"{IDP}/.well-known/openid-configuration"
os.environ["OIDC_AUDIENCE"] = AUDIENCE

from fastapi.testclient import TestClient  # noqa: E402

from app.main import app  # noqa: E402


@pytest.fixture(scope="session")
def client():
    # The `with` block runs the app's startup and shutdown.
    with TestClient(app) as c:
        yield c


def _token_endpoint() -> str:
    disco = httpx.get(f"{IDP}/.well-known/openid-configuration", timeout=10)
    disco.raise_for_status()
    return disco.json()["token_endpoint"]


@pytest.fixture(scope="session")
def token():
    endpoint = _token_endpoint()

    def get(client: str = "eval-client", resource: str = AUDIENCE) -> str:
        resp = httpx.post(
            endpoint,
            data={
                "grant_type": "client_credentials",
                "client_id": client,
                "client_secret": SECRET,
                "resource": resource,
                "scope": SCOPE,
            },
            timeout=10,
        )
        resp.raise_for_status()
        return resp.json()["access_token"]

    return get
