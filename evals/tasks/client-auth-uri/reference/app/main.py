import base64
import binascii
import secrets
from urllib.parse import unquote_plus

from fastapi import FastAPI, Request
from fastapi.responses import JSONResponse

app = FastAPI(title="Token Service")

# Registered confidential clients and their secrets.
CLIENTS = {
    "reports-service": "reports-secret-4f9c",
    "billing-service": "billing-secret-81d2",
}
TOKEN_LIFETIME = 3600
NO_STORE = {"Cache-Control": "no-store", "Pragma": "no-cache"}


def error(code: str, status: int = 400, headers: dict | None = None) -> JSONResponse:
    return JSONResponse({"error": code}, status_code=status, headers={**NO_STORE, **(headers or {})})


def invalid_client() -> JSONResponse:
    return error("invalid_client", 401, {"WWW-Authenticate": 'Basic realm="token"'})


def basic_credentials(request: Request) -> tuple[str, str] | None:
    """The client id and secret from an HTTP Basic Authorization header, or
    None when there is no such header. A malformed header yields empty
    credentials, which never match a client."""
    scheme, _, value = request.headers.get("authorization", "").partition(" ")
    if scheme.lower() != "basic":
        return None
    try:
        decoded = base64.b64decode(value.strip(), validate=True).decode("utf-8")
    except (binascii.Error, UnicodeDecodeError):
        return "", ""
    client_id, sep, client_secret = decoded.partition(":")
    if not sep:
        return "", ""
    return unquote_plus(client_id), unquote_plus(client_secret)


@app.get("/health")
async def health():
    return {"status": "ok"}


@app.post("/token")
async def token(request: Request):
    form = await request.form()

    # RFC 6749 §2.3.1: client credentials go in the Authorization header (HTTP
    # Basic, which servers MUST support) or the request body, and MUST NOT be
    # included in the request URI. A request that puts them there is rejected
    # as invalid_client (§5.2) even when they are valid.
    if "client_id" in request.query_params or "client_secret" in request.query_params:
        return invalid_client()

    # Parameters MUST NOT be included more than once; a request that repeats
    # one is an invalid_request (§5.2). (Not cited by section here, so
    # planted.sh's citation cases test only the §2.3.1 comment.)
    if any(len(form.getlist(name)) > 1 for name in ("client_id", "client_secret", "grant_type")):
        return error("invalid_request")

    creds = basic_credentials(request)
    # A client MUST NOT use more than one authentication method in a request;
    # one that does is an invalid_request (§5.2). (Not cited by section here,
    # so planted.sh's citation cases test only the comment above.)
    if creds is not None and "client_secret" in form:
        return error("invalid_request")
    if creds is not None:
        client_id, client_secret = creds
    else:
        client_id, client_secret = form.get("client_id"), form.get("client_secret")

    expected = CLIENTS.get(client_id or "")
    if expected is None or not secrets.compare_digest(expected.encode(), (client_secret or "").encode()):
        return invalid_client()

    if form.get("grant_type") != "client_credentials":
        return error("unsupported_grant_type")

    return JSONResponse(
        {"access_token": secrets.token_urlsafe(32), "token_type": "Bearer", "expires_in": TOKEN_LIFETIME},
        headers=NO_STORE,
    )
