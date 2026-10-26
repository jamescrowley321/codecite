# Notes

A small notes app. Users sign in at the identity provider with OpenID
Connect; the app keeps one session per sign-in.

    uv sync
    OIDC_DISCOVERY_URL=... OIDC_CLIENT_ID=... uv run uvicorn app.main:app --reload

Settings, read from the environment:

- `OIDC_DISCOVERY_URL`: the provider's discovery document
- `OIDC_CLIENT_ID`: this app's client id at the provider

Routes:

- `GET /health`: liveness check
- `GET /notes`: the notes
- `POST /backchannel-logout`: the provider calls this when a user signs out
  elsewhere, with a `logout_token` form field

Sessions live in memory (`app/sessions.py`).
