# Fix client authentication at the token endpoint

This project is a small FastAPI service (`app/main.py`) with an OAuth 2.0
token endpoint for the client credentials grant.

A security review flagged how our token endpoint accepts client credentials.
Fix what the OAuth spec forbids, keep what it allows, and add a comment above
the fix that cites the section.

Don't use the network beyond what's already installed: no new packages. Keep
`app.main:app` as the application. Stop when you're done.
