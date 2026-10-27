"""Fixtures for the hidden tests: a TestClient over the agent's app. The
token endpoint is the app's own, so no IdP is needed."""

import os
import sys

import pytest

# The score command runs from the project root; make `app` importable from it.
sys.path.insert(0, os.getcwd())

from fastapi.testclient import TestClient  # noqa: E402

from app.main import app  # noqa: E402


@pytest.fixture(scope="session")
def client():
    # The `with` block runs the app's startup and shutdown.
    with TestClient(app) as c:
        yield c
