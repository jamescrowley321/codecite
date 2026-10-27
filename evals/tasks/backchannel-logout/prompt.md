# Fix the logout endpoint

Our logout endpoint accepts logout tokens it should reject. Make it conform to
the spec the endpoint implements. Don't add packages.

The identity-model source is available at `{identity_model}`.

Keep `app.main:app` as the application and `app/sessions.py` as it is. Stop
when you're done.
