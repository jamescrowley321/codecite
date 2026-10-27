"""Fixtures for the hidden tests: a stub provider on loopback serving discovery
and JWKS, a test key that signs logout tokens, and a TestClient over the
agent's app. The task needs no real IdP."""

import json
import os
import sys
import threading
import time
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

import jwt
import pytest
from cryptography.hazmat.primitives.asymmetric import rsa

CLIENT_ID = "notes-rp"
KID = "test-key"
EVENT = "http://schemas.openid.net/event/backchannel-logout"

KEY = rsa.generate_private_key(public_exponent=65537, key_size=2048)
# Signs tokens the app must reject: same kid, wrong key.
OTHER_KEY = rsa.generate_private_key(public_exponent=65537, key_size=2048)


def _public_jwk(key) -> dict:
    jwk = json.loads(jwt.algorithms.RSAAlgorithm.to_jwk(key.public_key()))
    jwk.pop("key_ops", None)  # PyJWT adds it; a JWK may not carry both it and `use`
    return {**jwk, "kid": KID, "use": "sig", "alg": "RS256"}


class _Provider(BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path == "/.well-known/openid-configuration":
            body = {
                "issuer": ISSUER,
                "authorization_endpoint": f"{ISSUER}/authorize",
                "token_endpoint": f"{ISSUER}/token",
                "jwks_uri": f"{ISSUER}/jwks",
                "end_session_endpoint": f"{ISSUER}/logout",
                "response_types_supported": ["code"],
                "subject_types_supported": ["public"],
                "id_token_signing_alg_values_supported": ["RS256"],
                "backchannel_logout_supported": True,
                "backchannel_logout_session_supported": True,
            }
        elif self.path == "/jwks":
            body = {"keys": [_public_jwk(KEY)]}
        else:
            self.send_error(404)
            return
        data = json.dumps(body).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, *args):
        pass


_server = ThreadingHTTPServer(("127.0.0.1", 0), _Provider)
threading.Thread(target=_server.serve_forever, daemon=True).start()
ISSUER = f"http://127.0.0.1:{_server.server_address[1]}"

# The score command runs from the project root; make `app` importable from it.
sys.path.insert(0, os.getcwd())

# The app reads these from the environment; set them before importing it.
os.environ["OIDC_DISCOVERY_URL"] = f"{ISSUER}/.well-known/openid-configuration"
os.environ["OIDC_CLIENT_ID"] = CLIENT_ID

from fastapi.testclient import TestClient  # noqa: E402

from app import sessions  # noqa: E402
from app.main import app  # noqa: E402

_DROP = object()


def logout_token(key=KEY, **overrides) -> str:
    """A valid logout token; pass a claim to change it, or `claim=DROP` to omit it."""
    now = int(time.time())
    claims = {
        "iss": ISSUER,
        "aud": CLIENT_ID,
        "iat": now,
        "exp": now + 120,
        "jti": uuid.uuid4().hex,
        "sub": "user-1",
        "sid": "sid-1",
        "events": {EVENT: {}},
    }
    claims.update(overrides)
    claims = {k: v for k, v in claims.items() if v is not _DROP}
    return jwt.encode(claims, key, algorithm="RS256", headers={"kid": KID})


@pytest.fixture(scope="session")
def client():
    with TestClient(app) as c:
        yield c


@pytest.fixture
def session():
    """A fresh signed-in session: (sub, sid)."""
    sub, sid = f"user-{uuid.uuid4().hex[:8]}", f"sid-{uuid.uuid4().hex[:8]}"
    sessions.start(sub, sid)
    return sub, sid
