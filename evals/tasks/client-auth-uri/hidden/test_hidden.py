"""Hidden tests for the client-auth-uri task. The agent never sees these.

RFC 6749 §2.3.1: servers MUST support HTTP Basic, MAY accept credentials in
the request body, and the credentials MUST NOT be in the request URI. §5.2:
failed client authentication is `invalid_client`, with a 401 and a
WWW-Authenticate header when the client used the Authorization header.
"""

import base64
from urllib.parse import quote_plus

CLIENT_ID = "reports-service"
SECRET = "reports-secret-4f9c"
GRANT = {"grant_type": "client_credentials"}


def basic(client_id: str, secret: str) -> dict:
    raw = f"{quote_plus(client_id)}:{quote_plus(secret)}".encode()
    return {"Authorization": "Basic " + base64.b64encode(raw).decode()}


def assert_token(resp):
    assert resp.status_code == 200, resp.text
    body = resp.json()
    assert body.get("access_token")
    assert str(body.get("token_type", "")).lower() == "bearer"


def assert_invalid_client(resp):
    assert resp.status_code in (400, 401), resp.text
    body = resp.json()
    assert body.get("error") == "invalid_client"
    assert "access_token" not in body


def test_basic_gets_token(client):
    assert_token(client.post("/token", data=GRANT, headers=basic(CLIENT_ID, SECRET)))


def test_form_body_gets_token(client):
    assert_token(client.post("/token", data={**GRANT, "client_id": CLIENT_ID, "client_secret": SECRET}))


def test_query_credentials_rejected(client):
    resp = client.post("/token", params={"client_id": CLIENT_ID, "client_secret": SECRET}, data=GRANT)
    assert_invalid_client(resp)


def test_query_secret_rejected(client):
    resp = client.post("/token", params={"client_secret": SECRET}, data={**GRANT, "client_id": CLIENT_ID})
    assert_invalid_client(resp)


def test_wrong_basic_secret_rejected(client):
    resp = client.post("/token", data=GRANT, headers=basic(CLIENT_ID, "wrong"))
    assert_invalid_client(resp)
    assert resp.status_code == 401
    assert resp.headers.get("www-authenticate", "").lower().startswith("basic")


def test_wrong_form_secret_rejected(client):
    assert_invalid_client(client.post("/token", data={**GRANT, "client_id": CLIENT_ID, "client_secret": "wrong"}))


def test_unknown_client_rejected(client):
    assert_invalid_client(client.post("/token", data=GRANT, headers=basic("nobody", SECRET)))
