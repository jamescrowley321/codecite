"""Logout tokens the provider sends to /backchannel-logout."""

import os

import httpx
import jwt

DISCOVERY_URL = os.environ["OIDC_DISCOVERY_URL"]
CLIENT_ID = os.environ["OIDC_CLIENT_ID"]
BACKCHANNEL_LOGOUT_EVENT = "http://schemas.openid.net/event/backchannel-logout"


class InvalidLogoutToken(Exception):
    pass


def _discovery() -> dict:
    return httpx.get(DISCOVERY_URL, timeout=10).raise_for_status().json()


def _signing_keys(jwks_uri: str) -> jwt.PyJWKSet:
    jwks = httpx.get(jwks_uri, timeout=10).raise_for_status().json()
    return jwt.PyJWKSet.from_dict(jwks)


def decode_logout_token(token: str) -> dict:
    """The token's claims, once it passes Back-Channel Logout 1.0 §2.6."""
    try:
        disco = _discovery()
        kid = jwt.get_unverified_header(token).get("kid")
        key = next((k for k in _signing_keys(disco["jwks_uri"]).keys if k.key_id == kid), None)
        if key is None:
            raise InvalidLogoutToken("unknown signing key")
        claims = jwt.decode(
            token,
            key,
            algorithms=["RS256"],
            audience=CLIENT_ID,
            issuer=disco["issuer"],
            options={"require": ["iss", "aud", "iat", "exp", "jti"]},
        )
    except (jwt.InvalidTokenError, httpx.HTTPError, KeyError) as e:
        raise InvalidLogoutToken(str(e)) from e
    if "sub" not in claims and "sid" not in claims:
        raise InvalidLogoutToken("neither sub nor sid")
    events = claims.get("events")
    if not isinstance(events, dict) or not isinstance(events.get(BACKCHANNEL_LOGOUT_EVENT), dict):
        raise InvalidLogoutToken("events lacks the back-channel logout member")
    if "nonce" in claims:
        raise InvalidLogoutToken("nonce present")
    return claims
