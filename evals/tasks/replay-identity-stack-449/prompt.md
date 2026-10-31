The repository at `/work/project` is jamescrowley321/identity-stack. Make the change this issue
asks for. Stop when you're done.

# security: /api/fga/* resolves users project-wide, leaking cross-tenant existence and internal ids

Found in a fresh-context review of #425 (auth/authz lens). Pre-existing.

`backend/app/dependencies/fga.py:40-75` calls `load_user`, and `backend/app/services/descope.py:135-160` resolves by `loginIds` **project-wide with `withTestUser: true`** — no tenant filter.

`documents.py:363-370` and `:449-456` refuse targets outside the caller's `dct`. `routers/fga.py:148`, `:188`, `:269` do not.

Scenario — owner/admin of tenant A:

```
POST /api/fga/relations
{"resource_type":"document","resource_id":"<own doc>","relation":"viewer","target":"victim@other-tenant.example"}
```

→ `200` echoing `"target": "U3J..."`, the victim's Descope userId. A miss echoes the input unchanged, so hit/miss is trivially distinguishable: an existence oracle over every account in the project, plus disclosure of the internal id. It also writes a real grant keyed on a foreign-tenant subject.

Fix: apply the same `userTenants` containment check that share/revoke already use, on every `/api/fga/*` route that resolves a target.
