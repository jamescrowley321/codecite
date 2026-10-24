# Add token authentication to the Items API

This project is a small FastAPI service (`app/main.py`). Add bearer-token
authentication to it using identity-model's FastAPI integration, the
`fastapi-identity-model` package, which is already a dependency.

The identity-model source is available at `{identity_model}`.

Requirements:

1. Protect every route with identity-model's `TokenValidationMiddleware`.
2. Leave `/health` open: it must answer without a token.
3. Make `/me` return `{"sub": ..., "client_id": ...}` for the current user,
   using identity-model's `CurrentUser` dependency.
4. Read the discovery URL and the expected audience from the environment
   variables `OIDC_DISCOVERY_URL` and `OIDC_AUDIENCE`. Both are set before
   `app.main` is imported.
5. Don't use the network beyond what's already installed: no new packages.

Keep `app.main:app` as the application. Stop when you're done.
