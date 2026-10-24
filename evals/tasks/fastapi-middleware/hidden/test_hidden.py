"""Hidden tests for the fastapi-middleware task. The agent never sees these."""

import base64
import json
import time


def bearer(tok: str) -> dict:
    return {"Authorization": f"Bearer {tok}"}


def claims_of(tok: str) -> dict:
    payload = tok.split(".")[1]
    payload += "=" * (-len(payload) % 4)
    return json.loads(base64.urlsafe_b64decode(payload))


def test_no_token_401(client):
    assert client.get("/items").status_code == 401


def test_valid_token_200(client, token):
    assert client.get("/items", headers=bearer(token())).status_code == 200


def test_me_returns_identity(client, token):
    tok = token()
    resp = client.get("/me", headers=bearer(tok))
    assert resp.status_code == 200
    body = resp.json()
    assert body["client_id"] == "eval-client"
    assert body["sub"] == claims_of(tok)["sub"]


def test_tampered_401(client, token):
    header, payload, sig = token().split(".")
    # Flip a character in the middle of the signature: the last base64url
    # character may only carry padding bits, so changing it can be a no-op.
    i = len(sig) // 2
    flipped = "A" if sig[i] != "A" else "B"
    tampered = f"{header}.{payload}.{sig[:i]}{flipped}{sig[i + 1:]}"
    assert client.get("/items", headers=bearer(tampered)).status_code == 401


def test_expired_401(client, token):
    tok = token(client="eval-expiring")
    time.sleep(4)
    assert client.get("/items", headers=bearer(tok)).status_code == 401


def test_wrong_audience_401(client, token):
    tok = token(resource="urn:codecite-eval:other")
    assert client.get("/items", headers=bearer(tok)).status_code == 401


def test_health_open(client):
    assert client.get("/health").status_code == 200
