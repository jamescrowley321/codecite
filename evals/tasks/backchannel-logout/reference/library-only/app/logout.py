"""Logout tokens the provider sends to /backchannel-logout."""

import os

from py_identity_model import PyIdentityModelException, TokenValidationConfig
from py_identity_model.aio import validate_logout_token

DISCOVERY_URL = os.environ["OIDC_DISCOVERY_URL"]
CLIENT_ID = os.environ["OIDC_CLIENT_ID"]


class InvalidLogoutToken(Exception):
    pass


async def decode_logout_token(token: str) -> dict:
    """The token's claims, once it passes Back-Channel Logout 1.0 §2.6."""
    config = TokenValidationConfig(perform_disco=True, audience=CLIENT_ID, algorithms=["RS256"])
    try:
        return await validate_logout_token(token, config, disco_doc_address=DISCOVERY_URL)
    except PyIdentityModelException as e:
        raise InvalidLogoutToken(str(e)) from e
