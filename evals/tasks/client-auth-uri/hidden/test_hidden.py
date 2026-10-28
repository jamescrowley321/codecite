"""Hidden tests for the client-auth-uri task. The agent never sees these.

RFC 6749 §2.3.1: servers MUST support HTTP Basic, MAY accept credentials in
the request body, and the credentials MUST NOT be in the request URI. §5.2:
failed client authentication is `invalid_client`, with a 401 and a
WWW-Authenticate header when the client used the Authorization header.
§3.2: request parameters MUST NOT be included more than once, and §5.2
names a request that "repeats a parameter, includes multiple credentials"
an invalid_request; a body that repeats client_id is refused.
§2.3: a client MUST NOT use more than one authentication method in a
request, and §5.2 names that an invalid_request; HTTP Basic plus a
client_secret in the body is refused either way too.
Credentials in the URI may be refused either way: as failed authentication
(`invalid_client`) or as a malformed request (`invalid_request`, §5.2's "or is
otherwise malformed"), so the query-string tests take both.
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


def assert_invalid_client(resp, allowed=("invalid_client",)):
    assert resp.status_code in (400, 401), resp.text
    body = resp.json()
    assert body.get("error") in allowed
    assert "access_token" not in body


def assert_uri_refused(resp):
    assert_invalid_client(resp, allowed=("invalid_client", "invalid_request"))


def test_basic_gets_token(client):
    assert_token(client.post("/token", data=GRANT, headers=basic(CLIENT_ID, SECRET)))


def test_form_body_gets_token(client):
    assert_token(client.post("/token", data={**GRANT, "client_id": CLIENT_ID, "client_secret": SECRET}))


def test_query_credentials_rejected(client):
    resp = client.post("/token", params={"client_id": CLIENT_ID, "client_secret": SECRET}, data=GRANT)
    assert_uri_refused(resp)


def test_query_secret_rejected(client):
    resp = client.post("/token", params={"client_secret": SECRET}, data={**GRANT, "client_id": CLIENT_ID})
    assert_uri_refused(resp)


def test_wrong_basic_secret_rejected(client):
    resp = client.post("/token", data=GRANT, headers=basic(CLIENT_ID, "wrong"))
    assert_invalid_client(resp)
    assert resp.status_code == 401
    assert resp.headers.get("www-authenticate", "").lower().startswith("basic")


def test_wrong_form_secret_rejected(client):
    assert_invalid_client(client.post("/token", data={**GRANT, "client_id": CLIENT_ID, "client_secret": "wrong"}))


def test_unknown_client_rejected(client):
    assert_invalid_client(client.post("/token", data=GRANT, headers=basic("nobody", SECRET)))


def test_basic_and_form_secret_rejected(client):
    data = {**GRANT, "client_id": CLIENT_ID, "client_secret": SECRET}
    assert_uri_refused(client.post("/token", data=data, headers=basic(CLIENT_ID, SECRET)))


def test_repeated_client_id_rejected(client):
    body = f"grant_type=client_credentials&client_id=billing-service&client_id={CLIENT_ID}&client_secret={SECRET}"
    resp = client.post("/token", content=body, headers={"Content-Type": "application/x-www-form-urlencoded"})
    assert_uri_refused(resp)
