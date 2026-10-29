"""Logout tokens the provider sends to /backchannel-logout."""

import os

from py_identity_model import PyIdentityModelException, TokenValidationConfig
from py_identity_model.aio import validate_logout_token

DISCOVERY_URL = os.environ["OIDC_DISCOVERY_URL"]
CLIENT_ID = os.environ["OIDC_CLIENT_ID"]


class InvalidLogoutToken(Exception):
    pass


BACKCHANNEL_LOGOUT_EVENT = "http://schemas.openid.net/event/backchannel-logout"


async def decode_logout_token(token: str) -> dict:
    """The token's claims, once it passes Back-Channel Logout 1.0 §2.6.

    identity-model checks `exp` only when present and the `events` member's
    presence only; §2.4 (errata set 1) makes `exp` REQUIRED and the member's
    value a JSON object."""
    config = TokenValidationConfig(perform_disco=True, audience=CLIENT_ID, algorithms=["RS256"])
    try:
        claims = await validate_logout_token(token, config, disco_doc_address=DISCOVERY_URL)
    except PyIdentityModelException as e:
        raise InvalidLogoutToken(str(e)) from e
    if "exp" not in claims:
        raise InvalidLogoutToken("exp missing")
    if not isinstance(claims["events"].get(BACKCHANNEL_LOGOUT_EVENT), dict):
        raise InvalidLogoutToken("the back-channel logout event is not a JSON object")
    return claims
