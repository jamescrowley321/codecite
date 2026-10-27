# Token Service

A small OAuth 2.0 token service for our internal services. It issues access
tokens with the client credentials grant.

    uv sync
    uv run uvicorn app.main:app --reload

Routes:

- `GET /health`: liveness check
- `POST /token`: the token endpoint (`grant_type=client_credentials`)

Clients authenticate with their client id and secret.
