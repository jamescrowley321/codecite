"""Logout tokens the provider sends to /backchannel-logout."""

import os

import httpx
import jwt

DISCOVERY_URL = os.environ["OIDC_DISCOVERY_URL"]
CLIENT_ID = os.environ["OIDC_CLIENT_ID"]


class InvalidLogoutToken(Exception):
    pass


def _signing_keys() -> jwt.PyJWKSet:
    disco = httpx.get(DISCOVERY_URL, timeout=10).raise_for_status().json()
    jwks = httpx.get(disco["jwks_uri"], timeout=10).raise_for_status().json()
    return jwt.PyJWKSet.from_dict(jwks)


def decode_logout_token(token: str) -> dict:
    """The token's claims, once its signature and audience check out."""
    try:
        kid = jwt.get_unverified_header(token).get("kid")
        key = next((k for k in _signing_keys().keys if k.key_id == kid), None)
        if key is None:
            raise InvalidLogoutToken("unknown signing key")
        return jwt.decode(token, key, algorithms=["RS256"], audience=CLIENT_ID)
    except (jwt.InvalidTokenError, httpx.HTTPError) as e:
        raise InvalidLogoutToken(str(e)) from e
